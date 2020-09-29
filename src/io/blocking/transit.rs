use std::sync::atomic::AtomicBool;
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
use std::net::{TcpListener, TcpStream};
use std::net::{IpAddr, Ipv4Addr};
use pnet::datalink;
use pnet::ipnetwork::IpNetwork;
use std::io;
use std::io::BufReader;
use std::io::Write;
use std::io::Read;
use log::*;
use sodiumoxide::crypto::secretbox;
use std::time::Duration;
use anyhow::{Result, Error, ensure, bail, format_err, Context};
use super::derive_key_from_purpose;
use super::Wormhole;

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
    pub fn sender_connect<T>(
        w: &mut Wormhole,
        relay_url: &RelayUrl,
        appid: &str,
        post_handshake: impl FnOnce(&mut Wormhole) -> Result<T>,
        hacky_callback: impl FnOnce(&mut Transit, &T) -> Result<()>,
    ) -> Result<(Self, T)> {
        let transit_key = w.derive_transit_key(appid);
        debug!("transit key {}", hex::encode(&transit_key));

        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("[::]:0")?;
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
        w.send_message(transit_msg.as_bytes());

        // 5. receive transit message from peer.
        let msg = w.get_message();
        let maybe_transit = PeerMessage::deserialize(str::from_utf8(&msg)?);
        debug!("received transit message: {:?}", maybe_transit);

        let ttype = match maybe_transit {
            PeerMessage::Transit(tmsg) => tmsg,
            _ => bail!(format_err!("unexpected message: {:?}", maybe_transit)),
        };
        
        let handshake_result = post_handshake(w)?;

        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        // extract peer's ip/hostname from 'ttype'
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);

        // TODO: combine our relay hints with the peer's relay hints.

        let (successful_connections_tx, successful_connections_rx) = std::sync::mpsc::channel::<Transit>();
        let handshake_succeeded = Arc::new(AtomicBool::new(false));
        crossbeam_utils::thread::scope(|scope| {
            let transit_key = &transit_key; // Borrow instead of move
            for host in hosts {
                let successful_connections_tx = successful_connections_tx.clone();
                scope.spawn(move |_| {
                    debug!("host: {:?}", host);
                    let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
                    let direct_host = direct_host_iter.next().unwrap();

                    debug!("peer host: {}", direct_host);

                    // TODO wtf
                    match connect_or_accept(direct_host) {
                        Ok((mut socket, _addr)) => {
                            debug!("connected to {:?}", direct_host);
                            match tx_handshake_exchange(&mut socket, host.0, &transit_key) {
                                Ok((skey, rkey)) => {
                                    debug!("Handshake with {} succeeded", direct_host);
                                    successful_connections_tx.send(Transit {socket, skey, rkey}).unwrap();
                                },
                                Err(e) => {
                                    debug!("Handshake with {} failed :(, {}", direct_host, e);
                                }
                            }
                        },
                        Err(_) => {
                            trace!("could not connect to {:?}", direct_host);
                        },
                    }
                });
            }
            {
                let handshake_succeeded = handshake_succeeded.clone();
                scope.spawn(move |_| {
                    let port = listen_socket.port();
                    while !handshake_succeeded.load(std::sync::atomic::Ordering::Acquire) {
                        match listener.accept() {
                            Ok((mut socket, _addr)) => {
                                debug!("connected to localhost:{}", port);
                                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                                socket.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                                match tx_handshake_exchange(&mut socket, HostType::Direct, &transit_key) {
                                    Ok((skey, rkey)) => {
                                        successful_connections_tx.send(Transit {socket, skey, rkey}).unwrap();
                                    },
                                    Err(e) => {
                                        debug!("Handshake with localhost:{} failed :(, {}", port, e);
                                    }
                                }
                            },
                            Err(_) => {
                                debug!("could not connect to local host");
                            },
                        }
                    }
                    debug!("quit send loop");
                });
            }

            let mut transit: Transit = successful_connections_rx.recv()
                .context("Could not establish a connection to the other side")?;
            handshake_succeeded.store(true, std::sync::atomic::Ordering::Release);
    
            debug!("Sending 'go' message to {}", transit.socket.peer_addr().unwrap());
            send_buffer(&mut transit.socket, b"go\n")?;

            hacky_callback(&mut transit, &handshake_result)?;

            Ok((transit, handshake_result))
        }).unwrap()
    }

    pub fn receiver_connect<T>(
        w: &mut Wormhole,
        relay_url: &RelayUrl,
        appid: &str,
        ttype: TransitType,
        post_handshake: impl FnOnce(&mut Wormhole) -> Result<T>,
        hacky_callback: impl FnOnce(&mut Transit, &T) -> Result<()>,
    ) -> Result<(Self, T)> {
        let transit_key = w.derive_transit_key(appid);
        debug!("transit key {}", hex::encode(&transit_key));

        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("[::]:0")?;
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
        w.send_message(transit_msg.as_bytes());
        
        let handshake_result = post_handshake(w)?;

        // 4. listen for connections on the port and simultaneously try connecting to the
        //    peer listening port.
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);

        // TODO: combine our relay hints with the peer's relay hints.

        let (successful_connections_tx, successful_connections_rx) = std::sync::mpsc::channel();
        let handshake_succeeded = Arc::new(AtomicBool::new(false));

        crossbeam_utils::thread::scope(|scope| {
            let transit_key = &transit_key; // Borrow instead of move
            for host in hosts {
                let successful_connections_tx: std::sync::mpsc::Sender<_> = successful_connections_tx.clone();
                scope.spawn(move |_| {
                    debug!("host: {:?}", host);
                    let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
                    let direct_host = direct_host_iter.next().unwrap();

                    debug!("peer host: {}", direct_host);

                    match connect_or_accept(direct_host) {
                        Ok((mut socket, _addr)) => {
                            debug!("connected to {:?}", direct_host);
                            match rx_handshake_exchange(&mut socket, host.0, &transit_key) {
                                Ok((skey, rkey)) => {
                                    debug!("Handshake with {} succeeded", direct_host);
                                    successful_connections_tx.send(Transit {socket, skey, rkey}).unwrap();
                                },
                                Err(e) => {
                                    debug!("Handshake with {} failed :(, {}", direct_host, e);
                                }
                            }
                        },
                        Err(_) => {
                            debug!("could not connect to {:?}", direct_host);
                        },
                    }
                });
            }
            {
                let handshake_succeeded = handshake_succeeded.clone();
                scope.spawn(move |_| {
                    let port = listen_socket.port();
                    debug!("local host {}", port);

                    while !handshake_succeeded.load(std::sync::atomic::Ordering::Acquire) {
                        match listener.accept() {
                            Ok((mut socket, _addr)) => {
                                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                                socket.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                                debug!("connected to localhost:{}", port);
                                match rx_handshake_exchange(&mut socket, HostType::Direct, &transit_key) {
                                    Ok((skey, rkey)) => {
                                        successful_connections_tx.send(Transit {socket, skey, rkey}).unwrap();
                                    },
                                    Err(e) => {
                                        debug!("Handshake with localhost:{} failed :(, {}", port, e);
                                    }
                                }
                            },
                            Err(_) => {
                                debug!("could not connect to local host");
                            },
                        }
                    }
                    debug!("quit receive loop");
                });
            }

            let mut transit = successful_connections_rx.recv().context("Could not establish a connection to the other side")?;
            handshake_succeeded.store(true, std::sync::atomic::Ordering::Release);
            
            hacky_callback(&mut transit, &handshake_result)?;

            Ok((transit, handshake_result))
        })
        .map_err(|err| *err.downcast::<Error>().expect("Please only return 'anyhow::Error' in this code block"))?
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

