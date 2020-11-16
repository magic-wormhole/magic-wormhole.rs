//! Connect two sides via TCP, no matter where they are
//!
//! This protocol is the second part where the Wormhole magic happens. It does not strictly require a Wormhole connection,
//! but it depends on some kind of secure communication channel to talk to the other side. Conveniently, Wormhole provides
//! exactly such a thing :)
//!
//! Both clients exchange messages containing hints on how to find each other. These may be local IP Addresses for in case they
//! are in the same network, or the URL to a relay server. In case a direct connection fails, both will connect to the relay server
//! which will transparently glue the connections together.
//!
//! Each side might implement (or use/enable) some [abilities](Ability).
//!
//! **Notice:** while the resulting TCP connection is naturally bi-directional, the handshake is not symmetric. There *must* be one
//! "sender" side and one "receiver" side (doing the respectively correct part of the handshake).

use crate::{Key, KeyPurpose};
use serde_derive::{Deserialize, Serialize};

use anyhow::{ensure, format_err, Context, Error, Result};
use async_std::io::prelude::WriteExt;
use async_std::io::Read;
use async_std::io::ReadExt;
use async_std::net::{TcpListener, TcpStream};
use futures::future::TryFutureExt;
use futures::StreamExt;
use log::*;
use pnet::datalink;
use pnet::ipnetwork::IpNetwork;
use sodiumoxide::crypto::secretbox;
use std::net::ToSocketAddrs;
use std::str::FromStr;
use std::sync::Arc;

/// ULR to a default hosted relay server. Please don't abuse or DOS.
pub const DEFAULT_RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";

pub struct TransitKey;
impl KeyPurpose for TransitKey {}
pub struct TransitRxKey;
impl KeyPurpose for TransitRxKey {}
pub struct TransitTxKey;
impl KeyPurpose for TransitTxKey {}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitType {
    pub abilities_v1: Vec<Ability>,
    pub hints_v1: Vec<Hint>,
}

/**
 * Defines a way to find the other side.
 *
 * Each ability comes with a set of [hints](Hint) to encode how to meet up.
 */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Ability {
    /**
     * Try to connect directly to the other side.
     *
     * This usually requires both participants to be in the same network. [`DirectHint`s](DirectHint) are sent,
     * which encode all local IP addresses for the other side to find us.
     */
    DirectTcpV1,
    /** Try to meet the other side at a relay. */
    RelayV1,
    /* TODO Fix once https://github.com/serde-rs/serde/issues/912 is done */
    #[serde(other)]
    Other,
}

