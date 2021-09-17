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
//! Each side might implement (or use/enable) some [abilities](Ability).
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
use std::{collections::HashSet, str::FromStr, sync::Arc};
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::aead::{Aead, NewAead};

/// ULR to a default hosted relay server. Please don't abuse or DOS.
pub const DEFAULT_RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";

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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
pub enum Ability {
    /**
     * Try to connect directly to the other side via TCP.
     *
     * This usually requires both participants to be in the same network. [`DirectHint`s](DirectHint) are sent,
     * which encode all local IP addresses for the other side to find us.
     */
    DirectTcpV1,
    /**
     * UNSTABLE; NOT IMPLEMENTED!
     * Try to connect directly to the other side via UDT.
     *
     * This supersedes [`Ability::DirectTcpV1`] because it has several advantages:
     *
     * - Works with stateful firewalls, no need to open up ports
     * - Works despite many NAT types if combined with STUN
     * - UDT has a few other interesting performance-related properties that make it better
     *   suited than TCP (it's literally called "UDP-based Data Transfer Protocol")
     */
    DirectUdtV1,
    /** Try to meet the other side at a relay. */
    RelayV1,
    /* TODO Fix once https://github.com/serde-rs/serde/issues/912 is done */
    #[serde(other)]
    Other,
}

impl Ability {
    pub fn all_abilities() -> Vec<Ability> {
        vec![Self::DirectTcpV1, Self::DirectUdtV1, Self::RelayV1]
    }

    /**
     * If you absolutely don't want to use any relay servers.
     *
     * If the other side forces relay usage or doesn't support any of your connection modes
     * the attempt will fail.
     */
    pub fn force_direct() -> Vec<Ability> {
        vec![Self::DirectTcpV1, Self::DirectUdtV1]
    }

    /**
     * If you don't want to disclose your IP address to your peer
     *
     * If the other side forces a the usage of a direct connection the attempt will fail.
     * Note that the other side might control the relay server being used, if you really
     * don't want your IP to potentially be disclosed use TOR instead (not supported by
     * the Rust implementation yet).
     */
    pub fn force_relay() -> Vec<Ability> {
        vec![Self::RelayV1]
    }
}

#[derive(Clone, Debug, Default)]
pub struct Hints {
    pub direct_tcp: Vec<DirectHint>,
    pub relay: Vec<DirectHint>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, Hash, derive_more::Display)]
#[display(fmt = "tcp://{}:{}", hostname, port)]
pub struct DirectHint {
    // DirectHint also contains a `priority` field, but it is underspecified
    // and we won't use it
    // pub priority: f32,
    pub hostname: String,
    pub port: u16,
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
enum HostType {
    Direct,
    Relay,
}

pub struct RelayUrl {
    pub host: String,
    pub port: u16,
}

impl FromStr for RelayUrl {
    type Err = &'static str;

    fn from_str(url: &str) -> Result<Self, &'static str> {
        // TODO use proper URL parsing
        let v: Vec<&str> = url.split(':').collect();
        if v.len() == 3 && v[0] == "tcp" {
            v[2].parse()
                .map(|port| RelayUrl {
                    host: v[1].to_string(),
                    port,
                })
                .map_err(|_| "Cannot parse relay url port")
        } else {
            Err("Incorrect relay server url format")
        }
    }
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
    socket.set_nonblocking(true)?;
    /* Set our custum options */
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;

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

/**
 * Initialize a relay handshake
 *
 * Bind a port and generate our [`Hints`]. This does not do any communication yet.
 */
pub async fn init(
    abilities: Vec<Ability>,
    relay_url: &RelayUrl,
) -> Result<TransitConnector, std::io::Error> {
    let mut our_hints = Hints::default();
    let mut listener = None;

    /* Detect our local IP addresses if the ability is enabled */
    if abilities.contains(&Ability::DirectTcpV1) {
        /* Bind a port and find out which addresses it has */
        let socket =
            socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None).unwrap();
        socket.set_nonblocking(false).unwrap();
        socket.set_reuse_address(true).unwrap();
        socket.set_reuse_port(true).unwrap();

        socket
            .bind(&"[::]:0".parse::<std::net::SocketAddr>().unwrap().into())
            .unwrap();

        let port = socket.local_addr().unwrap().as_socket().unwrap().port();

        /* Do the same thing again, but this time open a listener on that port.
         * This sadly doubles the number of hints, but the method above doesn't work
         * for systems which don't have any firewalls.
         */
        let socket2 = TcpListener::bind("[::]:0").await?;
        let port2 = socket2.local_addr().unwrap().port();

        our_hints.direct_tcp.extend(
            get_if_addrs::get_if_addrs()?
                .iter()
                .filter(|iface| !iface.is_loopback())
                .flat_map(|ip|
                    /* TODO replace with array once into_iter works as it should */
                    vec![
                        DirectHint {
                            hostname: ip.ip().to_string(),
                            port,
                        },
                        DirectHint {
                            hostname: ip.ip().to_string(),
                            port: port2,
                        },
                    ].into_iter()),
        );

        listener = Some((socket, socket2));
    }

    if abilities.contains(&Ability::RelayV1) {
        our_hints.relay.push(DirectHint {
            hostname: relay_url.host.clone(),
            port: relay_url.port,
        });
    }

    Ok(TransitConnector {
        sockets: listener,
        our_abilities: Arc::new(abilities),
        our_hints: Arc::new(our_hints),
    })
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
    sockets: Option<(socket2::Socket, TcpListener)>,
    our_abilities: Arc<Vec<Ability>>,
    our_hints: Arc<Hints>,
}

