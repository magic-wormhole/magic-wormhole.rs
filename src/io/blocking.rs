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

#[deprecated]
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

async fn ws_connector(
    url: &str,
    handle: WSHandle,
    tx: Sender<ToCore>,
    ws_rx: futures::channel::mpsc::UnboundedReceiver<WSControl>,
) {
    use async_tungstenite::async_std::*;
    use futures::stream::StreamExt;
    use futures::sink::SinkExt;
    use async_tungstenite::tungstenite as ws2;

    let (ws_stream, _) = connect_async(url).await.unwrap();
    tx.send(ToCore::WebSocketConnectionMade(handle)).unwrap();
    let (mut write, mut read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    async_std::task::spawn(async move {
        while let Some(message) = read.next().await {
            match message.unwrap() {
                ws2::Message::Text(text) => {
                    tx.send(ToCore::WebSocketMessageReceived(handle, text)).unwrap();
                },
                ws2::Message::Close(_) => {
                    tx.send(ToCore::WebSocketConnectionLost(handle)).unwrap();
                },
                other => panic!(format!("Got an unexpected websocket message: {}", other))
            }
        }
        // read.for_each(|message| async {
        //     match message.unwrap() {
        //         ws2::Message::Text(text) => {
        //             println!("B");
        //             tx.send(ToCore::WebSocketMessageReceived(handle, text)).unwrap();
        //         },
        //         ws2::Message::Close(_) => {
        //             println!("C");
        //             tx.send(ToCore::WebSocketConnectionLost(handle)).unwrap();
        //         },
        //         _ => panic!()
        //     }
        // }).await;
    });
    /* Send events from the API to the other websocket side */
    async_std::task::spawn(async move {
        ws_rx
        .map(|c| {
            match c {
                WSControl::Data(d) => {
                    ws2::Message::Text(d)
                },
                WSControl::Close => ws2::Message::Close(None),
            }
        })
        .map(Ok)
        .forward(write)
        .await
        .unwrap();
    });
}

struct CoreWrapper {
    core: WormholeCore,

    tx_to_core: Sender<ToCore>, // give clones to websocket/timer threads
    rx_by_core: Receiver<ToCore>,

    timers: HashSet<TimerHandle>,
    websockets: HashMap<WSHandle, futures::channel::mpsc::UnboundedSender<WSControl>>,

    tx_welcome_to_app: futures::channel::mpsc::UnboundedSender<Value>,
    tx_messages_to_app: futures::channel::mpsc::UnboundedSender<Vec<u8>>,
    tx_key_to_transit: futures::channel::mpsc::UnboundedSender<Vec<u8>>,
    tx_code_to_app: futures::channel::mpsc::UnboundedSender<String>,
    tx_verifier_to_app: futures::channel::mpsc::UnboundedSender<Vec<u8>>,
    tx_versions_to_app: futures::channel::mpsc::UnboundedSender<Value>,
    tx_close_to_app: futures::channel::mpsc::UnboundedSender<Mood>,
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
            async_std::task::block_on(async {
                for action in actions {
                    use futures::SinkExt;
                    use self::APIAction::*;
                    use self::IOAction::*;
                    match action {
                        Action::API(GotWelcome(w)) => self.tx_welcome_to_app.send(w).await.unwrap(),
                        Action::API(GotMessage(m)) => self.tx_messages_to_app.send(m).await.unwrap(),
                        Action::API(GotCode(c)) => self.tx_code_to_app.send(c.to_string()).await.unwrap(),
                        Action::API(GotUnverifiedKey(k)) => self.tx_key_to_transit.send(k.to_vec()).await.unwrap(),
                        Action::API(GotVerifier(v)) => self.tx_verifier_to_app.send(v).await.unwrap(),
                        Action::API(GotVersions(v)) => self.tx_versions_to_app.send(v).await.unwrap(),
                        Action::API(GotClosed(mood)) => self.tx_close_to_app.send(mood).await.unwrap(),
                        Action::IO(StartTimer(handle, duration)) => {
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
                        Action::IO(CancelTimer(handle)) => {
                            self.timers.remove(&handle);
                        },
                        Action::IO(WebSocketOpen(handle, url)) => {
                            let tx = self.tx_to_core.clone();
                            let (ws_tx, ws_rx) = futures::channel::mpsc::unbounded();
                            self.websockets.insert(handle, ws_tx);
                            async_std::task::block_on(async move {
                                ws_connector(&url, handle, tx, ws_rx).await;
                            });
                        },
                        Action::IO(WebSocketSendMessage(handle, msg)) => {
                            self.websockets.get_mut(&handle).unwrap()
                                .send(WSControl::Data(msg)).await.unwrap();
                        },
                        Action::IO(WebSocketClose(handle)) => {
                            self.websockets.get_mut(&handle).unwrap()
                                .send(WSControl::Close).await.unwrap();
                            self.websockets.remove(&handle);
                        },
                    }
                }
            });
        }
    }
}

