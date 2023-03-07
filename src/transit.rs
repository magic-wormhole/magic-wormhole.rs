//! Connect two sides via TCP, no matter where they are
//!
//! This protocol is the second part where the Wormhole magic happens. It does not strictly require a Wormhole connection,
//! but it depends on some kind of secure communication channel to talk to the other side. Conveniently, Wormhole provides
//! exactly such a thing :)
//!
//! Both clients exchange messages containing hints on how to find each other. These may be local IP addresses for in case they
//! are in the same network, or the URL to a relay server. In case a direct connection fails, both will connect to the relay server
//! which will transparently glue the connections together.
//!
//! Each side might implement (or use/enable) some [abilities](Abilities).
//!
//! **Notice:** while the resulting TCP connection is naturally bi-directional, the handshake is not symmetric. There *must* be one
//! "leader" side and one "follower" side (formerly called "sender" and "receiver").

use crate::{Key, KeyPurpose};
use serde_derive::{Deserialize, Serialize};

use async_std::{
    io::{prelude::WriteExt, ReadExt},
    net::{TcpListener, TcpStream},
};
use futures::io::{AsyncRead, AsyncWrite};
#[allow(unused_imports)] /* We need them for the docs */
use futures::{future::TryFutureExt, Sink, SinkExt, Stream, StreamExt, TryStreamExt};
use log::*;
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::Arc,
};
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::aead::{Aead, NewAead};

mod crypto;

/// ULR to a default hosted relay server. Please don't abuse or DOS.
pub const DEFAULT_RELAY_SERVER: &str = "tcp://transit.magic-wormhole.io:4001";
// No need to make public, it's hard-coded anyways (:
// Open an issue if you want an API for this
// Use <stun.stunprotocol.org:3478> for non-production testing
const PUBLIC_STUN_SERVER: &str = "stun.piegames.de:3478";

#[derive(Debug)]
pub struct TransitKey;
impl KeyPurpose for TransitKey {}
#[derive(Debug)]
pub struct TransitRxKey;
impl KeyPurpose for TransitRxKey {}
#[derive(Debug)]
pub struct TransitTxKey;
impl KeyPurpose for TransitTxKey {}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransitConnectError {
    /** Incompatible abilities, or wrong hints */
    #[error("{}", _0)]
    Protocol(Box<str>),
    #[error("All (relay) handshakes failed or timed out; could not establish a connection with the peer")]
    Handshake,
    #[error("IO error")]
    IO(
        #[from]
        #[source]
        std::io::Error,
    ),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransitError {
    #[error("Cryptography error. This is probably an implementation bug, but may also be caused by an attack.")]
    Crypto,
    #[error("Wrong nonce received, got {:x?} but expected {:x?}. This is probably an implementation bug, but may also be caused by an attack.", _0, _1)]
    Nonce(Box<[u8]>, Box<[u8]>),
    #[error("IO error")]
    IO(
        #[from]
        #[source]
        std::io::Error,
    ),
}

impl From<()> for TransitError {
    fn from(_: ()) -> Self {
        Self::Crypto
    }
}

/**
 * Defines a way to find the other side.
 *
 * Each ability comes with a set of [`Hints`] to encode how to meet up.
 */
#[derive(Copy, Clone, Debug, Default)]
pub struct Abilities {
    /** Direct connection to the peer */
    pub direct_tcp_v1: bool,
    /** Connection over a relay */
    pub relay_v1: bool,
    #[cfg(all())]
    /** **Experimental** Use the [noise protocol](https://noiseprotocol.org) for the encryption. */
    pub noise_v1: bool,
}

impl Abilities {
    pub const ALL_ABILITIES: Self = Self {
        direct_tcp_v1: true,
        relay_v1: true,
        #[cfg(all())]
        noise_v1: false,
    };

    /**
     * If you absolutely don't want to use any relay servers.
     *
     * If the other side forces relay usage or doesn't support any of your connection modes
     * the attempt will fail.
     */
    pub const FORCE_DIRECT: Self = Self {
        direct_tcp_v1: true,
        relay_v1: false,
        #[cfg(all())]
        noise_v1: false,
    };

    /**
     * If you don't want to disclose your IP address to your peer
     *
     * If the other side forces a the usage of a direct connection the attempt will fail.
     * Note that the other side might control the relay server being used, if you really
     * don't want your IP to potentially be disclosed use TOR instead (not supported by
     * the Rust implementation yet).
     */
    pub const FORCE_RELAY: Self = Self {
        direct_tcp_v1: false,
        relay_v1: true,
        #[cfg(all())]
        noise_v1: false,
    };

    pub fn can_direct(&self) -> bool {
        self.direct_tcp_v1
    }

    pub fn can_relay(&self) -> bool {
        self.relay_v1
    }

    #[cfg(all())]
    pub fn can_noise_crypto(&self) -> bool {
        self.noise_v1
    }

    /** Keep only abilities that both sides support */
    pub fn intersect(mut self, other: &Self) -> Self {
        self.direct_tcp_v1 &= other.direct_tcp_v1;
        self.relay_v1 &= other.relay_v1;
        #[cfg(all())]
        {
            self.noise_v1 &= other.noise_v1;
        }
        self
    }
}

impl serde::Serialize for Abilities {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut hints = Vec::new();
        if self.direct_tcp_v1 {
            hints.push(serde_json::json!({
                "type": "direct-tcp-v1",
            }));
        }
        if self.relay_v1 {
            hints.push(serde_json::json!({
                "type": "relay-v1",
            }));
        }
        #[cfg(all())]
        if self.noise_v1 {
            hints.push(serde_json::json!({
                "type": "noise-crypto-v1",
            }));
        }
        serde_json::Value::Array(hints).serialize(ser)
    }
}

