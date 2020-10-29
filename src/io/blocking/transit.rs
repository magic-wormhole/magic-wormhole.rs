use futures::future::TryFutureExt;
use async_std::prelude::Future;
use std::sync::Arc;
use std::str::FromStr;
use crate::core::{
    TransitType,
    Hints,
    DirectType,
    Abilities,
    PeerMessage,
    TransitAck,
};
use std::str;
use std::net::{SocketAddr, ToSocketAddrs};
use async_std::net::{TcpListener, TcpStream};
use std::net::{IpAddr, Ipv4Addr};
use pnet::datalink;
use pnet::ipnetwork::IpNetwork;
use async_std::io::BufReader;
use async_std::io::Write;
use async_std::io::prelude::WriteExt;
use async_std::io::Read;
use async_std::io::ReadExt;
use log::*;
use sodiumoxide::crypto::secretbox;
use std::time::Duration;
use anyhow::{Result, Error, ensure, bail, format_err, Context};
use super::derive_key_from_purpose;
use super::Wormhole;
use futures::{Stream, StreamExt, Sink, SinkExt};

#[derive(Debug, PartialEq)]
enum HostType {
    Direct,
    Relay
}

pub struct RelayUrl {
    pub host: String,
    pub port: u16
}

impl FromStr for RelayUrl {
    type Err = &'static str;

    fn from_str(url: &str) -> Result<Self, &'static str> {
        // TODO use proper URL parsing
        let v: Vec<&str> = url.split(':').collect();
        if v.len() == 3 && v[0] == "tcp" {
            v[2].parse()
                .map(|port| RelayUrl{ host: v[1].to_string(), port})
                .map_err(|_| "Cannot parse relay url port")
        } else {
            Err("Incorrect relay server url format")
        }
    }
}

pub struct Transit {
    pub socket: TcpStream,
    pub skey: Vec<u8>,
    pub rkey: Vec<u8>,
}

