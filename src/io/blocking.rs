use std::str::FromStr;
use crate::core::WormholeCore;
use crate::core::{
    APIAction, APIEvent, Action, Code, IOAction, IOEvent, Mood, TimerHandle,
    WSHandle,
    message,
    message_ack,
    TransitType,
    Hints,
    DirectType,
    direct_type,
    relay_type,
    Abilities,
    transit,
    file_ack,
    PeerMessage,
    AnswerType,
    TransitAck,
    transit_ack,
    offer_file,
    OfferType,
};
use crate::core::key::derive_key;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time;
use url::Url;
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
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::secretbox;
use std::time::Duration;
use std::path::Path;
use std::fs::File;

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

#[derive(Debug, PartialEq)]
pub enum MessageType {
    Message(String),
    File {
        filename: String,
        filesize: u64,
    }
}

enum ToCore {
    API(APIEvent),
    #[allow(dead_code)]
    IO(IOEvent),
    TimerExpired(TimerHandle),
    WebSocketConnectionMade(WSHandle),
    WebSocketMessageReceived(WSHandle, String),
    WebSocketConnectionLost(WSHandle),
}

#[allow(dead_code)]
enum XXXFromCore {
    API(APIAction),
    IO(IOAction),
}

#[derive(Debug, PartialEq)]
enum HostType {
    Direct,
    Relay
}

enum WSControl {
    Data(String),
    Close,
}

struct CoreWrapper {
    core: WormholeCore,

    tx_to_core: Sender<ToCore>, // give clones to websocket/timer threads
    rx_by_core: Receiver<ToCore>,

    timers: HashSet<TimerHandle>,
    websockets: HashMap<WSHandle, Sender<WSControl>>,

    tx_welcome_to_app: Sender<Value>,
    tx_messages_to_app: Sender<Vec<u8>>,
    tx_key_to_transit: Sender<Vec<u8>>,
    tx_code_to_app: Sender<String>,
    tx_verifier_to_app: Sender<Vec<u8>>,
    tx_versions_to_app: Sender<Value>,
    tx_close_to_app: Sender<Mood>,
}

struct WSConnection {
    handle: WSHandle,
    tx: Sender<ToCore>,
}

impl ws::Handler for WSConnection {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        // now that the outbound side is prepared to send messages, notify
        // the Core
        self.tx
            .send(ToCore::WebSocketConnectionMade(self.handle))
            .unwrap();
        Ok(())
    }

    fn on_message(&mut self, msg: ws::Message) -> Result<(), ws::Error> {
        let s = msg.into_text().unwrap();
        self.tx
            .send(ToCore::WebSocketMessageReceived(self.handle, s))
            .unwrap();
        Ok(())
    }

    fn on_close(&mut self, _code: ws::CloseCode, _reason: &str) {
        self.tx
            .send(ToCore::WebSocketConnectionLost(self.handle))
            .unwrap();
    }
}

fn ws_outbound(ws_rx: &Receiver<WSControl>, out: &ws::Sender) {
    while let Ok(c) = ws_rx.recv() {
        match c {
            WSControl::Data(d) => out.send(ws::Message::Text(d)).unwrap(),
            WSControl::Close => out.close(ws::CloseCode::Normal).unwrap(),
        }
    }
}

struct WSFactory {
    handle: WSHandle,
    tx: Option<Sender<ToCore>>,
    ws_rx: Option<Receiver<WSControl>>,
}

impl ws::Factory for WSFactory {
    type Handler = WSConnection;
    fn connection_made(&mut self, out: ws::Sender) -> WSConnection {
        use std::mem;
        let ws_rx = mem::replace(&mut self.ws_rx, None).unwrap();
        let tx = mem::replace(&mut self.tx, None).unwrap();
        thread::spawn(move || ws_outbound(&ws_rx, &out));
        WSConnection {
            handle: self.handle,
            tx,
        }
    }
}

fn ws_connector(
    url: &str,
    handle: WSHandle,
    tx: Sender<ToCore>,
    ws_rx: Receiver<WSControl>,
) {
    // we're called in a new thread created just for this connection
    let f = WSFactory {
        handle,
        tx: Some(tx),
        ws_rx: Some(ws_rx),
    };
    let b = ws::Builder::new();
    let mut w1 = b.build(f).unwrap();
    w1.connect(Url::parse(url).unwrap()).unwrap();
    w1.run().unwrap(); // blocks forever
}