impl<'de> serde::Deserialize<'de> for Abilities {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "kebab-case", tag = "type")]
        enum Ability {
            DirectTcpV1,
            RelayV1,
            RelayV2,
            #[cfg(all())]
            NoiseCryptoV1,
            #[serde(other)]
            Other,
        }

        let mut abilities = Self::default();
        /* Specifying a hint multiple times is undefined behavior. Here, we simply merge all features. */
        for ability in <Vec<Ability> as serde::Deserialize>::deserialize(de)? {
            match ability {
                Ability::DirectTcpV1 => {
                    abilities.direct_tcp_v1 = true;
                },
                Ability::RelayV1 => {
                    abilities.relay_v1 = true;
                },
                #[cfg(all())]
                Ability::NoiseCryptoV1 => {
                    abilities.noise_v1 = true;
                },
                _ => (),
            }
        }
        Ok(abilities)
    }
}

/* Wire representation of a single hint */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
enum HintSerde {
    DirectTcpV1(DirectHint),
    RelayV1(RelayHint),
    #[serde(other)]
    Unknown,
}

/** Information about how to find a peer */
#[derive(Clone, Debug, Default)]
pub struct Hints {
    /** Hints for direct connection */
    pub direct_tcp: HashSet<DirectHint>,
    /** List of relay servers */
    pub relay: Vec<RelayHint>,
}

impl Hints {
    pub fn new(
        direct_tcp: impl IntoIterator<Item = DirectHint>,
        relay: impl IntoIterator<Item = RelayHint>,
    ) -> Self {
        Self {
            direct_tcp: direct_tcp.into_iter().collect(),
            relay: relay.into_iter().collect(),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Hints {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let hints: Vec<HintSerde> = serde::Deserialize::deserialize(de)?;
        let mut direct_tcp = HashSet::new();
        let mut relay = Vec::<RelayHint>::new();
        let mut relay_v2 = Vec::<RelayHint>::new();

        for hint in hints {
            match hint {
                HintSerde::DirectTcpV1(hint) => {
                    direct_tcp.insert(hint);
                },
                HintSerde::RelayV1(hint) => {
                    relay_v2.push(hint);
                },
                /* Ignore unknown hints */
                _ => {},
            }
        }

        /* If there are any relay-v2 hints, there relay-v1 are redundant */
        if !relay_v2.is_empty() {
            relay.clear();
        }
        relay.extend(relay_v2.into_iter().map(Into::into));

        Ok(Hints { direct_tcp, relay })
    }
}

impl serde::Serialize for Hints {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let direct = self.direct_tcp.iter().cloned().map(HintSerde::DirectTcpV1);
        let relay = self.relay.iter().cloned().map(HintSerde::RelayV1);
        ser.collect_seq(direct.chain(relay))
    }
}

/** hostname and port for direct connection */
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash, derive_more::Display)]
#[display(fmt = "tcp://{}:{}", hostname, port)]
pub struct DirectHint {
    // DirectHint also contains a `priority` field, but it is underspecified
    // and we won't use it
    // pub priority: f32,
    pub hostname: String,
    pub port: u16,
}

impl DirectHint {
    pub fn new(hostname: impl Into<String>, port: u16) -> Self {
        Self {
            hostname: hostname.into(),
            port,
        }
    }
}

/* Wire representation of a single relay hint (Helper struct for serialization) */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
struct RelayHintSerde {
    name: Option<String>,
    #[serde(rename = "hints")]
    endpoints: Vec<RelayHintSerdeInner>,
}

/* Wire representation of a single relay endpoint (Helper struct for serialization) */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
enum RelayHintSerdeInner {
    #[serde(rename = "direct-tcp-v1")]
    Tcp(DirectHint),
    Websocket {
        url: url::Url,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelayHintParseError {
    #[error(
        "Invalid TCP hint endpoint: '{}' (Does it have hostname and port?)",
        _0
    )]
    InvalidTcp(url::Url),
    #[error(
        "Unknown schema: '{}'. Currently known values are 'tcp', 'ws'  and 'wss'.",
        _0
    )]
    UnknownSchema(Box<str>),
    #[error("'{}' is not an absolute URL (must start with a '/')", _0)]
    UrlNotAbsolute(url::Url),
}

/**
 * Hint describing a relay server
 *
 * A server may be reachable at multiple locations. Any two must be relayable
 * over that server, therefore a client may pick only one of these per hint.
 *
 * All locations are URLs, but here they are already deconstructed and grouped
 * by schema out of convenience.
 */
/* RelayHint::default() gives the empty server (cannot be reached), and is only there for struct update syntax */
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RelayHint {
    /** Human readable name. The expectation is that when a server has multiple endpoints, the
     * expectation is that the domain name is used as name
     */
    pub name: Option<String>,
    /** TCP endpoints of that relay */
    pub tcp: HashSet<DirectHint>,
    /** WebSockets endpoints of that relay */
    pub ws: HashSet<url::Url>,
}

impl RelayHint {
    pub fn new(
        name: Option<String>,
        tcp: impl IntoIterator<Item = DirectHint>,
        ws: impl IntoIterator<Item = url::Url>,
    ) -> Self {
        Self {
            name,
            tcp: tcp.into_iter().collect(),
            ws: ws.into_iter().collect(),
        }
    }

    /// Construct a relay hint from a list of multiple endpoints, and optionally a name.
    ///
    /// Not all URLs are acceptable, therefore this method is fallible. Especially, TCP endpoints
    /// must be encoded as `tcp://hostname:port`. All URLs must be absolute, i.e. start with a `/`.
    ///
    /// Basic usage (default server):
    ///
    /// ```
    /// use magic_wormhole::transit;
    /// let hint =
    ///     transit::RelayHint::from_urls(None, [transit::DEFAULT_RELAY_SERVER.parse().unwrap()])
    ///         .unwrap();
    /// ```
    ///
    /// Custom relay server from url with name:
    ///
    /// ```
    /// use magic_wormhole::transit;
    /// # let url: url::Url = transit::DEFAULT_RELAY_SERVER.parse().unwrap();
    /// let hint = transit::RelayHint::from_urls(url.host_str().map(str::to_owned), [url]).unwrap();
    /// ```
    pub fn from_urls(
        name: Option<String>,
        urls: impl IntoIterator<Item = url::Url>,
    ) -> Result<Self, RelayHintParseError> {
        let mut this = Self {
            name,
            ..Self::default()
        };
        for url in urls.into_iter() {
            ensure!(
                !url.cannot_be_a_base(),
                RelayHintParseError::UrlNotAbsolute(url)
            );
            match url.scheme() {
                "tcp" => {
                    /* Using match */
                    let (hostname, port) = match (url.host_str(), url.port()) {
                        (Some(hostname), Some(port)) => (hostname.into(), port),
                        _ => bail!(RelayHintParseError::InvalidTcp(url)),
                    };
                    this.tcp.insert(DirectHint { hostname, port });
                },
                "ws" | "wss" => {
                    this.ws.insert(url);
                },
                other => bail!(RelayHintParseError::UnknownSchema(other.into())),
            }
        }
        assert!(
            !this.tcp.is_empty() || !this.ws.is_empty(),
            "No URLs provided"
        );
        Ok(this)
    }

