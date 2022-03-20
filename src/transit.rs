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
#[allow(unused_imports)] /* We need them for the docs */
use futures::{future::TryFutureExt, Sink, SinkExt, Stream, StreamExt, TryStreamExt};
use log::*;
use std::{collections::HashSet, sync::Arc};
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::aead::{Aead, NewAead};

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

/// Private, because we try multiple handshakes and only
/// one needs to succeed
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
enum TransitHandshakeError {
    #[error("Handshake failed")]
    HandshakeFailed,
    #[error("Relay handshake failed")]
    RelayHandshakeFailed,
    #[error("Malformed peer address")]
    BadAddress(
        #[from]
        #[source]
        std::net::AddrParseError,
    ),
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

/**
 * Defines a way to find the other side.
 *
 * Each ability comes with a set of [`Hints`] to encode how to meet up.
 */
#[derive(Copy, Clone, Debug, Default)]
pub struct Abilities {
    /** Direct connection to the peer */
    pub direct_tcp_v1: bool,
    /** Connection over a TCP relay */
    pub relay_v1: bool,
    /** Connection over a TCP or WebSocket relay */
    pub relay_v2: bool,
}

impl Abilities {
    pub const ALL_ABILITIES: Self = Self {
        direct_tcp_v1: true,
        relay_v1: true,
        relay_v2: true,
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
        relay_v2: false,
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
        relay_v2: true,
    };

    pub fn can_direct(&self) -> bool {
        self.direct_tcp_v1
    }

    pub fn can_relay(&self) -> bool {
        self.relay_v1 || self.relay_v2
    }