impl Transit {
    pub async fn sender_connect<
        'a,
        'b: 'a,
        T,
        C1,
        F1,
    >(
        wormhole: &'b mut Wormhole,
        relay_url: &RelayUrl,
        post_handshake: C1,
    ) -> Result<(Self, T)> 
        where 
        C1: FnOnce(&'a mut Wormhole) -> F1 + 'a,
        F1: Future<Output = Result<T>>,
    {
        let transit_key = Arc::new(wormhole.key.derive_transit_key());
        debug!("transit key {}", hex::encode(&*transit_key));

        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("[::]:0").await?;
        let listen_socket = listener.local_addr()?;

        // 2. send transit message to peer
        let direct_hints: Vec<Hints> = build_direct_hints(listen_socket.port());
        let relay_hints: Vec<Hints> = build_relay_hints(relay_url);

        let mut abilities = Vec::new();
        abilities.push(Abilities{ttype: "direct-tcp-v1".to_string()});
        abilities.push(Abilities{ttype: "relay-v1".to_string()});

        // combine direct hints and relay hints
        let mut our_hints: Vec<Hints> = Vec::new();
        for hint in direct_hints {
            our_hints.push(hint);
        }
        for hint in relay_hints {
            our_hints.push(hint);
        }

        // send the transit message
        let transit_msg = PeerMessage::new_transit(abilities, our_hints).serialize();
        debug!("transit_msg: {:?}", transit_msg);
        let _todo = wormhole.tx.send(transit_msg.as_bytes().to_vec()).await;

        // 5. receive transit message from peer.
        let msg = wormhole.rx.next().await.unwrap();
        let maybe_transit = PeerMessage::deserialize(str::from_utf8(&msg)?);
        debug!("received transit message: {:?}", maybe_transit);

        let ttype = match maybe_transit {
            PeerMessage::Transit(tmsg) => tmsg,
            _ => bail!(format_err!("unexpected message: {:?}", maybe_transit)),
        };
        // TODO remove this one day
        let ttype = &*Box::leak(Box::new(ttype));
        
        let post_handshake_result = post_handshake(wormhole).await?;

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        // extract peer's ip/hostname from 'ttype'
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
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
                let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
                let direct_host = direct_host_iter.next().unwrap();

                debug!("peer host: {}", direct_host);

                TcpStream::connect(direct_host)
                    .err_into::<Error>()
                    .and_then(|socket| {
                        tx_handshake_exchange(socket, host.0, &*transit_key)
                    })
                    .await
            });//);
            handshake_futures.push(future);
        }
        handshake_futures.push(async_std::task::spawn(async move {
            let port = listen_socket.port();
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
                }
                (Err(e), _index, remaining) => {
                    debug!("Some handshake failed {:#}", e);
                    handshake_futures = remaining;
                }
            }
        }
        let mut transit = transit;

        /* Cancel all remaining non-finished handshakes */
        handshake_futures.into_iter()
            .map(async_std::task::JoinHandle::cancel)
            .for_each(std::mem::drop);

        debug!("Sending 'go' message to {}", transit.socket.peer_addr().unwrap());
        send_buffer(&mut transit.socket, b"go\n").await?;

        Ok((transit, post_handshake_result))
    }

    pub async fn receiver_connect<
        'a,
        'b: 'a,
        T,
        C1,
        F1,
    >(
        wormhole: &'b mut Wormhole,
        relay_url: &RelayUrl,
        ttype: TransitType,
        post_handshake: C1,
    ) -> Result<(Self, T)> 
        where 
        C1: FnOnce(&'a mut Wormhole) -> F1 + 'a,
        F1: Future<Output = Result<T>>,
    {
        let ttype = &*Box::leak(Box::new(ttype)); // TODO remove this one day
        let transit_key = Arc::new(wormhole.key.derive_transit_key());
        debug!("transit key {}", hex::encode(&*transit_key));

        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("[::]:0").await?;
        let listen_socket = listener.local_addr()?;
        let port = listen_socket.port();

        // 2. send transit message to peer
        let direct_hints: Vec<Hints> = build_direct_hints(port);
        let relay_hints: Vec<Hints> = build_relay_hints(relay_url);

        let mut abilities = Vec::new();
        abilities.push(Abilities{ttype: "direct-tcp-v1".to_string()});
        abilities.push(Abilities{ttype: "relay-v1".to_string()});

        // combine direct hints and relay hints
        let mut our_hints: Vec<Hints> = Vec::new();
        for hint in direct_hints {
            our_hints.push(hint);
        }
        for hint in relay_hints {
            our_hints.push(hint);
        }

        // send the transit message
        let transit_msg = PeerMessage::new_transit(abilities, our_hints).serialize();
        debug!("Sending '{}'", &transit_msg);
        let _todo = wormhole.tx.send(transit_msg.as_bytes().to_vec()).await;
        
        let post_handshake_result = post_handshake(wormhole).await?;

        // 4. listen for connections on the port and simultaneously try connecting to the
        //    peer listening port.
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
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
                let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
                let direct_host = direct_host_iter.next().unwrap();

                debug!("peer host: {}", direct_host);

                TcpStream::connect(direct_host)
                    .err_into::<Error>()
                    .and_then(|socket| {
                        rx_handshake_exchange(socket, host.0, &*transit_key)
                    })
                    .await
            });//);
            handshake_futures.push(future);
        }
        handshake_futures.push(async_std::task::spawn(async move {
            let port = listen_socket.port();
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
                }
                (Err(e), _index, remaining) => {
                    debug!("Some handshake failed {:#}", e);
                    handshake_futures = remaining;
                }
            }
        }

        /* Cancel all remaining non-finished handshakes */
        handshake_futures.into_iter()
            .map(async_std::task::JoinHandle::cancel)
            .for_each(std::mem::drop);

        Ok((transit, post_handshake_result))
    }
}

pub fn make_transit_ack_msg(sha256: &str, key: &[u8]) -> Result<Vec<u8>> {
    let plaintext = TransitAck::new("ok", sha256).serialize();

    let nonce_slice: [u8; sodiumoxide::crypto::secretbox::NONCEBYTES]
        = [0; sodiumoxide::crypto::secretbox::NONCEBYTES];
    let nonce = secretbox::Nonce::from_slice(&nonce_slice[..]).unwrap();

    encrypt_record(&plaintext.as_bytes(), nonce, &key)
}