    pub fn can_merge(&self, other: &Self) -> bool {
        !self.tcp.is_disjoint(&other.tcp) || !self.ws.is_disjoint(&other.ws)
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.merge_mut(other);
        self
    }

    pub fn merge_mut(&mut self, other: Self) {
        self.tcp.extend(other.tcp);
        self.ws.extend(other.ws);
    }

    pub fn merge_into(self, collection: &mut Vec<RelayHint>) {
        for item in collection.iter_mut() {
            if item.can_merge(&self) {
                item.merge_mut(self);
                return;
            }
        }
        collection.push(self);
    }
}

impl serde::Serialize for RelayHint {
    fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut hints = Vec::new();
        hints.extend(self.tcp.iter().cloned().map(RelayHintSerdeInner::Tcp));
        hints.extend(
            self.ws
                .iter()
                .cloned()
                .map(|h| RelayHintSerdeInner::Websocket { url: h }),
        );

        serde_json::json!({
            "name": self.name,
            "hints": hints,
        })
        .serialize(ser)
    }
}

impl<'de> serde::Deserialize<'de> for RelayHint {
    fn deserialize<D>(de: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RelayHintSerde::deserialize(de)?;
        let mut hint = RelayHint {
            name: raw.name,
            tcp: HashSet::new(),
            ws: HashSet::new(),
        };

        for e in raw.endpoints {
            match e {
                RelayHintSerdeInner::Tcp(tcp) => {
                    hint.tcp.insert(tcp);
                },
                RelayHintSerdeInner::Websocket { url } => {
                    hint.ws.insert(url);
                },
                /* Ignore unknown hints */
                _ => {},
            }
        }

        Ok(hint)
    }
}

use std::convert::{TryFrom, TryInto};

impl TryFrom<&DirectHint> for IpAddr {
    type Error = std::net::AddrParseError;
    fn try_from(hint: &DirectHint) -> Result<IpAddr, std::net::AddrParseError> {
        hint.hostname.parse()
    }
}

impl TryFrom<&DirectHint> for SocketAddr {
    type Error = std::net::AddrParseError;
    /** This does not do the obvious thing and also implicitly maps all V4 addresses into V6 */
    fn try_from(hint: &DirectHint) -> Result<SocketAddr, std::net::AddrParseError> {
        let addr = hint.try_into()?;
        let addr = match addr {
            IpAddr::V4(v4) => IpAddr::V6(v4.to_ipv6_mapped()),
            IpAddr::V6(_) => addr,
        };
        Ok(SocketAddr::new(addr, hint.port))
    }
}

/// Direct or relay
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectionType {
    /// We are directly connected to our peer
    Direct,
    /// We are connected to a relay server, and may even know its name
    Relay { name: Option<String> },
}

/// Metadata for the established transit connection
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TransitInfo {
    /// Whether we are connected directly or via a relay server
    pub conn_type: ConnectionType,
    /// Target address of our connection. This may be our peer, or the relay server.
    /// This says nothing about the actual transport protocol used.
    pub peer_addr: SocketAddr,
    // Prevent exhaustive destructuring for future proofing
    _unused: (),
}

type TransitConnection = (Box<dyn TransitTransport>, TransitInfo);

fn set_socket_opts(socket: &socket2::Socket) -> std::io::Result<()> {
    socket.set_nonblocking(true)?;

    /* See https://stackoverflow.com/a/14388707/6094756.
     * On most BSD and Linux systems, we need both REUSEADDR and REUSEPORT;
     * and if they don't support the latter we won't compile.
     * On Windows, there is only REUSEADDR but it does what we want.
     */
    socket.set_reuse_address(true)?;
    #[cfg(all(unix, not(any(target_os = "solaris", target_os = "illumos"))))]
    {
        socket.set_reuse_port(true)?;
    }
    #[cfg(not(any(
        all(unix, not(any(target_os = "solaris", target_os = "illumos"))),
        target_os = "windows"
    )))]
    {
        compile_error!("Your system is not supported yet, please raise an error");
    }

    Ok(())
}

/**
 * Bind to a port with SO_REUSEADDR, connect to the destination and then hide the blood behind a pretty [`async_std::net::TcpStream`]
 *
 * We want an `async_std::net::TcpStream`, but with SO_REUSEADDR set.
 * The former is just a wrapper around `async_io::Async<std::net::TcpStream>`, of which we
 * copy the `connect` method to add a statement that will set the socket flag.
 * See https://github.com/smol-rs/async-net/issues/20.
 */
async fn connect_custom(
    local_addr: &socket2::SockAddr,
    dest_addr: &socket2::SockAddr,
) -> std::io::Result<async_std::net::TcpStream> {
    log::debug!("Binding to {}", local_addr.as_socket().unwrap());
    let socket = socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None)?;
    /* Set our custum options */
    set_socket_opts(&socket)?;

    socket.bind(local_addr)?;

    /* Initiate connect */
    match socket.connect(dest_addr) {
        Ok(_) => {},
        #[cfg(unix)]
        Err(err) if err.raw_os_error() == Some(libc::EINPROGRESS) => {},
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {},
        Err(err) => return Err(err),
    }

    let stream = async_io::Async::new(std::net::TcpStream::from(socket))?;
    /* The stream becomes writable when connected. */
    stream.writable().await?;

    /* Check if there was an error while connecting. */
    stream
        .get_ref()
        .take_error()
        .and_then(|maybe_err| maybe_err.map_or(Ok(()), Result::Err))?;
    /* Convert our mess to `async_std::net::TcpStream */
    Ok(stream.into_inner()?.into())
}

