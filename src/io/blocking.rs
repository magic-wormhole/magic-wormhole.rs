use crate::core::WormholeCore;
use crate::core::{
    APIAction, APIEvent, Action, Code, IOAction, IOEvent, Mood, TimerHandle,
    WSHandle,
    PeerMessage,
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
use log::*;
use anyhow::{Result, Error, ensure, bail, format_err, Context};

pub mod transit;
pub mod filetransfer;

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
        let derived_key = derive_key(&key, &transit_purpose.as_bytes(), length);
        trace!("Input key: {}, Transit key: {}, Transit purpose: '{}'", hex::encode(&key), hex::encode(&derived_key), &transit_purpose);
        derived_key
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
    
    #[deprecated(note = "This is application-specific code which doesn't belong into the API")]
    pub fn send(&mut self, app_id: &str, _code: &str, msg: MessageType, relay_url: &transit::RelayUrl) -> Result<()> {
        match msg {
            MessageType::Message(text) => {
                self.send_message(PeerMessage::new_offer_message(&text).serialize().as_bytes());
                debug!("sent..");
                // if we close right away, we won't actually send anything. Wait for at
                // least the verifier to be printed, that ought to give our outbound
                // message a chance to be delivered.
                // TODO this should not be required. If send_message ought to be blocking, the message should be sent
                // once it returns.
                let verifier = self.get_verifier();
                trace!("verifier: {}", hex::encode(verifier));
                trace!("got verifier, closing..");
                self.close();
                trace!("closed");
            },
            MessageType::File{filename, filesize} => {
                async_std::task::block_on(filetransfer::send_file(self, &filename, app_id, relay_url))?;
                debug!("send closed");
            }
        }
        Ok(())
    }

    #[deprecated(note = "This is application-specific code which doesn't belong into the API")]
    pub fn receive(&mut self, app_id: &str, relay_url: &transit::RelayUrl) -> Result<String> {
        let msg = self.get_message();
        let actual_message =
            PeerMessage::deserialize(str::from_utf8(&msg)?);
        let remote_msg = match actual_message {
            PeerMessage::Offer(offer) => match offer {
                OfferType::Message(msg) => {
                    debug!("{}", msg);
                    self.send_message(PeerMessage::new_message_ack("ok").serialize().as_bytes());
                    msg
                }
                OfferType::File { .. } => {
                    debug!("Received file offer {:?}", offer);
                    // TODO: We are doing file_ack without asking user
                    self.send_message(PeerMessage::new_file_ack("ok").serialize().as_bytes());
                    "".to_string()
                }
                OfferType::Directory { .. } => {
                    debug!("Received directory offer: {:?}", offer);
                    // TODO: We are doing file_ack without asking user
                    self.send_message(PeerMessage::new_file_ack("ok").serialize().as_bytes());
                    "".to_string()
                }
            },
            PeerMessage::Answer(_) => {
                bail!("Should not receive answer type, I'm receiver")
            },
            PeerMessage::Error(err) => {
                debug!("Something went wrong: {}", err);
                "".to_string()
            },
            PeerMessage::Transit(transit) => {
                debug!("Transit Message received: {:?}", transit);
                async_std::task::block_on(filetransfer::receive_file(self, transit, app_id, relay_url))?;
                "".to_string()
            }
        };
        debug!("closing..");
        self.close();
        debug!("closed");

        //let remote_msg = "foobar".to_string();
        Ok(remote_msg)
    }
}

fn derive_key_from_purpose(key: &[u8], purpose: &str) -> Vec<u8> {
    let length = sodiumoxide::crypto::secretbox::KEYBYTES;
    derive_key(key, &purpose.as_bytes(), length)
}