fn generate_transit_side() -> String {
    let x: [u8; 8] = rand::random();
    hex::encode(x)
}

fn make_record_keys(key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let s_purpose = "transit_record_sender_key";
    let r_purpose = "transit_record_receiver_key";

    let sender = derive_key_from_purpose(key, s_purpose);
    let receiver = derive_key_from_purpose(key, r_purpose);

    (sender, receiver)
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
        stream.read_exact(&mut buf[..to_read]).await.context("cannot read from the tcp connection")?;
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
    let (nonce, ciphertext) =
        enc_packet.split_at(sodiumoxide::crypto::secretbox::NONCEBYTES);

    assert_eq!(nonce.len(), sodiumoxide::crypto::secretbox::NONCEBYTES);
    let plaintext = secretbox::open(
        &ciphertext,
        &secretbox::Nonce::from_slice(nonce).context("nonce unwrap failed")?,
        &secretbox::Key::from_slice(&key).context("key unwrap failed")?,
    ).map_err(|()| format_err!("decryption failed"))?;

    trace!("decryption succeeded");
    Ok(plaintext)
}

fn make_receive_handshake(key: &[u8]) -> String {
    let purpose = "transit_receiver";
    let sub_key = derive_key_from_purpose(key, purpose);

    let msg = format!("transit receiver {} ready\n\n", hex::encode(sub_key));
    msg
}

fn make_send_handshake(key: &[u8]) -> String {
    let purpose = "transit_sender";
    let sub_key = derive_key_from_purpose(key, purpose);

    let msg = format!("transit sender {} ready\n\n", hex::encode(sub_key));
    msg
}

fn make_relay_handshake(key: &[u8], tside: &str) -> String {
    let purpose = "transit_relay_token";
    let sub_key = derive_key_from_purpose(key, purpose);
    let msg = format!("please relay {} for side {}\n", hex::encode(sub_key), tside);
    trace!("relay handshake message: {}", msg);
    msg
}

async fn rx_handshake_exchange(mut socket: TcpStream, host_type: HostType, key: impl AsRef<[u8]>) -> Result<Transit> {
    // create record keys
    let (skey, rkey) = make_record_keys(key.as_ref());

    // exchange handshake
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        trace!("initiating relay handshake");
        let relay_handshake = make_relay_handshake(key.as_ref(), &tside);
        let relay_handshake_msg = relay_handshake.as_bytes();
    
        send_buffer(&mut socket, relay_handshake_msg).await?;
    
        let mut rx = [0u8; 3];
        recv_buffer(&mut socket, &mut rx).await?;
    
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
        trace!("relay handshake succeeded");
    }

    {
        // send handshake and receive handshake
        let send_handshake_msg = make_send_handshake(key.as_ref());
        let rx_handshake = make_receive_handshake(key.as_ref());
        dbg!(&rx_handshake, rx_handshake.as_bytes().len());
        let receive_handshake_msg = rx_handshake.as_bytes();

        // for receive mode, send receive_handshake_msg and compare.
        // the received message with send_handshake_msg

        send_buffer(&mut socket, receive_handshake_msg).await?;

        trace!("quarter's done");

        let mut rx: [u8; 90] = [0; 90];
        recv_buffer(&mut socket, &mut rx[0..87]).await?;

        trace!("half's done");

        recv_buffer(&mut socket, &mut rx[87..90]).await?;

        // The received message "transit receiver $hash ready\n\n" has exactly 87 bytes
        // Three bytes for the "go\n" ack
        // TODO do proper line parsing one day, this is atrocious

        let mut s_handshake = send_handshake_msg.as_bytes().to_vec();
        let go_msg = b"go\n";
        s_handshake.extend_from_slice(go_msg);
        ensure!(s_handshake == &rx[..], "handshake failed");
    }

    trace!("handshake successful");

    Ok(Transit { socket, skey, rkey })
}