// we have one channel per API pathway
pub struct Wormhole {
    tx_event_to_core: Sender<ToCore>,

    rx_welcome_from_core: futures::channel::mpsc::UnboundedReceiver<Value>,
    rx_messages_from_core: futures::channel::mpsc::UnboundedReceiver<Vec<u8>>,
    rx_key_from_transit: futures::channel::mpsc::UnboundedReceiver<Vec<u8>>,
    rx_code_from_core: futures::channel::mpsc::UnboundedReceiver<String>,
    rx_verifier_from_core: futures::channel::mpsc::UnboundedReceiver<Vec<u8>>,
    rx_versions_from_core: futures::channel::mpsc::UnboundedReceiver<Value>,
    rx_close_from_core: futures::channel::mpsc::UnboundedReceiver<Mood>,

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
        let (tx_messages_to_app, rx_messages_from_core) = futures::channel::mpsc::unbounded();
        let (tx_welcome_to_app, rx_welcome_from_core) = futures::channel::mpsc::unbounded();
        let (tx_key_to_transit, rx_key_from_transit) = futures::channel::mpsc::unbounded();
        let (tx_code_to_app, rx_code_from_core) = futures::channel::mpsc::unbounded();
        let (tx_verifier_to_app, rx_verifier_from_core) = futures::channel::mpsc::unbounded();
        let (tx_versions_to_app, rx_versions_from_core) = futures::channel::mpsc::unbounded();
        let (tx_close_to_app, rx_close_from_core) = futures::channel::mpsc::unbounded();

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

    pub async fn get_message(&mut self) -> Vec<u8> {
        use futures::StreamExt;
        //b"fake".to_vec()
        // TODO: close, by first sending the mood on a separate channel, then
        // dropping the receiver. We should react to getting a RecvError from
        // .recv() by returning self.mood
        self.rx_messages_from_core.next().await.unwrap()
    }

    pub async fn close(&mut self) -> Mood {
        use futures::StreamExt;
        self.tx_event_to_core
            .send(ToCore::API(APIEvent::Close))
            .unwrap();
        self.rx_close_from_core.next().await.unwrap()
    }

    pub async fn get_code(&mut self) -> String {
        match self.code {
            Some(ref code) => code.clone(),
            None => {
                use futures::StreamExt;
                let code = self.rx_code_from_core.next().await.unwrap();
                self.code = Some(code.clone());
                code
            }
        }
    }

    pub async fn get_key(&mut self) -> Vec<u8> {
        match self.key {
            Some(ref key) => key.clone(),
            None => {
                use futures::StreamExt;
                let key = self.rx_key_from_transit.next().await.unwrap();
                self.key = Some(key.clone());
                key
            }
        }
    }

    pub async fn derive_transit_key(&mut self, appid: &str) -> Vec<u8> {
        let key = self.get_key().await;
        let mut transit_purpose = appid.to_owned();
        let const_transit_key_str = "/transit-key";
        transit_purpose.push_str(const_transit_key_str);

        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        let derived_key = derive_key(&key, &transit_purpose.as_bytes(), length);
        trace!("Input key: {}, Transit key: {}, Transit purpose: '{}'", hex::encode(&key), hex::encode(&derived_key), &transit_purpose);
        derived_key
    }

    pub async fn get_verifier(&mut self) -> Vec<u8> {
        match self.verifier {
            Some(ref verifier) => verifier.clone(),
            None => {
                use futures::StreamExt;
                let verifier = self.rx_verifier_from_core.next().await.unwrap();
                self.verifier = Some(verifier.clone());
                verifier
            }
        }
    }

    pub async fn get_versions(&mut self) -> Value {
        match self.versions {
            Some(ref versions) => versions.clone(),
            None => {
                use futures::StreamExt;
                let versions = self.rx_versions_from_core.next().await.unwrap();
                self.versions = Some(versions.clone());
                versions
            }
        }
    }

    pub async fn get_welcome(&mut self) -> Value {
        match self.welcome {
            Some(ref welcome) => welcome.clone(),
            None => {
                use futures::StreamExt;
                let welcome = self.rx_welcome_from_core.next().await.unwrap();
                self.welcome = Some(welcome.clone());
                welcome
            }
        }
    }
    
    #[deprecated(note = "This is application-specific code which doesn't belong into the API")]
    pub async fn send(&mut self, app_id: &str, _code: &str, msg: MessageType, relay_url: &transit::RelayUrl) -> Result<()> {
        match msg {
            MessageType::Message(text) => {
                self.send_message(PeerMessage::new_offer_message(&text).serialize().as_bytes());
                debug!("sent..");
                // if we close right away, we won't actually send anything. Wait for at
                // least the verifier to be printed, that ought to give our outbound
                // message a chance to be delivered.
                // TODO this should not be required. If send_message ought to be blocking, the message should be sent
                // once it returns.
                let verifier = self.get_verifier().await;
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
    pub async fn receive(&mut self, app_id: &str, relay_url: &transit::RelayUrl) -> Result<String> {
        let msg = self.get_message().await;
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