impl CoreWrapper {
    fn run(&mut self) {
        loop {
            let actions = match self.rx_by_core.recv().unwrap() {
                ToCore::API(a) => self.core.do_api(a),
                ToCore::IO(i) => self.core.do_io(i),
                ToCore::TimerExpired(handle) => {
                    if self.timers.contains(&handle) {
                        self.core.do_io(IOEvent::TimerExpired(handle))
                    } else {
                        vec![]
                    }
                }
                ToCore::WebSocketConnectionMade(handle) => {
                    self.core.do_io(IOEvent::WebSocketConnectionMade(handle))
                }
                ToCore::WebSocketMessageReceived(handle, msg) => self
                    .core
                    .do_io(IOEvent::WebSocketMessageReceived(handle, msg)),
                ToCore::WebSocketConnectionLost(handle) => {
                    self.core.do_io(IOEvent::WebSocketConnectionLost(handle))
                }
            };
            for action in actions {
                self.process_action(action);
            }
        }
    }

    fn process_action(&mut self, action: Action) {
        match action {
            Action::API(a) => self.process_api_action(a),
            Action::IO(i) => self.process_io_action(i),
        }
    }

    fn process_api_action(&mut self, action: APIAction) {
        use self::APIAction::*;
        match action {
            GotWelcome(w) => self.tx_welcome_to_app.send(w).unwrap(),
            GotMessage(m) => self.tx_messages_to_app.send(m).unwrap(),
            GotCode(c) => self.tx_code_to_app.send(c.to_string()).unwrap(),
            GotUnverifiedKey(k) => self.tx_key_to_transit.send(k.to_vec()).unwrap(),
            GotVerifier(v) => self.tx_verifier_to_app.send(v).unwrap(),
            GotVersions(v) => self.tx_versions_to_app.send(v).unwrap(),
            GotClosed(mood) => self.tx_close_to_app.send(mood).unwrap(),
        }
    }

    fn process_io_action(&mut self, action: IOAction) {
        use self::IOAction::*;
        match action {
            StartTimer(handle, duration) => {
                let tx = self.tx_to_core.clone();
                self.timers.insert(handle);
                thread::spawn(move || {
                    // ugh, why can't this just take a float? ok ok,
                    // Nan, negatives, fine fine
                    let dur_ms = (duration * 1000.0) as u64;
                    let dur = time::Duration::from_millis(dur_ms);
                    thread::sleep(dur);
                    tx.send(ToCore::TimerExpired(handle)).unwrap();
                });
            }
            CancelTimer(handle) => {
                self.timers.remove(&handle);
            }
            WebSocketOpen(handle, url) => self.websocket_open(handle, url),
            WebSocketSendMessage(handle, msg) => {
                self.websocket_send(handle, msg)
            }
            WebSocketClose(handle) => self.websocket_close(handle),
        }
    }

    fn websocket_open(&mut self, handle: WSHandle, url: String) {
        let tx = self.tx_to_core.clone();
        let (ws_tx, ws_rx) = channel();
        self.websockets.insert(handle, ws_tx);
        thread::spawn(move || ws_connector(&url, handle, tx, ws_rx));
    }

    fn websocket_send(&self, handle: WSHandle, msg: String) {
        self.websockets[&handle].send(WSControl::Data(msg)).unwrap();
    }

    fn websocket_close(&mut self, handle: WSHandle) {
        self.websockets[&handle].send(WSControl::Close).unwrap();
        self.websockets.remove(&handle);
    }
}

// we have one channel per API pathway
pub struct Wormhole {
    tx_event_to_core: Sender<ToCore>,

    rx_welcome_from_core: Receiver<Value>,
    rx_messages_from_core: Receiver<Vec<u8>>,
    rx_key_from_transit: Receiver<Vec<u8>>,
    rx_code_from_core: Receiver<String>,
    rx_verifier_from_core: Receiver<Vec<u8>>,
    rx_versions_from_core: Receiver<Value>,
    rx_close_from_core: Receiver<Mood>,

    code: Option<String>,
    key: Option<Vec<u8>>,
    welcome: Option<Value>,
    versions: Option<Value>,
    verifier: Option<Vec<u8>>,
}