    /** Keep only abilities that both sides support */
    pub fn intersect(mut self, other: &Self) -> Self {
        self.direct_tcp_v1 &= other.direct_tcp_v1;
        self.relay_v1 &= other.relay_v1;
        self.relay_v2 &= other.relay_v2;
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
        if self.relay_v2 {
            hints.push(serde_json::json!({
                "type": "relay-v2",
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
                Ability::RelayV2 => {
                    abilities.relay_v2 = true;
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
    RelayV1 {
        hints: HashSet<DirectHint>,
    },
    RelayV2(RelayHint),
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
                HintSerde::RelayV1 { hints } => {
                    relay.push(RelayHint {
                        tcp: hints,
                        ..RelayHint::default()
                    });
                },
                HintSerde::RelayV2(hint) => {
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
        let relay = self.relay.iter().flat_map(|hint| {
            [
                HintSerde::RelayV1 {
                    hints: hint.tcp.clone(),
                },
                HintSerde::RelayV2(hint.clone()),
            ]
        });
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
    Tcp(DirectHint),
    Websocket {
        url: url::Url,
    },
    #[serde(other)]
    Unknown,
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
    /** Human readable name */
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

    pub fn from_urls(name: Option<String>, urls: impl IntoIterator<Item = url::Url>) -> Self {
        let mut this = Self {
            name,
            ..Self::default()
        };
        for url in urls.into_iter() {
            match url.scheme() {
                "tcp" => {
                    this.tcp.insert(DirectHint {
                        hostname: url.host_str().expect("Missing hostname in relay URL (also TODO error handling)").into(),
                        port: url.port().expect("Missing port in relay URL (also TODO error handling)"),
                    });
                },
                "ws" | "wss" => {
                    this.ws.insert(url);
                },
                _ => {
                    // Do we fail or do we ignore?
                    todo!("TODO error handling");
                },
            }
        }
        this
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

impl TryFrom<&DirectHint> for std::net::IpAddr {
    type Error = std::net::AddrParseError;
    fn try_from(hint: &DirectHint) -> Result<std::net::IpAddr, std::net::AddrParseError> {
        hint.hostname.parse()
    }
}

impl TryFrom<&DirectHint> for std::net::SocketAddr {
    type Error = std::net::AddrParseError;
    /** This does not do the obvious thing and also implicitly maps all V4 addresses into V6 */
    fn try_from(hint: &DirectHint) -> Result<std::net::SocketAddr, std::net::AddrParseError> {
        let addr = hint.try_into()?;
        let addr = match addr {
            std::net::IpAddr::V4(v4) => std::net::IpAddr::V6(v4.to_ipv6_mapped()),
            std::net::IpAddr::V6(_) => addr,
        };
        Ok(std::net::SocketAddr::new(addr, hint.port))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TransitInfo {
    Direct,
    Relay { name: Option<String> },
}

type TransitConnection = (TcpStream, TransitInfo);

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
    #[error("No V4 addresses were found for the selected STUN server")]
    ServerIsV4Only,
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
async fn get_external_ip() -> Result<(std::net::SocketAddr, TcpStream), StunError> {
    let mut socket = connect_custom(
        &"[::]:0".parse::<std::net::SocketAddr>().unwrap().into(),
        &PUBLIC_STUN_SERVER
            .to_socket_addrs()?
            /* If you find yourself behind a NAT66, open an issue */
            .find(|x| x.is_ipv4())
            /* TODO add a helper method to stdlib for this */
            .map(|addr| match addr {
                std::net::SocketAddr::V4(v4) => std::net::SocketAddr::new(
                    std::net::IpAddr::V6(v4.ip().to_ipv6_mapped()),
                    v4.port(),
                ),
                std::net::SocketAddr::V6(_) => unreachable!(),
            })
            .ok_or(StunError::ServerIsV4Only)?
            .into(),
    )
    .await?;

    use bytecodec::{DecodeExt, EncodeExt};
    use std::net::{SocketAddr, ToSocketAddrs};
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

    fn decode_address(buf: &[u8]) -> Result<SocketAddr, bytecodec::Error> {
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
        let external_addr = external_addr.unwrap();

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
    let external_addr = decode_address(&buf[..20 + len as usize])?;

    Ok((external_addr, socket))
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
        /* Do a STUN query to get our public IP. If it works, we must reuse the same socket (port)
         * so that we will be NATted to the same port again. If it doesn't, simply bind a new socket
         * and use that instead.
         */
        let socket: MaybeConnectedSocket =
            match async_std::future::timeout(std::time::Duration::from_secs(4), get_external_ip())
                .await
                .map_err(|_| StunError::Timeout)
            {
                Ok(Ok((external_ip, stream))) => {
                    log::debug!("Our external IP address is {}", external_ip);
                    our_hints.direct_tcp.insert(DirectHint {
                        hostname: external_ip.ip().to_string(),
                        port: external_ip.port(),
                    });
                    stream.into()
                },
                // TODO replace with .flatten() once stable
                // https://github.com/rust-lang/rust/issues/70142
                Err(err) | Ok(Err(err)) => {
                    log::warn!("Failed to get external address via STUN, {}", err);
                    let socket =
                        socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None)
                            .unwrap();
                    set_socket_opts(&socket)?;

                    socket
                        .bind(&"[::]:0".parse::<std::net::SocketAddr>().unwrap().into())
                        .unwrap();

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
            get_if_addrs::get_if_addrs()?
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

        listener = Some((socket, socket2));
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
    ) -> Result<Transit, TransitConnectError> {
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

        let (mut transit, mut host_type) = async_std::future::timeout(
            std::time::Duration::from_secs(60),
            connection_stream.next(),
        )
        .await
        .map_err(|_| {
            log::debug!("`leader_connect` timed out");
            TransitConnectError::Handshake
        })?
        .ok_or(TransitConnectError::Handshake)?;

        if host_type != TransitInfo::Direct && our_abilities.can_direct() {
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
                while let Some((new_transit, new_host_type)) = connection_stream.next().await {
                    /* We already got a connection, so we're only interested in direct ones */
                    if new_host_type == TransitInfo::Direct {
                        transit = new_transit;
                        host_type = new_host_type;
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

        transit.socket.write_all(b"go\n").await?;
        match host_type {
            TransitInfo::Direct => {
                log::info!(
                    "Established direct transit connection to '{}'",
                    transit.socket.peer_addr().unwrap()
                );
            },
            TransitInfo::Relay { name: Some(name) } => {
                log::info!(
                    "Established transit connection via relay '{}' ({})",
                    name,
                    transit.socket.peer_addr().unwrap()
                );
            },
            TransitInfo::Relay { name: None } => {
                log::info!(
                    "Established transit connection via relay ({})",
                    transit.socket.peer_addr().unwrap()
                );
            },
        }

        Ok(transit)
    }

    /**
     * Connect to the other side, as receiver
     */
    pub async fn follower_connect(
        self,
        transit_key: Key<TransitKey>,
        their_abilities: Abilities,
        their_hints: Arc<Hints>,
    ) -> Result<Transit, TransitConnectError> {
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
            Ok(Some((transit, host_type))) => {
                match host_type {
                    TransitInfo::Direct => {
                        log::info!(
                            "Established direct transit connection to '{}'",
                            transit.socket.peer_addr().unwrap()
                        );
                    },
                    TransitInfo::Relay { name: Some(name) } => {
                        log::info!(
                            "Established transit connection via relay '{}' ({})",
                            name,
                            transit.socket.peer_addr().unwrap()
                        );
                    },
                    TransitInfo::Relay { name: None } => {
                        log::info!(
                            "Established transit connection via relay ({})",
                            transit.socket.peer_addr().unwrap()
                        );
                    },
                }
                Ok(transit)
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
    ) -> impl Stream<Item = Result<(Transit, TransitInfo), TransitHandshakeError>> + 'static {
        assert!(socket.is_some() == our_abilities.can_direct());

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        let tside = Arc::new(hex::encode(rand::random::<[u8; 8]>()));

        /* Iterator of futures yielding a connection. They'll be then mapped with the handshake, collected into
         * a Vec and polled concurrently.
         */
        use futures::future::BoxFuture;
        type BoxIterator<T> = Box<dyn Iterator<Item = T>>;
        type ConnectorFuture = BoxFuture<'static, Result<TransitConnection, TransitHandshakeError>>;
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
                                let dest_addr = std::net::SocketAddr::try_from(&hint)?;
                                log::debug!("Connecting directly to {}", dest_addr);
                                let socket = connect_custom(&local_addr, &dest_addr.into()).await?;
                                log::debug!("Connected to {}!", dest_addr);
                                Ok((socket, TransitInfo::Direct))
                            }
                        })
                        .map(|fut| Box::pin(fut) as ConnectorFuture),
                ),
            ) as BoxIterator<ConnectorFuture>;
            Some(socket2)
        } else {
            None
        };

        /* Relay hints. Make sure that both sides adverize it, since it is fine to support it without providing own hints. */
        if our_abilities.can_relay() && their_abilities.can_relay() {
            /* Collect intermediate into HashSet for deduplication */
            let mut relay_hints = Vec::<RelayHint>::new();
            relay_hints.extend(our_hints.relay.iter().take(2).cloned());
            for hint in their_hints.relay.iter().take(2).cloned() {
                hint.merge_into(&mut relay_hints);
            }

            /* Take a relay hint and try to connect to it */
            async fn hint_connector(
                host: DirectHint,
                name: Option<String>,
            ) -> Result<TransitConnection, TransitHandshakeError> {
                log::debug!("Connecting to relay {}", host);
                let transit = TcpStream::connect((host.hostname.as_str(), host.port))
                    .err_into::<TransitHandshakeError>()
                    .await?;
                log::debug!("Connected to {}!", host);

                Ok((transit, TransitInfo::Relay { name }))
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
                            hint_connector(host, name).await
                        })
                        .map(|fut| Box::pin(fut) as ConnectorFuture),
                ),
            ) as BoxIterator<ConnectorFuture>;
        }

        /* Do a handshake on all our found connections */
        let transit_key2 = transit_key.clone();
        let tside2 = tside.clone();
        let mut connectors = Box::new(
            connectors
                .map(move |fut| {
                    let transit_key = transit_key2.clone();
                    let tside = tside2.clone();
                    async move {
                        let (socket, host_type) = fut.await?;
                        let transit =
                            handshake_exchange(is_leader, tside, socket, &host_type, transit_key)
                                .await?;
                        Ok((transit, host_type))
                    }
                })
                .map(|fut| {
                    Box::pin(fut)
                        as BoxFuture<Result<(Transit, TransitInfo), TransitHandshakeError>>
                }),
        )
            as BoxIterator<BoxFuture<Result<(Transit, TransitInfo), TransitHandshakeError>>>;

        /* Also listen on some port just in case. */
        if let Some(socket2) = socket2 {
            connectors = Box::new(
                connectors.chain(
                    std::iter::once(async move {
                        let transit_key = transit_key.clone();
                        let tside = tside.clone();
                        let connect = || async {
                            let (stream, peer) = socket2.accept().await?;
                            log::debug!("Got connection from {}!", peer);
                            let transit = handshake_exchange(
                                is_leader,
                                tside.clone(),
                                stream,
                                &TransitInfo::Direct,
                                transit_key.clone(),
                            )
                            .await?;
                            Result::<_, TransitHandshakeError>::Ok((transit, TransitInfo::Direct))
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
                            as BoxFuture<Result<(Transit, TransitInfo), TransitHandshakeError>>
                    }),
                ),
            )
                as BoxIterator<BoxFuture<Result<(Transit, TransitInfo), TransitHandshakeError>>>;
        }
        connectors.collect::<futures::stream::futures_unordered::FuturesUnordered<_>>()
    }
}

/**
 * An established Transit connection.
 *
 * While you can manually send and receive bytes over the TCP stream, this is not recommended as the transit protocol
 * also specifies an encrypted record pipe that does all the hard work for you. See the provided methods.
 */
pub struct Transit {
    /** Raw transit connection */
    socket: TcpStream,
    /** Our key, used for sending */
    pub skey: Key<TransitTxKey>,
    /** Their key, used for receiving */
    pub rkey: Key<TransitRxKey>,
    /** Nonce for sending */
    pub snonce: secretbox::Nonce,
    /**
     * Nonce for receiving
     *
     * We'll count as receiver and track if messages come in in order
     */
    pub rnonce: secretbox::Nonce,
}

impl Transit {
    /** Receive and decrypt one message from the other side. */
    pub async fn receive_record(&mut self) -> Result<Box<[u8]>, TransitError> {
        Transit::receive_record_inner(&mut self.socket, &self.rkey, &mut self.rnonce).await
    }

    async fn receive_record_inner(
        socket: &mut (impl futures::io::AsyncRead + Unpin),
        rkey: &Key<TransitRxKey>,
        nonce: &mut secretbox::Nonce,
    ) -> Result<Box<[u8]>, TransitError> {
        let enc_packet = {
            // 1. read 4 bytes from the stream. This represents the length of the encrypted packet.
            let length = {
                let mut length_arr: [u8; 4] = [0; 4];
                socket.read_exact(&mut length_arr[..]).await?;
                u32::from_be_bytes(length_arr) as usize
            };
            ensure!(
                length >= secretbox::NONCE_SIZE,
                Error::new(
                    ErrorKind::InvalidData,
                    "Message must be long enough to contain at least the nonce"
                )
            );

            // 2. read that many bytes into an array (or a vector?)
            let mut buffer = Vec::with_capacity(length);
            let len = socket.take(length as u64).read_to_end(&mut buffer).await?;
            use std::io::{Error, ErrorKind};
            ensure!(
                len == length,
                Error::new(ErrorKind::UnexpectedEof, "failed to read whole message")
            );
            buffer
        };

        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = {
            let (received_nonce, ciphertext) = enc_packet.split_at(secretbox::NONCE_SIZE);
            {
                // Nonce check
                ensure!(
                    nonce.as_slice() == received_nonce,
                    TransitError::Nonce(received_nonce.into(), nonce.as_slice().into()),
                );

                crate::util::sodium_increment_be(nonce);
            }

            let cipher = secretbox::XSalsa20Poly1305::new(secretbox::Key::from_slice(rkey));
            cipher
                .decrypt(secretbox::Nonce::from_slice(received_nonce), ciphertext)
                /* TODO replace with (TransitError::Crypto) after the next xsalsa20poly1305 update */
                .map_err(|_| TransitError::Crypto)?
        };

        Ok(plaintext.into_boxed_slice())
    }

    /** Send an encrypted message to the other side */
    pub async fn send_record(&mut self, plaintext: &[u8]) -> Result<(), TransitError> {
        Transit::send_record_inner(&mut self.socket, &self.skey, plaintext, &mut self.snonce).await
    }

    async fn send_record_inner(
        socket: &mut (impl futures::io::AsyncWrite + Unpin),
        skey: &Key<TransitTxKey>,
        plaintext: &[u8],
        nonce: &mut secretbox::Nonce,
    ) -> Result<(), TransitError> {
        let sodium_key = secretbox::Key::from_slice(skey);

        let ciphertext = {
            let nonce_le = secretbox::Nonce::from_slice(nonce);

            let cipher = secretbox::XSalsa20Poly1305::new(sodium_key);
            cipher
                .encrypt(nonce_le, plaintext)
                /* TODO replace with (TransitError::Crypto) after the next xsalsa20poly1305 update */
                .map_err(|_| TransitError::Crypto)?
        };

        // send the encrypted record
        socket
            .write_all(&((ciphertext.len() + nonce.len()) as u32).to_be_bytes())
            .await?;
        socket.write_all(nonce).await?;
        socket.write_all(&ciphertext).await?;

        crate::util::sodium_increment_be(nonce);

        Ok(())
    }

    pub async fn flush(&mut self) -> Result<(), TransitError> {
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
                (writer, self.skey, self.snonce),
                |(mut writer, skey, mut nonce), plaintext: Box<[u8]>| async move {
                    Transit::send_record_inner(
                        &mut writer,
                        &skey as &Key<TransitTxKey>,
                        &plaintext,
                        &mut nonce,
                    )
                    .await
                    .map(|()| (writer, skey, nonce))
                },
            ),
            futures::stream::try_unfold(
                (reader, self.rkey, self.rnonce),
                |(mut reader, rkey, mut nonce)| async move {
                    Transit::receive_record_inner(&mut reader, &rkey, &mut nonce)
                        .await
                        .map(|record| Some((record, (reader, rkey, nonce))))
                },
            ),
        )
    }
}

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
    mut socket: TcpStream,
    host_type: &TransitInfo,
    key: Arc<Key<TransitKey>>,
) -> Result<Transit, TransitHandshakeError> {
    // 9. create record keys
    let (rkey, skey) = if is_leader {
        let rkey = key.derive_subkey_from_purpose("transit_record_receiver_key");
        let skey = key.derive_subkey_from_purpose("transit_record_sender_key");
        (rkey, skey)
    } else {
        /* The order here is correct. The "sender" and "receiver" side are a misnomer and should be called
         * "leader" and "follower" instead. As a follower, we use the leader key for receiving and our
         * key for sending.
         */
        let rkey = key.derive_subkey_from_purpose("transit_record_sender_key");
        let skey = key.derive_subkey_from_purpose("transit_record_receiver_key");
        (rkey, skey)
    };

    if host_type != &TransitInfo::Direct {
        trace!("initiating relay handshake");

        let sub_key = key.derive_subkey_from_purpose::<crate::GenericKey>("transit_relay_token");
        socket
            .write_all(format!("please relay {} for side {}\n", sub_key.to_hex(), tside).as_bytes())
            .await?;
        let mut rx = [0u8; 3];
        socket.read_exact(&mut rx).await?;
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, TransitHandshakeError::RelayHandshakeFailed);
    }

    if is_leader {
        // for transmit mode, send send_handshake_msg and compare.
        // the received message with send_handshake_msg
        socket
            .write_all(
                format!(
                    "transit sender {} ready\n\n",
                    key.derive_subkey_from_purpose::<crate::GenericKey>("transit_sender")
                        .to_hex()
                )
                .as_bytes(),
            )
            .await?;

        // The received message "transit sender $hash ready\n\n" has exactly 89 bytes
        // TODO do proper line parsing one day, this is atrocious
        let mut rx: [u8; 89] = [0; 89];
        socket.read_exact(&mut rx).await?;

        let expected_rx_handshake = format!(
            "transit receiver {} ready\n\n",
            key.derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver")
                .to_hex()
        );
        ensure!(
            &rx[..] == expected_rx_handshake.as_bytes(),
            TransitHandshakeError::HandshakeFailed,
        );
    } else {
        // for receive mode, send receive_handshake_msg and compare.
        // the received message with send_handshake_msg
        socket
            .write_all(
                format!(
                    "transit receiver {} ready\n\n",
                    key.derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver")
                        .to_hex(),
                )
                .as_bytes(),
            )
            .await?;

        // The received message "transit receiver $hash ready\n\n" has exactly 87 bytes
        // Three bytes for the "go\n" ack
        // TODO do proper line parsing one day, this is atrocious
        let mut rx: [u8; 90] = [0; 90];
        socket.read_exact(&mut rx).await?;

        let expected_tx_handshake = format!(
            "transit sender {} ready\n\ngo\n",
            key.derive_subkey_from_purpose::<crate::GenericKey>("transit_sender")
                .to_hex(),
        );
        ensure!(
            &rx[..] == expected_tx_handshake.as_bytes(),
            TransitHandshakeError::HandshakeFailed
        );
    }

    Ok(Transit {
        socket,
        skey,
        rkey,
        snonce: Default::default(),
        rnonce: Default::default(),
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    #[test]
    pub fn test_abilities_encoding() {
        assert_eq!(
            serde_json::to_value(Abilities::ALL_ABILITIES).unwrap(),
            json!([{"type": "direct-tcp-v1"}, {"type": "relay-v1"}, {"type": "relay-v2"}])
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
                    "hints": [
                        {
                            "hostname": "transit.magic-wormhole.io",
                            "port": 4001,
                        }
                    ]
                },
                {
                    "type": "relay-v2",
                    "name": "default",
                    "hints": [
                        {
                            "type": "tcp",
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