async fn tx_handshake_exchange(mut socket: TcpStream, host_type: HostType, key: impl AsRef<[u8]>) -> Result<Transit> {
    // 9. create record keys
    let (skey, rkey) = make_record_keys(key.as_ref());

    // 10. exchange handshake over tcp
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        trace!("initiating relay handshake");
        let relay_handshake = make_relay_handshake(key.as_ref(), &tside);
        let relay_handshake_msg = relay_handshake.as_bytes();
    
        send_buffer(&mut socket, relay_handshake_msg).await?;
    
        let mut rx = [0u8; 3];
        recv_buffer(&mut socket, &mut rx).await?;
    
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
        trace!("relay handshake succeeded");
    }

    {
        // send handshake and receive handshake
        let tx_handshake = make_send_handshake(key.as_ref());
        let rx_handshake = make_receive_handshake(key.as_ref());
        dbg!(&tx_handshake, tx_handshake.as_bytes().len());

        debug!("tx handshake started");

        let tx_handshake_msg = tx_handshake.as_bytes();
        let rx_handshake_msg = rx_handshake.as_bytes();
            
        // for transmit mode, send send_handshake_msg and compare.
        // the received message with send_handshake_msg
        send_buffer(&mut socket, tx_handshake_msg).await?;

        trace!("half's done");

        // The received message "transit sender $hash ready\n\n" has exactly 89 bytes
        // TODO do proper line parsing one day, this is atrocious
        let mut rx: [u8; 89] = [0; 89];
        recv_buffer(&mut socket, &mut rx).await?;

        trace!("{:?}", rx_handshake_msg.len());

        let r_handshake = rx_handshake_msg;
        ensure!(r_handshake == &rx[..], format_err!("handshake failed"));
    }

    trace!("handshake successful");

    Ok(Transit { socket, skey, rkey } )
}

fn build_direct_hints(port: u16) -> Vec<Hints> {
    let hints = datalink::interfaces().iter()
        .filter(|iface| !datalink::NetworkInterface::is_loopback(iface))
        .flat_map(|iface| iface.ips.iter())
        .map(|n| n as &IpNetwork)
        // .filter(|ip| ip.is_ipv4()) // TODO why was that there can we remove it?
        .map(|ip| Hints::DirectTcpV1(DirectType{ priority: 0.0, hostname: ip.ip().to_string(), port}))
        .collect::<Vec<_>>();
    dbg!(&hints);

    hints
}

fn build_relay_hints(relay_url: &RelayUrl) -> Vec<Hints> {
    let mut hints = Vec::new();
    hints.push(Hints::new_relay(vec![DirectType {
        priority: 0.0, 
        hostname: relay_url.host.clone(), 
        port: relay_url.port
    }]));

    hints
}

#[allow(clippy::type_complexity)]
fn get_direct_relay_hosts<'a, 'b: 'a>(ttype: &'b TransitType) -> (Vec<(HostType, &'a DirectType)>, Vec<(HostType, &'a DirectType)>) {
    let direct_hosts: Vec<(HostType, &DirectType)> = ttype.hints_v1.iter()
        .filter(|hint|
                match hint {
                    Hints::DirectTcpV1(_) => true,
                    _ => false,
                })
        .map(|hint|
             match hint {
                 Hints::DirectTcpV1(dt) => (HostType::Direct, dt),
                 _ => unreachable!(),
             })
        .collect();
    let relay_hosts_list: Vec<&Vec<DirectType>> = ttype.hints_v1.iter()
        .filter(|hint|
                match hint {
                    Hints::RelayV1(_) => true,
                    _ => false,
                })
        .map(|hint|
             match hint {
                 Hints::RelayV1(rt) => &rt.hints,
                 _ => unreachable!(),
             })
        .collect();

    let _hosts: Vec<(HostType, &DirectType)> = Vec::new();
    let maybe_relay_hosts = relay_hosts_list.first();
    let relay_hosts: Vec<(HostType, &DirectType)> = match maybe_relay_hosts {
        Some(relay_host_vec) => relay_host_vec.iter()
            .map(|host| (HostType::Relay, host))
            .collect(),
        None => vec![],
    };

    (direct_hosts, relay_hosts)
}

async fn send_buffer(stream: &mut TcpStream, buf: &[u8]) -> std::io::Result<()> {
    stream.write_all(buf).await
}

async fn recv_buffer(stream: &mut TcpStream, buf: &mut [u8]) -> std::io::Result<()> {
    stream.read_exact(buf).await
}