impl Wormhole {
    pub fn new(appid: &str, relay_url: &str) -> Wormhole {
        // the Wormhole object lives in the same thread as the application,
        // and it blocks. We put the core in a separate thread, and use a
        // channel to talk to it.
        let (tx_event_to_core, rx_by_core) = channel();
        // the inbound messages get their own channel
        let (tx_messages_to_app, rx_messages_from_core) = channel();
        let (tx_welcome_to_app, rx_welcome_from_core) = channel();
        let (tx_key_to_transit, rx_key_from_transit) = channel();
        let (tx_code_to_app, rx_code_from_core) = channel();
        let (tx_verifier_to_app, rx_verifier_from_core) = channel();
        let (tx_versions_to_app, rx_versions_from_core) = channel();
        let (tx_close_to_app, rx_close_from_core) = channel();

        let mut cw = CoreWrapper {
            core: WormholeCore::new(appid, relay_url),
            tx_to_core: tx_event_to_core.clone(),
            rx_by_core,
            timers: HashSet::new(),
            websockets: HashMap::new(),
            tx_welcome_to_app,
            tx_messages_to_app,
            tx_key_to_transit,
            tx_code_to_app,
            tx_verifier_to_app,
            tx_versions_to_app,
            tx_close_to_app,
        };

        thread::spawn(move || cw.run());
        // kickstart the core, which will start by starting a websocket
        // connection
        tx_event_to_core.send(ToCore::API(APIEvent::Start)).unwrap();

        Wormhole {
            code: None,
            key: None,
            welcome: None,
            versions: None,
            verifier: None,
            tx_event_to_core,
            rx_messages_from_core,
            rx_welcome_from_core,
            rx_key_from_transit,
            rx_code_from_core,
            rx_verifier_from_core,
            rx_versions_from_core,
            rx_close_from_core,
        }
    }