impl Ability {
    pub fn all_abilities() -> Vec<Ability> {
        vec![Self::DirectTcpV1, Self::RelayV1]
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Hint {
    DirectTcpV1(DirectHint),
    RelayV1(RelayHint),
}

impl Hint {
    pub fn new_direct(priority: f32, hostname: &str, port: u16) -> Self {
        Hint::DirectTcpV1(DirectHint {
            priority,
            hostname: hostname.to_string(),
            port,
        })
    }

    pub fn new_relay(h: Vec<DirectHint>) -> Self {
        Hint::RelayV1(RelayHint { hints: h })
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type", rename = "direct-tcp-v1")]
pub struct DirectHint {
    pub priority: f32,
    pub hostname: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type", rename = "relay-v1")]
pub struct RelayHint {
    pub hints: Vec<DirectHint>,
}

#[derive(Debug, PartialEq)]
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
 * Initialize a relay handshake
 *
 * Bind a port and generate our [`TransitType`]. This does not do any communication yet.
 */
pub async fn init(abilities: Vec<Ability>, relay_url: &RelayUrl) -> Result<TransitConnector> {
    let listener = TcpListener::bind("[::]:0").await?;
    let port = listener.local_addr()?.port();

    let mut our_hints: Vec<Hint> = Vec::new();
    if abilities.contains(&Ability::DirectTcpV1) {
        our_hints.extend(
            datalink::interfaces()
                .iter()
                .filter(|iface| !datalink::NetworkInterface::is_loopback(iface))
                .flat_map(|iface| iface.ips.iter())
                .map(|n| n as &IpNetwork)
                // .filter(|ip| ip.is_ipv4()) // TODO why was that there can we remove it?
                .map(|ip| {
                    Hint::DirectTcpV1(DirectHint {
                        priority: 0.0,
                        hostname: ip.ip().to_string(),
                        port,
                    })
                }),
        );
    }
    if abilities.contains(&Ability::RelayV1) {
        our_hints.push(Hint::new_relay(vec![DirectHint {
            priority: 0.0,
            hostname: relay_url.host.clone(),
            port: relay_url.port,
        }]));
    }

    Ok(TransitConnector {
        listener,
        port,
        our_side_ttype: Arc::new(TransitType {
            abilities_v1: abilities,
            hints_v1: our_hints,
        }),
    })
}

/**
 * A partially set up [`Transit`] connection.
 *
 * For the transit handshake, each side generates a [ttype](`TransitType`) with all the hints to find the other. You need
 * to exchange it (as in: send yours, receive theirs) with them. This is outside of the transit protocol to be protocol
 * agnostic.
 */
pub struct TransitConnector {
    listener: TcpListener,
    port: u16,
    our_side_ttype: Arc<TransitType>,
}

impl TransitConnector {
    /** Send this one to the other side */
    pub fn our_side_ttype(&self) -> &Arc<TransitType> {
        &self.our_side_ttype
    }

    /**
     * Connect to the other side, as sender.
     */
    pub async fn sender_connect(
        self,
        transit_key: Key<TransitKey>,
        other_side_ttype: TransitType,
    ) -> Result<Transit> {
        let transit_key = Arc::new(transit_key);
        /* TODO This Deref thing is getting out of hand. Maybe implementing AsRef or some other trait may help? */
        debug!("transit key {}", hex::encode(&***transit_key));

        let port = self.port;
        let listener = self.listener;
        // let other_side_ttype = Arc::new(other_side_ttype);
        // TODO remove this one day
        let ttype = &*Box::leak(Box::new(other_side_ttype));

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        // extract peer's ip/hostname from 'ttype'
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectHint)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);
        // TODO: combine our relay hints with the peer's relay hints.

        let mut handshake_futures = Vec::new();
        for host in hosts {
            // TODO use async scopes to borrow instead of cloning one day
            let transit_key = transit_key.clone();
            let future = async_std::task::spawn(
                //async_std::future::timeout(Duration::from_secs(5),
                async move {
                    debug!("host: {:?}", host);
                    let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port)
                        .to_socket_addrs()
                        .unwrap();
                    let direct_host = direct_host_iter.next().unwrap();

                    debug!("peer host: {}", direct_host);

                    TcpStream::connect(direct_host)
                        .err_into::<Error>()
                        .and_then(|socket| tx_handshake_exchange(socket, host.0, &*transit_key))
                        .await
                },
            ); //);
            handshake_futures.push(future);
        }
        handshake_futures.push(async_std::task::spawn(async move {
            debug!("local host {}", port);

            /* Mixing and matching two different futures library probably isn't the
             * best idea, but here we are. Simply be careful about prelude::* imports
             * and don't have both StreamExt/FutureExt/… imported at once
             */
            use futures::stream::TryStreamExt;
            async_std::stream::StreamExt::skip_while(listener.incoming()
                .err_into::<Error>()
                .and_then(move |socket| {
                    /* Pinning a future + moving some value from outer scope is a bit painful */
                    let transit_key = transit_key.clone();
                    Box::pin(async move {
                        tx_handshake_exchange(socket, HostType::Direct, &*transit_key).await
                    })
                }),
                Result::is_err)
                /* We only care about the first that succeeds */
                .next()
                .await
                /* Next always returns Some because Incoming is an infinite stream. We gotta succeed _sometime_. */
                .unwrap()
        }));

        /* Try to get a Transit out of the first handshake that succeeds. If all fail,
         * we fail.
         */
        let transit;
        loop {
            if handshake_futures.is_empty() {
                return Err(format_err!("All handshakes failed or timed out"));
            }
            match futures::future::select_all(handshake_futures).await {
                (Ok(transit2), _index, remaining) => {
                    transit = transit2;
                    handshake_futures = remaining;
                    break;
                },
                (Err(e), _index, remaining) => {
                    debug!("Some handshake failed {:#}", e);
                    handshake_futures = remaining;
                },
            }
        }
        let mut transit = transit;

        /* Cancel all remaining non-finished handshakes */
        handshake_futures
            .into_iter()
            .map(async_std::task::JoinHandle::cancel)
            .for_each(std::mem::drop);

        debug!(
            "Sending 'go' message to {}",
            transit.socket.peer_addr().unwrap()
        );
        transit.socket.write_all(b"go\n").await?;

        Ok(transit)
    }

    /**
     * Connect to the other side, as receiver
     */
    pub async fn receiver_connect(
        self,
        transit_key: Key<TransitKey>,
        other_side_ttype: TransitType,
    ) -> Result<Transit> {
        let transit_key = Arc::new(transit_key);
        /* TODO This Deref thing is getting out of hand. Maybe implementing AsRef or some other trait may help? */
        debug!("transit key {}", hex::encode(&***transit_key));

        let port = self.port;
        let listener = self.listener;
        // let other_side_ttype = Arc::new(other_side_ttype);
        let ttype = &*Box::leak(Box::new(other_side_ttype)); // TODO remove this one day

        // 4. listen for connections on the port and simultaneously try connecting to the
        //    peer listening port.
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectHint)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);
        // TODO: combine our relay hints with the peer's relay hints.

        let mut handshake_futures = Vec::new();
        for host in hosts {
            let transit_key = transit_key.clone();

            let future = async_std::task::spawn(
                //async_std::future::timeout(Duration::from_secs(5),
                async move {
                    debug!("host: {:?}", host);
                    let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port)
                        .to_socket_addrs()
                        .unwrap();
                    let direct_host = direct_host_iter.next().unwrap();

                    debug!("peer host: {}", direct_host);

                    TcpStream::connect(direct_host)
                        .err_into::<Error>()
                        .and_then(|socket| rx_handshake_exchange(socket, host.0, &*transit_key))
                        .await
                },
            ); //);
            handshake_futures.push(future);
        }
        handshake_futures.push(async_std::task::spawn(async move {
            debug!("local host {}", port);

            /* Mixing and matching two different futures library probably isn't the
             * best idea, but here we are. Simply be careful about prelude::* imports
             * and don't have both StreamExt/FutureExt/… imported at once
             */
            use futures::stream::TryStreamExt;
            async_std::stream::StreamExt::skip_while(listener.incoming()
                .err_into::<Error>()
                .and_then(move |socket| {
                    /* Pinning a future + moving some value from outer scope is a bit painful */
                    let transit_key = transit_key.clone();
                    Box::pin(async move {
                        rx_handshake_exchange(socket, HostType::Direct, &*transit_key).await
                    })
                }),
                Result::is_err)
                /* We only care about the first that succeeds */
                .next()
                .await
                /* Next always returns Some because Incoming is an infinite stream. We gotta succeed _sometime_. */
                .unwrap()
        }));