#[derive(Debug, thiserror::Error)]
enum StunError {
    #[error("No IPv4 addresses were found for the selected STUN server")]
    ServerIsV6Only,
    #[error("Server did not tell us our IP address")]
    ServerNoResponse,
    #[error("Connection timed out")]
    Timeout,
    #[error("IO error")]
    IO(
        #[from]
        #[source]
        std::io::Error,
    ),
    #[error("Malformed STUN packet")]
    Codec(
        #[from]
        #[source]
        bytecodec::Error,
    ),
}

/** Perform a STUN query to get the external IP address */
async fn get_external_ip() -> Result<(SocketAddr, TcpStream), StunError> {
    let mut socket = connect_custom(
        &"[::]:0".parse::<SocketAddr>().unwrap().into(),
        &PUBLIC_STUN_SERVER
            .to_socket_addrs()?
            /* If you find yourself behind a NAT66, open an issue */
            .find(|x| x.is_ipv4())
            /* TODO add a helper method to stdlib for this */
            .map(|addr| match addr {
                SocketAddr::V4(v4) => {
                    SocketAddr::new(IpAddr::V6(v4.ip().to_ipv6_mapped()), v4.port())
                },
                SocketAddr::V6(_) => unreachable!(),
            })
            .ok_or(StunError::ServerIsV6Only)?
            .into(),
    )
    .await?;

    use bytecodec::{DecodeExt, EncodeExt};
    use stun_codec::{
        rfc5389::{
            self,
            attributes::{MappedAddress, Software, XorMappedAddress},
            Attribute,
        },
        Message, MessageClass, MessageDecoder, MessageEncoder, TransactionId,
    };

    fn get_binding_request() -> Result<Vec<u8>, bytecodec::Error> {
        use rand::Rng;
        let random_bytes = rand::thread_rng().gen::<[u8; 12]>();

        let mut message = Message::new(
            MessageClass::Request,
            rfc5389::methods::BINDING,
            TransactionId::new(random_bytes),
        );

        message.add_attribute(Attribute::Software(Software::new(
            "magic-wormhole-rust".to_owned(),
        )?));

        // Encodes the message
        let mut encoder = MessageEncoder::new();
        let bytes = encoder.encode_into_bytes(message.clone())?;
        Ok(bytes)
    }

    fn decode_address(buf: &[u8]) -> Result<Option<SocketAddr>, bytecodec::Error> {
        let mut decoder = MessageDecoder::<Attribute>::new();
        let decoded = decoder.decode_from_bytes(buf)??;

        let external_addr1 = decoded
            .get_attribute::<XorMappedAddress>()
            .map(|x| x.address());
        //let external_addr2 = decoded.get_attribute::<XorMappedAddress2>().map(|x|x.address());
        let external_addr3 = decoded
            .get_attribute::<MappedAddress>()
            .map(|x| x.address());
        let external_addr = external_addr1
            // .or(external_addr2)
            .or(external_addr3);

        Ok(external_addr)
    }

    /* Connect the plugs */

    socket.write_all(get_binding_request()?.as_ref()).await?;

    let mut buf = [0u8; 256];
    /* Read header first */
    socket.read_exact(&mut buf[..20]).await?;
    let len: u16 = u16::from_be_bytes([buf[2], buf[3]]);
    /* Read the rest of the message */
    socket.read_exact(&mut buf[20..][..len as usize]).await?;
    let external_addr =
        decode_address(&buf[..20 + len as usize])?.ok_or(StunError::ServerNoResponse)?;

    Ok((external_addr, socket))
}

/// Utility method that logs information of the transit result
///
/// Example usage:
///
/// ```no_run
/// use magic_wormhole as mw;
/// # #[async_std::main] async fn main() -> Result<(), mw::transit::TransitConnectError> {
/// # let derived_key = unimplemented!();
/// # let their_abilities = unimplemented!();
/// # let their_hints = unimplemented!();
/// let connector: mw::transit::TransitConnector = unimplemented!("transit::init(…).await?");
/// let (mut transit, info) = connector
///     .leader_connect(derived_key, their_abilities, their_hints)
///     .await?;
/// mw::transit::log_transit_connection(info);
/// # Ok(())
/// # }
/// ```
pub fn log_transit_connection(info: TransitInfo) {
    match info.conn_type {
        ConnectionType::Direct => {
            log::info!(
                "Established direct transit connection to '{}'",
                info.peer_addr,
            );
        },
        ConnectionType::Relay { name: Some(name) } => {
            log::info!(
                "Established transit connection via relay '{}' ({})",
                name,
                info.peer_addr,
            );
        },
        ConnectionType::Relay { name: None } => {
            log::info!(
                "Established transit connection via relay ({})",
                info.peer_addr,
            );
        },
    }
}

/**
 * Initialize a relay handshake
 *
 * Bind a port and generate our [`Hints`]. This does not do any communication yet.
 */