    pub fn set_code(&mut self, code: &str) {
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::SetCode(Code(code.to_string()))))
            .unwrap();
    }

    pub fn allocate_code(&mut self, num_words: usize) {
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::AllocateCode(num_words)))
            .unwrap();
    }

    pub fn send_message(&mut self, msg: &[u8]) {
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::Send(msg.to_vec())))
            .unwrap();
    }

    pub fn get_message(&mut self) -> Vec<u8> {
        //b"fake".to_vec()
        // TODO: close, by first sending the mood on a separate channel, then
        // dropping the receiver. We should react to getting a RecvError from
        // .recv() by returning self.mood
        self.rx_messages_from_core.recv().unwrap()
    }

    pub fn close(&mut self) -> Mood {
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::Close))
            .unwrap();
        self.rx_close_from_core.recv().unwrap()
    }

    pub fn get_code(&mut self) -> String {
        match self.code {
            Some(ref code) => code.clone(),
            None => {
                let code = self.rx_code_from_core.recv().unwrap();
                self.code = Some(code.clone());
                code
            }
        }
    }

    pub fn get_key(&mut self) -> Vec<u8> {
        match self.key {
            Some(ref key) => key.clone(),
            None => {
                let key = self.rx_key_from_transit.recv().unwrap();
                self.key = Some(key.clone());
                key
            }
        }
    }

    pub fn derive_transit_key(&mut self, appid: &str) -> Vec<u8> {
        let key = self.get_key();
        let mut transit_purpose = appid.to_owned();
        let const_transit_key_str = "/transit-key";
        transit_purpose.push_str(const_transit_key_str);

        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        derive_key(&key, &transit_purpose.as_bytes().to_vec(), length)
    }

    pub fn get_verifier(&mut self) -> Vec<u8> {
        match self.verifier {
            Some(ref verifier) => verifier.clone(),
            None => {
                let verifier = self.rx_verifier_from_core.recv().unwrap();
                self.verifier = Some(verifier.clone());
                verifier
            }
        }
    }

    pub fn get_versions(&mut self) -> Value {
        match self.versions {
            Some(ref versions) => versions.clone(),
            None => {
                let versions = self.rx_versions_from_core.recv().unwrap();
                self.versions = Some(versions.clone());
                versions
            }
        }
    }

    pub fn get_welcome(&mut self) -> Value {
        match self.welcome {
            Some(ref welcome) => welcome.clone(),
            None => {
                let welcome = self.rx_welcome_from_core.recv().unwrap();
                self.welcome = Some(welcome.clone());
                welcome
            }
        }
    }

    pub fn send_file(&mut self, filename: &str, filesize: u64, key: &[u8], relay_url: &RelayUrl) {
        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("0.0.0.0:0").unwrap();
        let listen_socket = listener.local_addr().unwrap();
        let _sockaddrs_iter = listen_socket.to_socket_addrs().unwrap();
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

        let transit_msg = transit(abilities, our_hints).serialize();
        println!("transit_msg: {:?}", transit_msg);

        // send the transit message
        self.send_message(transit_msg.as_bytes());

        // 5. receive transit message from peer.
        let msg = self.get_message();
        let maybe_transit = PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
        println!("received transit message: {:?}", maybe_transit);

        let ttype = match maybe_transit {
            PeerMessage::Transit(tmsg) => tmsg,
            _ => panic!("unexpected message: {:?}", maybe_transit),
        };

        // 6. send file offer message.
        let offer_msg = offer_file(filename, filesize).serialize();
        self.send_message(offer_msg.as_bytes());
        
        // 7. wait for file_ack
        let maybe_fileack = self.get_message();
        let fileack_msg = PeerMessage::deserialize(str::from_utf8(&maybe_fileack).unwrap());
        println!("received file ack message: {:?}", fileack_msg);

        match fileack_msg {
            PeerMessage::Answer(AnswerType::FileAck(msg)) => {
                if msg != "ok" {
                    panic!("file ack failed");
                }
            },
            _ => panic!("did not receive file ack")
        }
        
        // 8. listen for connections on the port and simultaneously try connecting to the peer port.
        // extract peer's ip/hostname from 'ttype'
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);

        // TODO: combine our relay hints with the peer's relay hints.
        
        // TODO: connection establishment and handshake should happen concurrently
        // and whichever handshake succeeds should only proceed. Right now, it is
        // serial and painfully slow.
        for host in hosts {
            println!("host: {:?}", host);
            let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
            let direct_host = direct_host_iter.next().unwrap();

            println!("peer host: {}", direct_host);
            let val = connect_or_accept(direct_host);

            match val {
                Ok((socket, _addr)) => {
                    println!("connected to {:?}", direct_host);
                    let result = tcp_file_send(socket, host.0, key, filename);
                    match result {
                        Ok(()) => break,
                        _ => continue
                    }
                },
                Err(_) => {
                    println!("could not connect to {:?}", direct_host);
                    continue
                },
            }
        }
    }

    pub fn receive_file(&mut self, key: &[u8], ttype: TransitType, relay_url: &RelayUrl) {
        // 1. start a tcp server on a random port
        let listener = TcpListener::bind("0.0.0.0:0").unwrap();
        let listen_socket = listener.local_addr().unwrap();
        let _sockaddrs_iter = listen_socket.to_socket_addrs().unwrap();
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

        let transit_msg = transit(abilities, our_hints).serialize();

        // send the transit message
        self.send_message(transit_msg.as_bytes());

        // 3. receive file offer message from peer
        let msg = self.get_message();
        let maybe_offer = PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
        println!("received offer message: {:?}", maybe_offer);

        let (filename, filesize) = match maybe_offer {
            PeerMessage::Offer(offer_type) => {
                match offer_type {
                    OfferType::File{filename, filesize} => (filename, filesize),
                    _ => panic!("unexpected offer type"),
                }
            },
            _ => panic!("unexpected message: {:?}", maybe_offer),
        };

        // send file ack.
        let file_ack_msg = file_ack("ok").serialize();
        self.send_message(file_ack_msg.as_bytes());

        // 4. listen for connections on the port and simultaneously try connecting to the
        //    peer listening port.
        let (mut direct_hosts, mut relay_hosts) = get_direct_relay_hosts(&ttype);

        let mut hosts: Vec<(HostType, &DirectType)> = Vec::new();
        hosts.append(&mut direct_hosts);
        hosts.append(&mut relay_hosts);

        // TODO: combine our relay hints with the peer's relay hints.
        
        // TODO: connection establishment and handshake should happen concurrently
        // and whichever handshake succeeds should only proceed. Right now, it is
        // serial and painfully slow.
        for host in hosts {
            println!("host: {:?}", host);
            let mut direct_host_iter = format!("{}:{}", host.1.hostname, host.1.port).to_socket_addrs().unwrap();
            let direct_host = direct_host_iter.next().unwrap();

            println!("peer host: {}", direct_host);
            let val = connect_or_accept(direct_host);

            println!("returned from connect_or_accept");

            match val {
                Ok((mut socket, _addr)) => {
                    println!("connected to {:?}", direct_host);

                    // create record keys
                    let (skey, rkey) = make_record_keys(key);
        
                    // exchange handshake
                    let tside = generate_transit_side();
                    println!("{:?}", tside);

                    let result = if host.0 == HostType::Relay {
                        relay_handshake_exchange(&mut socket, key, &tside)
                    }
                    else {
                        Ok(())
                    };

                    match result {
                        Ok(()) => {
                            rx_handshake_exchange(&mut socket, key, &tside.as_bytes()).unwrap();

                            // 5. receive encrypted records
                            // now skey and rkey can be used. skey is used by the tx side, rkey is used
                            // by the rx side for symmetric encryption.
                            let checksum = receive_records(&filename, filesize, &mut socket, &skey);

                            let sha256sum = hex::encode(checksum.as_slice());
                            println!("sha256 sum: {:?}", sha256sum);

                            // 6. verify sha256 sum by sending an ack message to peer along with checksum.
                            let ack_msg = make_transit_ack_msg(&sha256sum, &rkey);
                            send_record(&mut socket, &ack_msg).unwrap();

                            // 7. close socket.
                            // well, no need, it gets dropped when it goes out of scope.

                            break
                        },
                        Err(s) => panic!("Relay handshake failed: {}", s),
                    }
                },
                Err(_) => {
                    println!("could not connect to {:?}", direct_host);
                    continue
                },
            }
        }
    }
    
    pub fn send(&mut self, app_id: String, _code: String, msg: MessageType, relay_url: &RelayUrl) {
        match msg {
            MessageType::Message(text) => {
                self.send_message(message(&text).serialize().as_bytes());
                println!("sent..");
                // if we close right away, we won't actually send anything. Wait for at
                // least the verifier to be printed, that ought to give our outbound
                // message a chance to be delivered.
                let verifier = self.get_verifier();
                println!("verifier: {}", hex::encode(verifier));
                println!("got verifier, closing..");
                self.close();
                println!("closed");
            },
            MessageType::File{filename, filesize} => {
                let k = self.derive_transit_key(&app_id);
                self.send_file(&filename, filesize, &k, relay_url);
            }
        }
    }
    
    // TODO this method should not be static
    pub fn receive(mailbox_server: String, app_id: String, code: String, relay_url: &RelayUrl) -> String {
        trace!("connecting..");
        let mut w = Wormhole::new(&app_id, &mailbox_server);
        w.set_code(&code);
        let verifier = w.get_verifier();
        trace!("verifier: {}", hex::encode(verifier));
        trace!("receiving..");
        let msg = w.get_message();
        let actual_message =
            PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
        let remote_msg = match actual_message {
            PeerMessage::Offer(offer) => match offer {
                OfferType::Message(msg) => {
                    trace!("{}", msg);
                    w.send_message(message_ack("ok").serialize().as_bytes());
                    msg
                }
                OfferType::File { .. } => {
                    trace!("Received file offer {:?}", offer);
                    w.send_message(file_ack("ok").serialize().as_bytes());
                    "".to_string()
                }
                OfferType::Directory { .. } => {
                    trace!("Received directory offer: {:?}", offer);
                    // TODO: We are doing file_ack without asking user
                    w.send_message(file_ack("ok").serialize().as_bytes());
                    "".to_string()
                }
            },
            PeerMessage::Answer(_) => {
                panic!("Should not receive answer type, I'm receiver")
            },
            PeerMessage::Error(err) => {
                trace!("Something went wrong: {}", err);
                "".to_string()
            },
            PeerMessage::Transit(transit) => {
                // TODO: This should start transit server connection or direct file transfer
                // first derive a transit key.
                let k = w.derive_transit_key(&app_id);
                println!("Transit Message received: {:?}", transit);
                w.receive_file(&k, transit, relay_url);
                "".to_string()
            }
        };
        trace!("closing..");
        w.close();
        trace!("closed");
    
        //let remote_msg = "foobar".to_string();
        remote_msg
    }
}

