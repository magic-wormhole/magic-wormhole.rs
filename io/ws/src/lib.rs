extern crate magic_wormhole_core;
extern crate ws;
use magic_wormhole_core::WormholeCore;
use magic_wormhole_core::{APIAction, APIEvent, IOAction, IOEvent, Action,
                          TimerHandle};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use std::collections::{HashMap, HashSet};
use std::time;

enum ToCore {
    API(APIEvent),
    #[allow(dead_code)]
    IO(IOEvent),
    TimerExpired(TimerHandle), // handle
}

#[allow(dead_code)]
enum XXXFromCore {
    API(APIAction),
    IO(IOAction),
}

struct CoreWrapper {
    core: WormholeCore,

    tx_to_core: Sender<ToCore>, // give clones to websocket/timer threads
    rx_by_core: Receiver<ToCore>,

    timers: HashSet<TimerHandle>,


    tx_welcome_to_app: Sender<HashMap<String, String>>,
    tx_messages_to_app: Sender<Vec<u8>>,
    tx_code_to_app: Sender<String>,
    tx_verifier_to_app: Sender<Vec<u8>>,
    tx_versions_to_app: Sender<HashMap<String, String>>,
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
                },
            };
            for action in actions {
                self.process_action(action);
            }
        }
    }

    fn process_action(&mut self, action: Action) {
        match action {
            Action::API(a) => {
                use APIAction::*;
                match a {
                    GotWelcome(w) => self.tx_welcome_to_app.send(w).unwrap(),
                    GotMessage(m) => self.tx_messages_to_app.send(m).unwrap(),
                    GotCode(c) => self.tx_code_to_app.send(c).unwrap(),
                    GotUnverifiedKey(_k) => (),
                    GotVerifier(v) => self.tx_verifier_to_app.send(v).unwrap(),
                    GotVersions(v) => self.tx_versions_to_app.send(v).unwrap(),
                    GotClosed(_mood) => (),
                }
            },
            Action::IO(i) => {
                use IOAction::*;
                match i {
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
                    },
                    CancelTimer(handle) => {
                        self.timers.remove(&handle);
                    },
                    WebSocketOpen(_handle, _url) => (),
                    WebSocketSendMessage(_handle, _msg) => (),
                    WebSocketClose(_handle) => (),
                }
            }
        }
    }
}


// we have one channel per API pathway
pub struct Wormhole {
    tx_event_to_core: Sender<ToCore>,

    rx_welcome_from_core: Receiver<HashMap<String, String>>,
    rx_messages_from_core: Receiver<Vec<u8>>,
    rx_code_from_core: Receiver<String>,
    rx_verifier_from_core: Receiver<Vec<u8>>,
    rx_versions_from_core: Receiver<HashMap<String, String>>,

    code: Option<String>,
    welcome: Option<HashMap<String, String>>,
    versions: Option<HashMap<String, String>>,
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

        let mut cw = CoreWrapper {
            core: WormholeCore::new(appid, relay_url),
            tx_to_core: tx_event_to_core.clone(),
            rx_by_core: rx_by_core,
            timers: HashSet::new(),
            tx_welcome_to_app: tx_welcome_to_app,
            tx_messages_to_app: tx_messages_to_app,
            tx_code_to_app: tx_code_to_app,
            tx_verifier_to_app: tx_verifier_to_app,
            tx_versions_to_app: tx_versions_to_app,
        };

        thread::spawn(move|| { cw.run() } );

        Wormhole {
            code: None,
            welcome: None,
            versions: None,
            verifier: None,
            tx_event_to_core: tx_event_to_core,
            rx_messages_from_core: rx_messages_from_core,
            rx_welcome_from_core: rx_welcome_from_core,
            rx_code_from_core: rx_code_from_core,
            rx_verifier_from_core: rx_verifier_from_core,
            rx_versions_from_core: rx_versions_from_core,
        }
    }

    pub fn set_code(&mut self, code: &str) {
        self.tx_event_to_core.send(ToCore::API(APIEvent::SetCode(code.to_string()))).unwrap();
    }

    pub fn send_message(&mut self, msg: &[u8]) {
        self.tx_event_to_core.send(ToCore::API(APIEvent::Send(msg.to_vec()))).unwrap();
    }

    pub fn get_message(&mut self) -> Vec<u8> {
        //b"fake".to_vec()
        // TODO: close, by first sending the mood on a separate channel, then
        // dropping the receiver. We should react to getting a RecvError from
        // .recv() by returning self.mood
        self.rx_messages_from_core.recv().unwrap()
    }

    pub fn close(&mut self) {
        self.tx_event_to_core.send(ToCore::API(APIEvent::Close)).unwrap();
        // TODO mood
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

    pub fn get_versions(&mut self) -> HashMap<String, String> {
        match self.versions {
            Some(ref versions) => versions.clone(),
            None => {
                let versions = self.rx_versions_from_core.recv().unwrap();
                self.versions = Some(versions.clone());
                versions
            }
        }
    }

    pub fn get_welcome(&mut self) -> HashMap<String, String> {
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