// TODO cleanup
fn connect_or_accept(addr: SocketAddr) -> Result<(TcpStream, SocketAddr), std::io::Error> {
    // let listen_socket = thread::spawn(move || {
    //     listener.accept()
    // });
    
    // let connect_socket = thread::spawn(move || {
        let five_seconds = Duration::new(5, 0);
        let tcp_stream = TcpStream::connect_timeout(&addr, five_seconds);
        match tcp_stream {
            Ok(stream) => {
                stream.set_read_timeout(Some(five_seconds))?;
                stream.set_write_timeout(Some(five_seconds))?;
                Ok((stream, addr))
            },
            Err(e) => Err(e)
        }
    // });

    // connect_socket.join().unwrap()
}

fn make_record_keys(key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let s_purpose = "transit_record_sender_key";
    let r_purpose = "transit_record_receiver_key";

    let sender = derive_key_from_purpose(key, s_purpose);
    let receiver = derive_key_from_purpose(key, r_purpose);

    (sender, receiver)
}

pub fn send_record(stream: &mut TcpStream, buf: &[u8]) -> io::Result<()> {
    let buf_length: u32 = buf.len() as u32;
    trace!("record size: {:?}", buf_length);
    let buf_length_array: [u8; 4] = buf_length.to_be_bytes();
    stream.write_all(&buf_length_array[..])?;
    stream.write_all(buf)
}