fn make_transit_ack_msg(sha256: &str, key: &[u8]) -> Vec<u8> {
    let plaintext = transit_ack("ok", sha256).serialize();

    let nonce_slice: [u8; sodiumoxide::crypto::secretbox::NONCEBYTES]
        = [0; sodiumoxide::crypto::secretbox::NONCEBYTES];
    let nonce = secretbox::Nonce::from_slice(&nonce_slice[..]).unwrap();

    encrypt_record(&plaintext.as_bytes().to_vec(), nonce, &key)
}

fn generate_transit_side() -> String {
    let x: [u8; 8] = rand::random();
    hex::encode(x)
}

fn connect_or_accept(addr: SocketAddr) -> Result<(TcpStream, SocketAddr), std::io::Error> {
    // let listen_socket = thread::spawn(move || {
    //     listener.accept()
    // });

    let connect_socket = thread::spawn(move || {
        let five_seconds = Duration::new(5, 0);
        let tcp_stream = TcpStream::connect_timeout(&addr, five_seconds);
        match tcp_stream {
            Ok(stream) => Ok((stream, addr)),
            Err(e) => Err(e)
        }
    });

    connect_socket.join().unwrap()
}

fn encrypt_record(plaintext: &[u8], nonce: secretbox::Nonce, key: &[u8]) -> Vec<u8> {
    let sodium_key = secretbox::Key::from_slice(&key).unwrap();
    // nonce in little endian (to interop with python client)
    let mut nonce_vec = nonce.as_ref().to_vec();
    nonce_vec.reverse();
    let maybe_nonce_le = secretbox::Nonce::from_slice(nonce_vec.as_ref());
    let nonce_le = match maybe_nonce_le {
        Some(nonce_le) => nonce_le,
        None => panic!("encrypt_record: unable to create nonce"),
    };
    let ciphertext = secretbox::seal(plaintext, &nonce_le, &sodium_key);
    let mut ciphertext_and_nonce = Vec::new();
    println!("nonce: {:?}", nonce_vec);
    ciphertext_and_nonce.extend(nonce_vec);
    ciphertext_and_nonce.extend(ciphertext);

    ciphertext_and_nonce
}