        /* Try to get a Transit out of the first handshake that succeeds. If all fail,
         * we fail.
         */
        let transit;
        loop {
            if handshake_futures.is_empty() {
                return Err(format_err!("All handshakes failed or timed out"));
            }
            match futures::future::select_all(handshake_futures).await {
                (Ok(transit2), _index, remaining) => {
                    transit = transit2;
                    handshake_futures = remaining;
                    break;
                },
                (Err(e), _index, remaining) => {
                    debug!("Some handshake failed {:#}", e);
                    handshake_futures = remaining;
                },
            }
        }

        /* Cancel all remaining non-finished handshakes */
        handshake_futures
            .into_iter()
            .map(async_std::task::JoinHandle::cancel)
            .for_each(std::mem::drop);

        Ok(transit)
    }
}

pub struct Transit {
    pub socket: TcpStream,
    pub skey: Key<TransitTxKey>,
    pub rkey: Key<TransitRxKey>,
}

pub fn make_transit_ack_msg(sha256: &str, key: &[u8]) -> Result<Vec<u8>> {
    let plaintext = crate::transfer::TransitAck::new("ok", sha256).serialize();

    let nonce_slice: [u8; sodiumoxide::crypto::secretbox::NONCEBYTES] =
        [0; sodiumoxide::crypto::secretbox::NONCEBYTES];
    let nonce = secretbox::Nonce::from_slice(&nonce_slice[..]).unwrap();

    encrypt_record(&plaintext.as_bytes(), nonce, &key)
}