pub async fn init(
    mut abilities: Abilities,
    peer_abilities: Option<Abilities>,
    relay_hints: Vec<RelayHint>,
) -> Result<TransitConnector, std::io::Error> {
    let mut our_hints = Hints::default();
    let mut listener = None;

    if let Some(peer_abilities) = peer_abilities {
        abilities = abilities.intersect(&peer_abilities);
    }

    /* Detect our IP addresses if the ability is enabled */
    if abilities.can_direct() {
        let create_sockets = async {
            /* Do a STUN query to get our public IP. If it works, we must reuse the same socket (port)
             * so that we will be NATted to the same port again. If it doesn't, simply bind a new socket
             * and use that instead.
             */
            let socket: MaybeConnectedSocket = match async_std::future::timeout(
                std::time::Duration::from_secs(4),
                get_external_ip(),
            )
            .await
            .map_err(|_| StunError::Timeout)
            {
                Ok(Ok((external_ip, stream))) => {
                    log::debug!("Our external IP address is {}", external_ip);
                    our_hints.direct_tcp.insert(DirectHint {
                        hostname: external_ip.ip().to_string(),
                        port: external_ip.port(),
                    });
                    log::debug!(
                        "Our socket for connecting is bound to {} and connected to {}",
                        stream.local_addr()?,
                        stream.peer_addr()?,
                    );
                    stream.into()
                },
                // TODO replace with .flatten() once stable
                // https://github.com/rust-lang/rust/issues/70142
                Err(err) | Ok(Err(err)) => {
                    log::warn!("Failed to get external address via STUN, {}", err);
                    let socket =
                        socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None)?;
                    set_socket_opts(&socket)?;

                    socket.bind(&"[::]:0".parse::<SocketAddr>().unwrap().into())?;
                    log::debug!(
                        "Our socket for connecting is bound to {}",
                        socket.local_addr()?.as_socket().unwrap(),
                    );

                    socket.into()
                },
            };

            /* Get a second socket, but this time open a listener on that port.
             * This sadly doubles the number of hints, but the method above doesn't work
             * for systems which don't have any firewalls. Also, this time we can't reuse
             * the port. In theory, we could, but it really confused the kernel to the point
             * of `accept` calls never returning again.
             */
            let socket2 = TcpListener::bind("[::]:0").await?;

            /* Find our ports, iterate all our local addresses, combine them with the ports and that's our hints */
            let port = socket.local_addr()?.as_socket().unwrap().port();
            let port2 = socket2.local_addr()?.port();
            our_hints.direct_tcp.extend(
                if_addrs::get_if_addrs()?
                    .iter()
                    .filter(|iface| !iface.is_loopback())
                    .flat_map(|ip| {
                        [
                            DirectHint {
                                hostname: ip.ip().to_string(),
                                port,
                            },
                            DirectHint {
                                hostname: ip.ip().to_string(),
                                port: port2,
                            },
                        ]
                        .into_iter()
                    }),
            );
            log::debug!("Our socket for listening is {}", socket2.local_addr()?);

            Ok::<_, std::io::Error>((socket, socket2))
        };

        listener = create_sockets
            .await
            // TODO replace with inspect_err once stable
            .map_err(|err| {
                log::error!("Failed to create direct hints for our side: {}", err);
                err
            })
            .ok();
    }

    if abilities.can_relay() {
        our_hints.relay.extend(relay_hints);
    }

    Ok(TransitConnector {
        sockets: listener,
        our_abilities: abilities,
        our_hints: Arc::new(our_hints),
    })
}

#[derive(derive_more::From)]
enum MaybeConnectedSocket {
    #[from]
    Socket(socket2::Socket),
    #[from]
    Stream(TcpStream),
}

impl MaybeConnectedSocket {
    fn local_addr(&self) -> std::io::Result<socket2::SockAddr> {
        match &self {
            Self::Socket(socket) => socket.local_addr(),
            Self::Stream(stream) => Ok(stream.local_addr()?.into()),
        }
    }
}

/**
 * A partially set up [`Transit`] connection.
 *
 * For the transit handshake, each side generates a [`Hints`] with all the information to find the other. You need
 * to exchange it (as in: send yours, receive theirs) with them. This is outside of the transit protocol, because we
 * are protocol agnostic.
 */
pub struct TransitConnector {
    /* Only `Some` if direct-tcp-v1 ability has been enabled.
     * The first socket is the port from which we will start connection attempts.
     * For in case the user is behind no firewalls, we must also listen to the second socket.
     */
    sockets: Option<(MaybeConnectedSocket, TcpListener)>,
    our_abilities: Abilities,
    our_hints: Arc<Hints>,
}

impl TransitConnector {
    pub fn our_abilities(&self) -> &Abilities {
        &self.our_abilities
    }

    /** Send this one to the other side */
    pub fn our_hints(&self) -> &Arc<Hints> {
        &self.our_hints
    }

    /**
     * Connect to the other side, as sender.
     */
    pub async fn leader_connect(
        self,
        transit_key: Key<TransitKey>,
        their_abilities: Abilities,
        their_hints: Arc<Hints>,
    ) -> Result<(Transit, TransitInfo), TransitConnectError> {
        let Self {
            sockets,
            our_abilities,
            our_hints,
        } = self;
        let transit_key = Arc::new(transit_key);

        let start = std::time::Instant::now();
        let mut connection_stream = Box::pin(
            Self::connect(
                true,
                transit_key,
                our_abilities,
                our_hints,
                their_abilities,
                their_hints,
                sockets,
            )
            .filter_map(|result| async {
                match result {
                    Ok(val) => Some(val),
                    Err(err) => {
                        log::debug!("Some leader handshake failed: {:?}", err);
                        None
                    },
                }
            }),
        );

        let (mut transit, mut finalizer, mut conn_info) = async_std::future::timeout(
            std::time::Duration::from_secs(60),
            connection_stream.next(),
        )
        .await
        .map_err(|_| {
            log::debug!("`leader_connect` timed out");
            TransitConnectError::Handshake
        })?
        .ok_or(TransitConnectError::Handshake)?;

        if conn_info.conn_type != ConnectionType::Direct && our_abilities.can_direct() {
            log::debug!(
                "Established transit connection over relay. Trying to find a direct connection …"
            );
            /* Measure the time it took us to get a response. Based on this, wait some more for more responses
             * in case we like one better.
             */
            let elapsed = start.elapsed();
            let to_wait = if elapsed.as_secs() > 5 {
                /* If our RTT was *that* long, let's just be happy we even got one connection */
                std::time::Duration::from_secs(1)
            } else {
                elapsed.mul_f32(0.3)
            };
            let _ = async_std::future::timeout(to_wait, async {
                while let Some((new_transit, new_finalizer, new_conn_info)) =
                    connection_stream.next().await
                {
                    /* We already got a connection, so we're only interested in direct ones */
                    if new_conn_info.conn_type == ConnectionType::Direct {
                        transit = new_transit;
                        finalizer = new_finalizer;
                        conn_info = new_conn_info;
                        log::debug!("Found direct connection; using that instead.");
                        break;
                    }
                }
            })
            .await;
            log::debug!("Did not manage to establish a better connection in time.");
        } else {
            log::debug!("Established direct transit connection");
        }

        /* Cancel all remaining non-finished handshakes. We could send "nevermind" to explicitly tell
         * the other side (probably, this is mostly for relay server statistics), but eeh, nevermind :)
         */
        std::mem::drop(connection_stream);

        let (tx, rx) = finalizer
            .handshake_finalize(&mut transit)
            .await
            .map_err(|e| {
                log::debug!("`handshake_finalize` failed: {e}");
                TransitConnectError::Handshake
            })?;

        // let socket = Box::new(socket) as Box<dyn TransitTransport>;
        Ok((
            Transit {
                socket: transit,
                tx,
                rx,
            },
            conn_info,
        ))
    }