impl TransitConnector {
    pub fn our_abilities(&self) -> &Arc<Vec<Ability>> {
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
        their_abilities: Arc<Vec<Ability>>,
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
                our_abilities.clone(),
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

        let (mut transit, host_type) = async_std::future::timeout(
            std::time::Duration::from_secs(60),
            connection_stream.next(),
        )
        .await
        .map_err(|_| {
            log::debug!("`leader_connect` timed out");
            TransitConnectError::Handshake
        })?
        .ok_or(TransitConnectError::Handshake)?;

        if host_type == HostType::Relay && our_abilities.contains(&Ability::DirectTcpV1) {
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
                    if new_host_type == HostType::Direct {
                        transit = new_transit;
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
        info!(
            "Established transit connection to '{}'",
            transit.socket.peer_addr().unwrap()
        );

        Ok(transit)
    }

    /**
     * Connect to the other side, as receiver
     */
    pub async fn follower_connect(
        self,
        transit_key: Key<TransitKey>,
        their_abilities: Arc<Vec<Ability>>,
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
                log::debug!(
                    "Established a {} transit connection.",
                    if host_type == HostType::Direct {
                        "direct"
                    } else {
                        "relay"
                    }
                );
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
        our_abilities: Arc<Vec<Ability>>,
        our_hints: Arc<Hints>,
        their_abilities: Arc<Vec<Ability>>,
        their_hints: Arc<Hints>,
        socket: Option<(socket2::Socket, TcpListener)>,
    ) -> impl Stream<Item = Result<(Transit, HostType), TransitHandshakeError>> + 'static {
        assert!(socket.is_some() == our_abilities.contains(&Ability::DirectTcpV1));

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        let tside = Arc::new(hex::encode(rand::random::<[u8; 8]>()));

        /* Iterator of futures yielding a connection. They'll be then mapped with the handshake, collected into
         * a Vec and polled concurrently.
         */
        use futures::future::BoxFuture;
        type BoxIterator<T> = Box<dyn Iterator<Item = T>>;
        type ConnectorFuture =
            BoxFuture<'static, Result<(TcpStream, HostType), TransitHandshakeError>>;
        // type ConnectorIterator = Box<dyn Iterator<Item = ConnectorFuture>>;
        let mut connectors: BoxIterator<ConnectorFuture> = Box::new(std::iter::empty());

        /* Create direct connection sockets, if we support it. If peer doesn't support it, their list of hints will
         * be empty and no entries will be pushed.
         */
        let socket2 = if let Some((socket, socket2)) = socket {
            let local_addr = Arc::new(socket.local_addr().unwrap());
            dbg!(&their_hints.direct_tcp);
            /* Connect to each hint of the peer */
            connectors = Box::new(
                connectors.chain(
                    their_hints
                        .direct_tcp
                        .clone()
                        .into_iter()
                        /* Nobody should have that many IP addresses, even with NATing */
                        .take(10)
                        .map(move |hint| {
                            let local_addr = local_addr.clone();
                            async move {
                                let dest_addr = std::net::SocketAddr::try_from(&hint)?;
                                log::debug!("Connecting directly to {}", dest_addr);
                                let socket = connect_custom(&local_addr, &dest_addr.into()).await?;
                                log::debug!("Connected to {}!", dest_addr);
                                Ok((socket, HostType::Direct))
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
        if our_abilities.contains(&Ability::RelayV1) && their_abilities.contains(&Ability::RelayV1)
        {
            connectors = Box::new(
                connectors.chain(
                /* TODO maybe take 2 at random instead of always the first two? */
                /* TODO also deduplicate the results list */
                our_hints
                    .relay
                    .clone()
                    .into_iter()
                    .take(2)
                    .chain(
                        their_hints
                            .relay
                            .clone()
                            .into_iter()
                            .take(2)
                    )
                    .map(|host| async move {
                        log::debug!("Connecting to relay {}", host);
                        let transit = TcpStream::connect((host.hostname.as_str(), host.port))
                            .err_into::<TransitHandshakeError>()
                            .await?;
                        log::debug!("Connected to {}!", host);

                        Ok((transit, HostType::Relay))
                    })
                    .map(|fut| Box::pin(fut) as ConnectorFuture)
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
                            handshake_exchange(is_leader, tside, socket, host_type, transit_key)
                                .await?;
                        Ok((transit, host_type))
                    }
                })
                .map(|fut| {
                    Box::pin(fut) as BoxFuture<Result<(Transit, HostType), TransitHandshakeError>>
                }),
        )
            as BoxIterator<BoxFuture<Result<(Transit, HostType), TransitHandshakeError>>>;

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
                                HostType::Direct,
                                transit_key.clone(),
                            )
                            .await?;
                            Result::<_, TransitHandshakeError>::Ok((transit, HostType::Direct))
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
                            as BoxFuture<Result<(Transit, HostType), TransitHandshakeError>>
                    }),
                ),
            )
                as BoxIterator<BoxFuture<Result<(Transit, HostType), TransitHandshakeError>>>;
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

            // 2. read that many bytes into an array (or a vector?)
            let mut buffer = Vec::with_capacity(length);
            socket.take(length as u64).read_to_end(&mut buffer).await?;
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

    /** Convert the transit connection to a [`Stream`]/[`Sink`] pair */
    pub fn split(
        self,
    ) -> (
        impl futures::sink::Sink<Box<[u8]>>,
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
    host_type: HostType,
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

    if host_type == HostType::Relay {
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
