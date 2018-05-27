use magic_wormhole_core::WormholeCore;
use magic_wormhole_core::{APIAction, APIEvent, Action, Code, IOAction,
                          IOEvent, Mood, TimerHandle, WSHandle};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time;
use url::Url;
use ws;

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

fn ws_outbound(ws_rx: Receiver<WSControl>, out: ws::Sender) {
    loop {
        match ws_rx.recv() {
            Ok(c) => match c {
                WSControl::Data(d) => out.send(ws::Message::Text(d)).unwrap(),
                WSControl::Close => out.close(ws::CloseCode::Normal).unwrap(),
            },
            Err(_e) => {
                //println!("ws_rx.recv Err {:?}", e);
                break;
            }
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
        thread::spawn(move || ws_outbound(ws_rx, out));
        WSConnection {
            handle: self.handle,
            tx: tx,
        }
    }
}

fn ws_connector(
    url: String,
    handle: WSHandle,
    tx: Sender<ToCore>,
    ws_rx: Receiver<WSControl>,
) {
    // we're called in a new thread created just for this connection
    let f = WSFactory {
        handle: handle,
        tx: Some(tx),
        ws_rx: Some(ws_rx),
    };
    let b = ws::Builder::new();
    let mut w1 = b.build(f).unwrap();
    w1.connect(Url::parse(&url).unwrap()).unwrap();
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
                ToCore::WebSocketConnectionMade(handle) => self.core
                    .do_io(IOEvent::WebSocketConnectionMade(handle)),
                ToCore::WebSocketMessageReceived(handle, msg) => self.core
                    .do_io(IOEvent::WebSocketMessageReceived(handle, msg)),
                ToCore::WebSocketConnectionLost(handle) => self.core
                    .do_io(IOEvent::WebSocketConnectionLost(handle)),
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
            GotUnverifiedKey(_k) => (),
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
        thread::spawn(move || ws_connector(url, handle, tx, ws_rx));
    }

    fn websocket_send(&self, handle: WSHandle, msg: String) {
        self.websockets
            .get(&handle)
            .unwrap()
            .send(WSControl::Data(msg))
            .unwrap();
    }

    fn websocket_close(&mut self, handle: WSHandle) {
        self.websockets
            .get(&handle)
            .unwrap()
            .send(WSControl::Close)
            .unwrap();
        self.websockets.remove(&handle);
    }
}

// we have one channel per API pathway
pub struct Wormhole {
    tx_event_to_core: Sender<ToCore>,

    rx_welcome_from_core: Receiver<Value>,
    rx_messages_from_core: Receiver<Vec<u8>>,
    rx_code_from_core: Receiver<String>,
    rx_verifier_from_core: Receiver<Vec<u8>>,
    rx_versions_from_core: Receiver<Value>,
    rx_close_from_core: Receiver<Mood>,

    code: Option<String>,
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
            tx_code_to_app,
            tx_verifier_to_app,
            tx_versions_to_app,
            tx_close_to_app,
        };

        thread::spawn(move || cw.run());
        // kickstart the core, which will start by starting a websocket
        // connection
        tx_event_to_core
            .send(ToCore::API(APIEvent::Start))
            .unwrap();

        Wormhole {
            code: None,
            welcome: None,
            versions: None,
            verifier: None,
            tx_event_to_core,
            rx_messages_from_core,
            rx_welcome_from_core,
            rx_code_from_core,
            rx_verifier_from_core,
            rx_versions_from_core,
            rx_close_from_core,
        }
    }

    pub fn set_code(&mut self, code: &str) {
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::SetCode(Code(
                code.to_string(),
            ))))
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
}