fn decrypt_record(enc_packet: &[u8], key: &[u8]) -> Vec<u8> {
    // 3. decrypt the vector 'enc_packet' with the key.
    let (nonce, ciphertext) =
        enc_packet.split_at(sodiumoxide::crypto::secretbox::NONCEBYTES);

    assert_eq!(nonce.len(), sodiumoxide::crypto::secretbox::NONCEBYTES);
    let plaintext = secretbox::open(
        &ciphertext,
        &secretbox::Nonce::from_slice(nonce).expect("nonce unwrap failed"),
        &secretbox::Key::from_slice(&key).expect("key unwrap failed"),
    ).expect("decryption failed");

    println!("decryption succeeded");
    plaintext
}

/// receive a packet and return it (encrypted)
fn receive_record<T: Read>(stream: &mut BufReader<T>) -> Vec<u8> {
    // 1. read 4 bytes from the stream. This represents the length of the encrypted packet.
    let mut length_arr: [u8; 4] = [0; 4];
    stream.read_exact(&mut length_arr[..]).unwrap();
    let mut length = u32::from_be_bytes(length_arr);
    println!("encrypted packet length: {}", length);

    // 2. read that many bytes into an array (or a vector?)
    let enc_packet_length = length as usize;
    let mut enc_packet = Vec::with_capacity(enc_packet_length);
    let mut buf = [0u8; 1024];
    while length > 0 {
        let to_read = length.min(buf.len() as u32) as usize;
        if stream.read_exact(&mut buf[..to_read]).is_err() {
            panic!("cannot read from the tcp connection");
        }
        enc_packet.append(&mut buf.to_vec());
        length -= to_read as u32;
    }

    enc_packet.truncate(enc_packet_length);
    println!("length of the ciphertext: {:?}", enc_packet.len());

    enc_packet
}

fn receive_records(filepath: &str, filesize: u64, tcp_conn: &mut TcpStream, skey: &[u8]) -> Vec<u8> {
    let mut stream = BufReader::new(tcp_conn);
    let mut hasher = Sha256::default();
    let mut f = File::create(filepath).unwrap();
    let mut remaining_size = filesize as usize;

    while remaining_size > 0 {
        println!("remaining size: {:?}", remaining_size);

        let enc_packet = receive_record(&mut stream);

        // enc_packet.truncate(enc_packet_length);
        println!("length of the ciphertext: {:?}", enc_packet.len());

        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = decrypt_record(&enc_packet, &skey);

        println!("decryption succeeded");
        f.write_all(&plaintext).unwrap();

        // 4. calculate a rolling sha256 sum of the decrypted output.
        hasher.input(&plaintext);

        remaining_size -= plaintext.len();
    }

    println!("done");
    // TODO: 5. write the buffer into a file.
    hasher.result().to_vec()
}