    /**
     * Connect to the other side, as receiver
     */
    pub async fn follower_connect(
        self,
        transit_key: Key<TransitKey>,
        their_abilities: Abilities,
        their_hints: Arc<Hints>,
    ) -> Result<(Transit, TransitInfo), TransitConnectError> {
        let Self {
            sockets,
            our_abilities,
            our_hints,
        } = self;
        let transit_key = Arc::new(transit_key);

        let mut connection_stream = Box::pin(
            Self::connect(
                false,
                transit_key,
                our_abilities,
                our_hints,
                their_abilities,
                their_hints,
                sockets,
            )
            .filter_map(|result| async {
                match result {
                    Ok(val) => Some(val),
                    Err(err) => {
                        log::debug!("Some follower handshake failed: {:?}", err);
                        None
                    },
                }
            }),
        );

        let transit = match async_std::future::timeout(
            std::time::Duration::from_secs(60),
            &mut connection_stream.next(),
        )
        .await
        {
            Ok(Some((mut socket, finalizer, conn_info))) => {
                let (tx, rx) = finalizer
                    .handshake_finalize(&mut socket)
                    .await
                    .map_err(|e| {
                        log::debug!("`handshake_finalize` failed: {e}");
                        TransitConnectError::Handshake
                    })?;
                // let socket = Box::new(socket) as Box<dyn TransitTransport>;

                Ok((Transit { socket, tx, rx }, conn_info))
            },
            Ok(None) | Err(_) => {
                log::debug!("`follower_connect` timed out");
                Err(TransitConnectError::Handshake)
            },
        };

        /* Cancel all remaining non-finished handshakes. We could send "nevermind" to explicitly tell
         * the other side (probably, this is mostly for relay server statistics), but eeh, nevermind :)
         */
        std::mem::drop(connection_stream);

        transit
    }

    /** Try to establish a connection with the peer.
     *
     * This encapsulates code that is common to both the leader and the follower.
     *
     * ## Panics
     *
     * If the receiving end of the channel for the results is closed before all futures in the return
     * value are cancelled/dropped.
     */
    fn connect(
        is_leader: bool,
        transit_key: Arc<Key<TransitKey>>,
        our_abilities: Abilities,
        our_hints: Arc<Hints>,
        their_abilities: Abilities,
        their_hints: Arc<Hints>,
        socket: Option<(MaybeConnectedSocket, TcpListener)>,
    ) -> impl Stream<Item = Result<HandshakeResult, crypto::TransitHandshakeError>> + 'static {
        /* Take a tcp connection and transform it into a `TransitConnection` (mainly set timeouts) */
        fn wrap_tcp_connection(
            socket: TcpStream,
            conn_type: ConnectionType,
        ) -> Result<TransitConnection, crypto::TransitHandshakeError> {
            /* Set proper read and write timeouts. This will temporarily set the socket into blocking mode :/ */
            // https://github.com/async-rs/async-std/issues/499
            let socket = std::net::TcpStream::try_from(socket)
                .expect("Internal error: this should not fail because we never cloned the socket");
            socket.set_write_timeout(Some(std::time::Duration::from_secs(120)))?;
            socket.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
            let socket: TcpStream = socket.into();

            let info = TransitInfo {
                conn_type,
                peer_addr: socket
                    .peer_addr()
                    .expect("Internal error: socket must be IP"),
                _unused: (),
            };

            Ok((Box::new(socket), info))
        }

        /* Have socket => can direct */
        assert!(socket.is_none() || our_abilities.can_direct());

        let cryptor = if our_abilities.can_noise_crypto() && their_abilities.can_noise_crypto() {
            log::debug!("Using noise protocol for encryption");
            Arc::new(crypto::NoiseInit {
                key: transit_key.clone(),
            }) as Arc<dyn crypto::TransitCryptoInit>
        } else {
            log::debug!("Using secretbox for encryption");
            Arc::new(crypto::SecretboxInit {
                key: transit_key.clone(),
            }) as Arc<dyn crypto::TransitCryptoInit>
        };

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        let tside = Arc::new(hex::encode(rand::random::<[u8; 8]>()));

        /* Iterator of futures yielding a connection. They'll be then mapped with the handshake, collected into
         * a Vec and polled concurrently.
         */
        use futures::future::BoxFuture;
        type BoxIterator<T> = Box<dyn Iterator<Item = T>>;
        type ConnectorFuture =
            BoxFuture<'static, Result<TransitConnection, crypto::TransitHandshakeError>>;
        let mut connectors: BoxIterator<ConnectorFuture> = Box::new(std::iter::empty());