fn generate_transit_side() -> String {
    let x: [u8; 8] = rand::random();
    hex::encode(x)
}

pub async fn send_record(stream: &mut TcpStream, buf: &[u8]) -> std::io::Result<()> {
    let buf_length: u32 = buf.len() as u32;
    trace!("record size: {:?}", buf_length);
    let buf_length_array: [u8; 4] = buf_length.to_be_bytes();
    stream.write_all(&buf_length_array[..]).await?;
    stream.write_all(buf).await
}

/// receive a packet and return it (encrypted)
pub async fn receive_record(stream: &mut (impl Read + Unpin)) -> Result<Vec<u8>> {
    // 1. read 4 bytes from the stream. This represents the length of the encrypted packet.
    let mut length_arr: [u8; 4] = [0; 4];
    stream.read_exact(&mut length_arr[..]).await?;
    let mut length = u32::from_be_bytes(length_arr);
    trace!("encrypted packet length: {}", length);

    // 2. read that many bytes into an array (or a vector?)
    let enc_packet_length = length as usize;
    let mut enc_packet = Vec::with_capacity(enc_packet_length);
    let mut buf = [0u8; 1024];
    while length > 0 {
        let to_read = length.min(buf.len() as u32) as usize;
        stream
            .read_exact(&mut buf[..to_read])
            .await
            .context("cannot read from the tcp connection")?;
        enc_packet.append(&mut buf.to_vec());
        length -= to_read as u32;
    }

    enc_packet.truncate(enc_packet_length);
    trace!("length of the ciphertext: {:?}", enc_packet.len());

    Ok(enc_packet)
}

pub fn encrypt_record(plaintext: &[u8], nonce: secretbox::Nonce, key: &[u8]) -> Result<Vec<u8>> {
    let sodium_key = secretbox::Key::from_slice(&key).unwrap();
    // nonce in little endian (to interop with python client)
    let mut nonce_vec = nonce.as_ref().to_vec();
    nonce_vec.reverse();
    let nonce_le = secretbox::Nonce::from_slice(nonce_vec.as_ref())
        .ok_or_else(|| format_err!("encrypt_record: unable to create nonce"))?;

    let ciphertext = secretbox::seal(plaintext, &nonce_le, &sodium_key);
    let mut ciphertext_and_nonce = Vec::new();
    trace!("nonce: {:?}", nonce_vec);
    ciphertext_and_nonce.extend(nonce_vec);
    ciphertext_and_nonce.extend(ciphertext);

    Ok(ciphertext_and_nonce)
}

pub fn decrypt_record(enc_packet: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    // 3. decrypt the vector 'enc_packet' with the key.
    let (nonce, ciphertext) = enc_packet.split_at(sodiumoxide::crypto::secretbox::NONCEBYTES);

    assert_eq!(nonce.len(), sodiumoxide::crypto::secretbox::NONCEBYTES);
    let plaintext = secretbox::open(
        &ciphertext,
        &secretbox::Nonce::from_slice(nonce).context("nonce unwrap failed")?,
        &secretbox::Key::from_slice(&key).context("key unwrap failed")?,
    )
    .map_err(|()| format_err!("decryption failed"))?;

    trace!("decryption succeeded");
    Ok(plaintext)
}

fn make_relay_handshake(key: &Key<TransitKey>, tside: &str) -> String {
    let sub_key = key.derive_subkey_from_purpose::<crate::GenericKey>("transit_relay_token");
    format!(
        "please relay {} for side {}\n",
        hex::encode(&**sub_key),
        tside
    )
}

async fn rx_handshake_exchange(
    mut socket: TcpStream,
    host_type: HostType,
    key: &Key<TransitKey>,
) -> Result<Transit> {
    // create record keys
    let skey = key.derive_subkey_from_purpose("transit_record_sender_key");
    let rkey = key.derive_subkey_from_purpose("transit_record_receiver_key");

    // exchange handshake
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        trace!("initiating relay handshake");
        socket
            .write_all(make_relay_handshake(key, &tside).as_bytes())
            .await?;
        let mut rx = [0u8; 3];
        socket.read_exact(&mut rx).await?;
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
    }

    {
        // for receive mode, send receive_handshake_msg and compare.
        // the received message with send_handshake_msg

        socket
            .write_all(
                format!(
                    "transit receiver {} ready\n\n",
                    hex::encode(
                        &**key.derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver")
                    )
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
            hex::encode(&**key.derive_subkey_from_purpose::<crate::GenericKey>("transit_sender"))
        );
        ensure!(
            &rx[..] == expected_tx_handshake.as_bytes(),
            "handshake failed"
        );
    }

    Ok(Transit { socket, skey, rkey })
}