/// receive a packet and return it (encrypted)
pub fn receive_record<T: Read>(stream: &mut BufReader<T>) -> Result<Vec<u8>> {
    // 1. read 4 bytes from the stream. This represents the length of the encrypted packet.
    let mut length_arr: [u8; 4] = [0; 4];
    stream.read_exact(&mut length_arr[..])?;
    let mut length = u32::from_be_bytes(length_arr);
    trace!("encrypted packet length: {}", length);

    // 2. read that many bytes into an array (or a vector?)
    let enc_packet_length = length as usize;
    let mut enc_packet = Vec::with_capacity(enc_packet_length);
    let mut buf = [0u8; 1024];
    while length > 0 {
        let to_read = length.min(buf.len() as u32) as usize;
        stream.read_exact(&mut buf[..to_read]).context("cannot read from the tcp connection")?;
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

fn rx_handshake_exchange(socket: &mut TcpStream, host_type: HostType, key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // create record keys
    let (skey, rkey) = make_record_keys(key);

    // exchange handshake
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        trace!("initiating relay handshake");
        let relay_handshake = make_relay_handshake(key, &tside);
        let relay_handshake_msg = relay_handshake.as_bytes();
    
        send_buffer(socket, relay_handshake_msg)?;
    
        let mut rx = [0u8; 3];
        recv_buffer(socket, &mut rx)?;
    
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
        trace!("relay handshake succeeded");
    }

    {
        // send handshake and receive handshake
        let send_handshake_msg = make_send_handshake(key);
        let rx_handshake = make_receive_handshake(key);
        dbg!(&rx_handshake, rx_handshake.as_bytes().len());
        let receive_handshake_msg = rx_handshake.as_bytes();

        // for receive mode, send receive_handshake_msg and compare.
        // the received message with send_handshake_msg

        send_buffer(socket, receive_handshake_msg)?;

        trace!("quarter's done");

        let mut rx: [u8; 90] = [0; 90];
        recv_buffer(socket, &mut rx[0..87])?;

        trace!("half's done");

        recv_buffer(socket, &mut rx[87..90])?;

        // The received message "transit receiver $hash ready\n\n" has exactly 87 bytes
        // Three bytes for the "go\n" ack
        // TODO do proper line parsing one day, this is atrocious

        let mut s_handshake = send_handshake_msg.as_bytes().to_vec();
        let go_msg = b"go\n";
        s_handshake.extend_from_slice(go_msg);
        ensure!(s_handshake == &rx[..], "handshake failed");
    }

    trace!("handshake successful");

    Ok((skey, rkey))
}

fn tx_handshake_exchange(socket: &mut TcpStream, host_type: HostType, key: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // 9. create record keys
    let (skey, rkey) = make_record_keys(key);

    // 10. exchange handshake over tcp
    let tside = generate_transit_side();

    if host_type == HostType::Relay {
        trace!("initiating relay handshake");
        let relay_handshake = make_relay_handshake(key, &tside);
        let relay_handshake_msg = relay_handshake.as_bytes();
    
        send_buffer(socket, relay_handshake_msg)?;
    
        let mut rx = [0u8; 3];
        recv_buffer(socket, &mut rx)?;
    
        let ok_msg: [u8; 3] = *b"ok\n";
        ensure!(ok_msg == rx, format_err!("relay handshake failed"));
        trace!("relay handshake succeeded");
    }

    {
        // send handshake and receive handshake
        let tx_handshake = make_send_handshake(key);
        let rx_handshake = make_receive_handshake(key);
        dbg!(&tx_handshake, tx_handshake.as_bytes().len());

        debug!("tx handshake started");

        let tx_handshake_msg = tx_handshake.as_bytes();
        let rx_handshake_msg = rx_handshake.as_bytes();
            
        // for transmit mode, send send_handshake_msg and compare.
        // the received message with send_handshake_msg
        send_buffer(socket, tx_handshake_msg)?;

        trace!("half's done");

        // The received message "transit sender $hash ready\n\n" has exactly 89 bytes
        // TODO do proper line parsing one day, this is atrocious
        let mut rx: [u8; 89] = [0; 89];
        recv_buffer(socket, &mut rx)?;

        trace!("{:?}", rx_handshake_msg.len());

        let r_handshake = rx_handshake_msg;
        ensure!(r_handshake == &rx[..], format_err!("handshake failed"));
    }

    trace!("handshake successful");

    Ok((skey, rkey))
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
fn get_direct_relay_hosts(ttype: &TransitType) -> (Vec<(HostType, &DirectType)>, Vec<(HostType, &DirectType)>) {
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

fn send_buffer(stream: &mut TcpStream, buf: &[u8]) -> io::Result<()> {
    stream.write_all(buf)
}

fn recv_buffer(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<()> {
    stream.read_exact(buf)
}