        /* Create direct connection sockets, if we support it. If peer doesn't support it, their list of hints will
         * be empty and no entries will be pushed.
         */
        let socket2 = if let Some((socket, socket2)) = socket {
            let local_addr = Arc::new(socket.local_addr().unwrap());
            /* Connect to each hint of the peer */
            connectors = Box::new(
                connectors.chain(
                    their_hints
                        .direct_tcp
                        .clone()
                        .into_iter()
                        /* Nobody should have that many IP addresses, even with NATing */
                        .take(50)
                        .map(move |hint| {
                            let local_addr = local_addr.clone();
                            async move {
                                let dest_addr = SocketAddr::try_from(&hint)?;
                                log::debug!("Connecting directly to {}", dest_addr);
                                let socket = connect_custom(&local_addr, &dest_addr.into()).await?;
                                log::debug!("Connected to {}!", dest_addr);

                                wrap_tcp_connection(socket, ConnectionType::Direct)
                            }
                        })
                        .map(|fut| Box::pin(fut) as ConnectorFuture),
                ),
            ) as BoxIterator<ConnectorFuture>;
            Some(socket2)
        } else if our_abilities.can_direct() {
            /* Fallback: We did not manage to bind a listener but we can still connect to the peer's hints */
            connectors = Box::new(
                connectors.chain(
                    their_hints
                        .direct_tcp
                        .clone()
                        .into_iter()
                        /* Nobody should have that many IP addresses, even with NATing */
                        .take(50)
                        .map(move |hint| async move {
                            let dest_addr = SocketAddr::try_from(&hint)?;
                            log::debug!("Connecting directly to {}", dest_addr);
                            let socket = async_std::net::TcpStream::connect(&dest_addr).await?;
                            log::debug!("Connected to {}!", dest_addr);

                            wrap_tcp_connection(socket, ConnectionType::Direct)
                        })
                        .map(|fut| Box::pin(fut) as ConnectorFuture),
                ),
            ) as BoxIterator<ConnectorFuture>;
            None
        } else {
            None
        };

        /* Relay hints. Make sure that both sides advertize it, since it is fine to support it without providing own hints. */
        if our_abilities.can_relay() && their_abilities.can_relay() {
            /* Collect intermediate into HashSet for deduplication */
            let mut relay_hints = Vec::<RelayHint>::new();
            relay_hints.extend(our_hints.relay.iter().take(2).cloned());
            for hint in their_hints.relay.iter().take(2).cloned() {
                hint.merge_into(&mut relay_hints);
            }

            connectors = Box::new(
                connectors.chain(
                    relay_hints
                        .into_iter()
                        /* A hint may have multiple addresses pointing towards the server. This may be multiple
                         * domain aliases or different ports or an IPv6 or IPv4 address. We only need
                         * to connect to one of them, since they are considered equivalent. However, we
                         * also want to be prepared for the rare case of one failing, thus we try to reach
                         * up to three different addresses. To not flood the system with requests, we
                         * start them in a 5 seconds interval spread. If one of them succeeds, the remaining ones
                         * will be cancelled anyways. Note that a hint might not necessarily be reachable via TCP.
                         */
                        .flat_map(|hint| {
                            /* If the hint has no name, take the first domain name as fallback */
                            let name = hint.name
                                .or_else(|| {
                                    /* Try to parse as IP address. We are only interested in human readable names (the IP address will be printed anyways) */
                                    hint.tcp.iter()
                                        .filter_map(|hint| match url::Host::parse(&hint.hostname) {
                                            Ok(url::Host::Domain(_)) => Some(hint.hostname.clone()),
                                            _ => None,
                                        })
                                        .next()
                                });
                            hint.tcp
                                .into_iter()
                                .take(3)
                                .enumerate()
                                .map(move |(i, h)| (i, h, name.clone()))
                        })
                        .map(|(index, host, name)| async move {
                            async_std::task::sleep(std::time::Duration::from_secs(
                                index as u64 * 5,
                            ))
                            .await;
                            log::debug!("Connecting to relay {}", host);
                            let socket = TcpStream::connect((host.hostname.as_str(), host.port))
                                .err_into::<crypto::TransitHandshakeError>()
                                .await?;
                            log::debug!("Connected to {}!", host);

                            wrap_tcp_connection(socket, ConnectionType::Relay { name })
                        })
                        .map(|fut| Box::pin(fut) as ConnectorFuture),
                ),
            ) as BoxIterator<ConnectorFuture>;
        }

        /* Do a handshake on all our found connections */
        let transit_key2 = transit_key.clone();
        let tside2 = tside.clone();
        let cryptor2 = cryptor.clone();
        let mut connectors = Box::new(
            connectors
                .map(move |fut| {
                    let transit_key = transit_key2.clone();
                    let tside = tside2.clone();
                    let cryptor = cryptor2.clone();
                    async move {
                        let (socket, conn_info) = fut.await?;
                        let (transit, finalizer) = handshake_exchange(
                            is_leader,
                            tside,
                            socket,
                            &conn_info.conn_type,
                            &*cryptor,
                            transit_key,
                        )
                        .await?;
                        Ok((transit, finalizer, conn_info))
                    }
                })
                .map(|fut| {
                    Box::pin(fut)
                        as BoxFuture<Result<HandshakeResult, crypto::TransitHandshakeError>>
                }),
        )
            as BoxIterator<BoxFuture<Result<HandshakeResult, crypto::TransitHandshakeError>>>;

        /* Also listen on some port just in case. */
        if let Some(socket2) = socket2 {
            connectors = Box::new(
                connectors.chain(
                    std::iter::once(async move {
                        let transit_key = transit_key.clone();
                        let tside = tside.clone();
                        let cryptor = cryptor.clone();
                        let connect = || async {
                            let (socket, peer) = socket2.accept().await?;
                            let (socket, info) =
                                wrap_tcp_connection(socket, ConnectionType::Direct)?;
                            log::debug!("Got connection from {}!", peer);
                            let (transit, finalizer) = handshake_exchange(
                                is_leader,
                                tside.clone(),
                                socket,
                                &ConnectionType::Direct,
                                &*cryptor,
                                transit_key.clone(),
                            )
                            .await?;
                            Result::<_, crypto::TransitHandshakeError>::Ok((
                                transit, finalizer, info,
                            ))
                        };
                        loop {
                            match connect().await {
                                Ok(success) => break Ok(success),
                                Err(err) => {
                                    log::debug!(
                                        "Some handshake failed on the listening port: {:?}",
                                        err
                                    );
                                    continue;
                                },
                            }
                        }
                    })
                    .map(|fut| {
                        Box::pin(fut)
                            as BoxFuture<Result<HandshakeResult, crypto::TransitHandshakeError>>
                    }),
                ),
            )
                as BoxIterator<BoxFuture<Result<HandshakeResult, crypto::TransitHandshakeError>>>;
        }
        connectors.collect::<futures::stream::futures_unordered::FuturesUnordered<_>>()
    }
}