fn derive_key_from_purpose(key: &[u8], purpose: &str) -> Vec<u8> {
    let length = sodiumoxide::crypto::secretbox::KEYBYTES;
    derive_key(key, &purpose.as_bytes().to_vec(), length)
}

fn make_record_keys(key: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let s_purpose = "transit_record_sender_key";
    let r_purpose = "transit_record_receiver_key";

    let sender = derive_key_from_purpose(key, s_purpose);
    let receiver = derive_key_from_purpose(key, r_purpose);

    (sender, receiver)
}

fn send_buffer(stream: &mut TcpStream, buf: &[u8]) -> io::Result<usize> {
    stream.write(buf)
}

fn send_record(stream: &mut TcpStream, buf: &[u8]) -> io::Result<usize> {
    let buf_length: u32 = buf.len() as u32;
    println!("record size: {:?}", buf_length);
    let buf_length_array: [u8; 4] = buf_length.to_be_bytes();
    stream.write_all(&buf_length_array[..]).unwrap();
    stream.write(buf)
}
    
fn recv_buffer(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<()> {
    stream.read_exact(buf)
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
    println!("relay handshake message: {}", msg);
    msg
}

fn tx_handshake_exchange(sock: &mut TcpStream, key: &[u8], _tside: &[u8]) -> Result<(), String>{
    // send handshake and receive handshake
    let tx_handshake = make_send_handshake(key);
    let rx_handshake = make_receive_handshake(key);

    let tx_handshake_msg = tx_handshake.as_bytes();
    let rx_handshake_msg = rx_handshake.as_bytes();
        
    // for transmit mode, send send_handshake_msg and compare.
    // the received message with send_handshake_msg
    send_buffer(sock, tx_handshake_msg).unwrap();

    let mut rx: [u8; 89] = [0; 89];
    recv_buffer(sock, &mut rx).unwrap();

    println!("{:?}", rx_handshake_msg.to_vec().len());

    let r_handshake = rx_handshake_msg.to_vec();
    let go_msg = b"go\n";
    if r_handshake == rx.to_vec() {
        // send the "go/n" message
        send_buffer(sock, go_msg).unwrap();
        Ok(())
    }
    else {
        Err("handshake failed".to_string())
    }
}

fn rx_handshake_exchange(sock: &mut TcpStream, key: &[u8], _tside: &[u8]) -> Result<(), String>{
    // send handshake and receive handshake
    let send_handshake_msg = make_send_handshake(key);
    let rx_handshake = make_receive_handshake(key);
    let receive_handshake_msg = rx_handshake.as_bytes();

    // for receive mode, send receive_handshake_msg and compare.
    // the received message with send_handshake_msg
    send_buffer(sock, receive_handshake_msg).unwrap();

    let mut rx: [u8; 90] = [0; 90];
    recv_buffer(sock, &mut rx).unwrap();

    let mut s_handshake = send_handshake_msg.as_bytes().to_vec();
    let go_msg = b"go\n";
    s_handshake.append(&mut go_msg.to_vec());
    if s_handshake == rx.to_vec() {
        Ok(())
    }
    else {
        Err("handshake failed".to_string())
    }
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
fn send_records(filepath: &str, stream: &mut TcpStream, skey: &[u8]) -> Vec<u8> {
    // rough plan:
    // 1. Open the file
    // 2. read a block of N bytes
    // 3. calculate a rolling sha256sum.
    // 4. AEAD with skey and with nonce as a counter from 0.
    // 5. send the encrypted buffer to the socket.
    // 6. go to step #2 till eof.
    // 7. if eof, return sha256 sum.

    let path = Path::new(filepath);

    let mut file = match File::open(&path) {
        Err(why) => panic!("Could not open {}: {}", path.display(), why.to_string()),
        Ok(f) => f
    };

    let mut hasher = Sha256::default();

    let nonce_slice: [u8; sodiumoxide::crypto::secretbox::NONCEBYTES]
        = [0; sodiumoxide::crypto::secretbox::NONCEBYTES];
    let mut nonce = secretbox::Nonce::from_slice(&nonce_slice[..]).unwrap();
        
    loop {
        println!("sending records");
        // read a block of 4096 bytes
        let mut plaintext = [0u8; 4096];
        let n = match file.read(&mut plaintext[..]) {
            Ok(usize) => usize,
            Err(why) => panic!("failed to read from file: {}", why.to_string())
        };

        let mut plaintext_vec = plaintext.to_vec();
        plaintext_vec.truncate(n);
        
        let ciphertext = encrypt_record(&plaintext_vec, nonce, &skey);

        // send the encrypted record
        send_record(stream, &ciphertext).unwrap();

        // increment nonce
        nonce.increment_le_inplace();

        // sha256 of the input
        hasher.input(&plaintext[..n]);

        if n < 4096 {
            break;
        }
        else {
            continue;
        }
    }
    hasher.result().to_vec()
}

fn build_direct_hints(port: u16) -> Vec<Hints> {
    let _localhost = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let ifaces = datalink::interfaces();

    let non_local_ifaces: Vec<&datalink::NetworkInterface> = ifaces.iter().filter(|iface| !datalink::NetworkInterface::is_loopback(iface))
        .collect();
    let ips: Vec<IpNetwork> = non_local_ifaces.iter()
        .map(|iface| iface.ips.clone())
        .flatten()
        .map(|n| n as IpNetwork)
        .filter(|ip| ip.is_ipv4())
        .collect::<Vec<IpNetwork>>();
    println!("ips: {:?}", ips);

    // create abilities and hints
    // TODO: enumerate for all ips, not just ips[0]
    let mut hints = Vec::new();
    hints.push(Hints::DirectTcpV1(DirectType{ priority: 0.0, hostname: ips[0].ip().to_string(), port}));

    hints
}

fn build_relay_hints(relay_url: &RelayUrl) -> Vec<Hints> {
    let mut hints = Vec::new();
    let mut dhints = Vec::new();
    dhints.push(direct_type(0.0, &relay_url.host.clone(), relay_url.port));
    hints.push(Hints::RelayV1(relay_type(dhints)));

    hints
}

fn relay_handshake_exchange(sock: &mut TcpStream, key: &[u8], tside: &str) -> Result<(), String> {
    let relay_handshake = make_relay_handshake(key, tside);
    let relay_handshake_msg = relay_handshake.as_bytes();

    send_buffer(sock, relay_handshake_msg).unwrap();

    let mut rx = [0u8; 3];
    recv_buffer(sock, &mut rx).unwrap();

    let ok_msg: [u8; 3] = *b"ok\n";
    if ok_msg == rx {
        println!("relay handshake succeeded");
        Ok(())
    }
    else {
        // bad handshake (gets back the characters 'b', 'a', 'd')
        Err("relay handshake failed".to_string())
    }
}

fn tcp_file_send(mut socket: TcpStream, host_type: HostType, key: &[u8], filename: &str) -> Result<(), String> {

    // 9. create record keys
    let (skey, rkey) = make_record_keys(key);
    println!("created record keys");

    // 10. exchange handshake over tcp
    let tside = generate_transit_side();

    let result = if host_type == HostType::Relay {
        relay_handshake_exchange(&mut socket, key, &tside)
    }
    else {
        Ok(())
    };

    match result {
        Ok(()) => {
            tx_handshake_exchange(&mut socket, key, &tside.as_bytes()).unwrap();
            // 11. send the file as encrypted records.
            // fn send_records(&mut self, filepath: &str, stream: &mut TcpStream, skey: &Vec<u8>) -> Vec<u8>
            println!("handshake successful");
            let checksum = send_records(filename, &mut socket, &skey);

            // 13. wait for the transit ack with sha256 sum from the peer.
            let enc_transit_ack = receive_record(&mut BufReader::new(socket));
            let transit_ack = decrypt_record(&enc_transit_ack, &rkey);

            let transit_ack_msg = TransitAck::deserialize(str::from_utf8(&transit_ack).expect("could not parse the message"));
            if transit_ack_msg.sha256 == hex::encode(checksum) {
                println!("transfer complete!");
                Ok(())
            } else {
                panic!("receive checksum error");
            }
        },
        Err(s) => panic!("Relay handshake failed: {}", s),
    }
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
                 _ => panic!("should not come here"),
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
                 _ => panic!("should not come here"),
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