async fn tx_handshake_exchange(
    mut socket: TcpStream,
    host_type: HostType,
    key: &Key<TransitKey>,
) -> Result<Transit> {
    // 9. create record keys
    let skey = key.derive_subkey_from_purpose("transit_record_sender_key");
    let rkey = key.derive_subkey_from_purpose("transit_record_receiver_key");

    // 10. exchange handshake over tcp
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        socket
            .write_all(make_relay_handshake(key, &tside).as_bytes())
            .await?;
        let mut rx = [0u8; 3];
        socket.read_exact(&mut rx).await?;
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
    }

    {
        // for transmit mode, send send_handshake_msg and compare.
        // the received message with send_handshake_msg
        socket
            .write_all(
                format!(
                    "transit sender {} ready\n\n",
                    hex::encode(
                        &**key.derive_subkey_from_purpose::<crate::GenericKey>("transit_sender")
                    )
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
            hex::encode(&**key.derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver"))
        );
        ensure!(
            &rx[..] == expected_rx_handshake.as_bytes(),
            format_err!("handshake failed")
        );
    }

    Ok(Transit { socket, skey, rkey })
}

#[allow(clippy::type_complexity)]
fn get_direct_relay_hosts<'a, 'b: 'a>(
    ttype: &'b TransitType,
) -> (
    Vec<(HostType, &'a DirectHint)>,
    Vec<(HostType, &'a DirectHint)>,
) {
    let direct_hosts: Vec<(HostType, &DirectHint)> = ttype
        .hints_v1
        .iter()
        .filter(|hint| match hint {
            Hint::DirectTcpV1(_) => true,
            _ => false,
        })
        .map(|hint| match hint {
            Hint::DirectTcpV1(dt) => (HostType::Direct, dt),
            _ => unreachable!(),
        })
        .collect();
    let relay_hosts_list: Vec<&Vec<DirectHint>> = ttype
        .hints_v1
        .iter()
        .filter(|hint| match hint {
            Hint::RelayV1(_) => true,
            _ => false,
        })
        .map(|hint| match hint {
            Hint::RelayV1(rt) => &rt.hints,
            _ => unreachable!(),
        })
        .collect();

    let _hosts: Vec<(HostType, &DirectHint)> = Vec::new();
    let maybe_relay_hosts = relay_hosts_list.first();
    let relay_hosts: Vec<(HostType, &DirectHint)> = match maybe_relay_hosts {
        Some(relay_host_vec) => relay_host_vec
            .iter()
            .map(|host| (HostType::Relay, host))
            .collect(),
        None => vec![],
    };

    (direct_hosts, relay_hosts)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_transit() {
        let abilities = vec![Ability::DirectTcpV1, Ability::RelayV1];
        let hints = vec![
            Hint::new_direct(0.0, "192.168.1.8", 46295),
            Hint::new_relay(vec![DirectHint {
                priority: 2.0,
                hostname: "magic-wormhole-transit.debian.net".to_string(),
                port: 4001,
            }]),
        ];
        let t = crate::transfer::PeerMessage::new_transit(abilities, hints);
        assert_eq!(t.serialize(), "{\"transit\":{\"abilities-v1\":[{\"type\":\"direct-tcp-v1\"},{\"type\":\"relay-v1\"}],\"hints-v1\":[{\"hostname\":\"192.168.1.8\",\"port\":46295,\"priority\":0.0,\"type\":\"direct-tcp-v1\"},{\"hints\":[{\"hostname\":\"magic-wormhole-transit.debian.net\",\"port\":4001,\"priority\":2.0,\"type\":\"direct-tcp-v1\"}],\"type\":\"relay-v1\"}]}}")
    }
}