/// Trait abstracting our socket used for communicating over the wire.
///
/// Will be primarily instantiated by either a TCP or web socket. Custom methods
/// will be added in the future.
pub(self) trait TransitTransport:
    AsyncRead + AsyncWrite + std::any::Any + Unpin + Send
{
}

impl<T> TransitTransport for T where T: AsyncRead + AsyncWrite + std::any::Any + Unpin + Send {}

/**
 * An established Transit connection.
 *
 * While you can manually send and receive bytes over the TCP stream, this is not recommended as the transit protocol
 * also specifies an encrypted record pipe that does all the hard work for you. See the provided methods.
 */
pub struct Transit {
    /** Raw transit connection */
    socket: Box<dyn TransitTransport>,
    tx: Box<dyn crypto::TransitCryptoEncrypt>,
    rx: Box<dyn crypto::TransitCryptoDecrypt>,
}

impl Transit {
    /** Receive and decrypt one message from the other side. */
    pub async fn receive_record(&mut self) -> Result<Box<[u8]>, TransitError> {
        self.rx.decrypt(&mut self.socket).await
    }

    /** Send an encrypted message to the other side */
    pub async fn send_record(&mut self, plaintext: &[u8]) -> Result<(), TransitError> {
        assert!(!plaintext.is_empty());
        self.tx.encrypt(&mut self.socket, plaintext).await
    }

    pub async fn flush(&mut self) -> Result<(), TransitError> {
        log::debug!("Flush");
        self.socket.flush().await.map_err(Into::into)
    }

    /** Convert the transit connection to a [`Stream`]/[`Sink`] pair */
    pub fn split(
        self,
    ) -> (
        impl futures::sink::Sink<Box<[u8]>, Error = TransitError>,
        impl futures::stream::Stream<Item = Result<Box<[u8]>, TransitError>>,
    ) {
        use futures::io::AsyncReadExt;

        let (reader, writer) = self.socket.split();
        (
            futures::sink::unfold(
                (writer, self.tx),
                |(mut writer, mut tx), plaintext: Box<[u8]>| async move {
                    tx.encrypt(&mut writer, &plaintext)
                        .await
                        .map(|()| (writer, tx))
                },
            ),
            futures::stream::try_unfold((reader, self.rx), |(mut reader, mut rx)| async move {
                rx.decrypt(&mut reader)
                    .await
                    .map(|record| Some((record, (reader, rx))))
            }),
        )
    }
}

type HandshakeResult = (
    Box<dyn TransitTransport>,
    Box<dyn crypto::TransitCryptoInitFinalizer>,
    TransitInfo,
);

/**
 * Do a transit handshake exchange, to establish a direct connection.
 *
 * This automatically does the relay handshake first if necessary. On the follower
 * side, the future will successfully run to completion if a connection could be
 * established. On the leader side, the handshake is not 100% completed: the caller
 * must write `Ok\n` into the stream that should be used (and optionally `Nevermind\n`
 * into all others).
 */
async fn handshake_exchange(
    is_leader: bool,
    tside: Arc<String>,
    mut socket: Box<dyn TransitTransport>,
    host_type: &ConnectionType,
    cryptor: &dyn crypto::TransitCryptoInit,
    key: Arc<Key<TransitKey>>,
) -> Result<
    (
        Box<dyn TransitTransport>,
        Box<dyn crypto::TransitCryptoInitFinalizer>,
    ),
    crypto::TransitHandshakeError,
> {
    if host_type != &ConnectionType::Direct {
        log::trace!("initiating relay handshake");

        let sub_key = key.derive_subkey_from_purpose::<crate::GenericKey>("transit_relay_token");
        socket
            .write_all(format!("please relay {} for side {}\n", sub_key.to_hex(), tside).as_bytes())
            .await?;
        let mut rx = [0u8; 3];
        socket.read_exact(&mut rx).await?;
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(
            ok_msg == rx,
            crypto::TransitHandshakeError::RelayHandshakeFailed
        );
    }

    let finalizer = if is_leader {
        cryptor.handshake_leader(&mut socket).await?
    } else {
        cryptor.handshake_follower(&mut socket).await?
    };

    Ok((socket, finalizer))
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    pub fn test_abilities_encoding() {
        assert_eq!(
            serde_json::to_value(Abilities::ALL_ABILITIES).unwrap(),
            json!([{"type": "direct-tcp-v1"}, {"type": "relay-v1"}])
        );
        assert_eq!(
            serde_json::to_value(Abilities::FORCE_DIRECT).unwrap(),
            json!([{"type": "direct-tcp-v1"}])
        );
    }

    #[test]
    pub fn test_hints_encoding() {
        assert_eq!(
            serde_json::to_value(Hints::new(
                [DirectHint {
                    hostname: "localhost".into(),
                    port: 1234
                }],
                [RelayHint::new(
                    Some("default".into()),
                    [DirectHint::new("transit.magic-wormhole.io", 4001)],
                    ["ws://transit.magic-wormhole.io/relay".parse().unwrap(),],
                )]
            ))
            .unwrap(),
            json!([
                {
                    "type": "direct-tcp-v1",
                    "hostname": "localhost",
                    "port": 1234
                },
                {
                    "type": "relay-v1",
                    "name": "default",
                    "hints": [
                        {
                            "type": "direct-tcp-v1",
                            "hostname": "transit.magic-wormhole.io",
                            "port": 4001,
                        },
                        {
                            "type": "websocket",
                            "url": "ws://transit.magic-wormhole.io/relay",
                        },
                    ]
                }
            ])
        )
    }
}
