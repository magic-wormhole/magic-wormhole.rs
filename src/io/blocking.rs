use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use futures::Sink;
use futures::Stream;
use crate::core::WormholeCore;
use crate::core::{
    APIAction, APIEvent, Code, IOAction, IOEvent, Mood, TimerHandle,
    WSHandle,
    PeerMessage,
    OfferType,
};

use crate::core::key::derive_key;
use serde_json::Value;
use std::thread;
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

pub enum CodeProvider {
    AllocateCode(usize),
    SetCode(String),
}

impl Default for CodeProvider {
    fn default() -> Self {
        CodeProvider::AllocateCode(2)
    }
}

// enum WormholeError {
//     Closed(Mood),
// }

struct WormholeState {
    welcome: Option<String>,
    code: Option<String>,
    verifier: Option<String>,
    versions: Option<String>,
}

pub struct WormholeConnector {
    queued_messages: Vec<APIAction>,
    /* In case we got that too early */
    key: Option<Vec<u8>>,
    tx_api_to_core: UnboundedSender<APIEvent>,
    rx_api_from_core: UnboundedReceiver<APIAction>,
}

impl WormholeConnector {
    pub async fn do_pake(mut self)
    ->
    (
        (impl Sink<Vec<u8>> + std::marker::Unpin),
        (impl Stream<Item = Vec<u8>> + std::marker::Unpin),
        Vec<u8>,
    ) {
        use futures::StreamExt;
        use futures::SinkExt;

        let mut verifier = None;
        let mut versions = None;

        if self.key.is_none() {
            while let Some(action) = self.rx_api_from_core.next().await {
                use self::APIAction::*;
                match action {
                    GotUnverifiedKey(k) => {
                        debug!("Got key");
                        self.key = Some(k.0.clone());
                    },
                    GotVerifier(v) => {
                        debug!("Got verifier");
                        verifier = Some(v);
                    },
                    GotVersions(v) => {
                        debug!("Got version");
                        versions = Some(v);
                    },
                    action @ GotMessage(_) => {
                        warn!("Got message from other side during initialization. Will deliver it after initialization.");
                        self.queued_messages.push(action);
                    },
                    GotWelcome(_) | GotCode(_) => {
                        panic!("TODO I don't want this what is this?!");
                    },
                    GotClosed(mood) => {
                        panic!("TODO return error");
                    },
                }

                if self.key.is_some() {
                    /* We know enough */
                    break;
                }
            }
        }

        let tx_api_to_core = self.tx_api_to_core
            .with(|message| async {Result::<_, futures::channel::mpsc::SendError>::Ok(APIEvent::Send(message))});
        let rx_api_from_core = futures::stream::iter(self.queued_messages)
            .chain(self.rx_api_from_core)
            .filter_map(|action| async { match action {
                APIAction::GotMessage(m) => {
                    Some(m)
                },
                APIAction::GotClosed(_) => {
                    todo!("close streams");
                },
                action => {
                    warn!("Received unexpected action after initialization: '{:?}'", action);
                    // todo!("error handling"); // TODO
                    None
                }
            }});

        (
            Box::pin(tx_api_to_core),
            Box::pin(rx_api_from_core),
            self.key.unwrap(),
        )
    }
}

pub struct Wormhole2 {
    pub code: Code,
    pub welcome: String,
}

impl Wormhole2 {
    pub async fn new(appid: &str, relay_url: &str, code_provider: CodeProvider, )
    -> (
        Wormhole2,
        WormholeConnector,
    ) {
        use futures::channel::mpsc::unbounded;
        use futures::StreamExt;
        use futures::SinkExt;

        let (tx_io_to_core, mut rx_io_to_core) = unbounded();
        let (tx_api_to_core, mut rx_api_to_core) = unbounded();
        let (mut tx_api_from_core, mut rx_api_from_core) = unbounded();
        let io = crate::io::AsyncStdIO::new(tx_io_to_core);
        let mut core = WormholeCore::new(appid, relay_url, io);

        async_std::task::spawn(async move {
            loop {
                let actions = futures::select! {
                    action = rx_api_to_core.select_next_some() => {
                        debug!("Doing API {:?}", action);
                        core.do_api(action)
                    },
                    action = rx_io_to_core.select_next_some() => {
                        debug!("Doing IO {:?}", action);
                        core.do_io(action)
                    },
                };
                debug!("Done API/IO {:?}", &actions);
                for action in actions {
                    dbg!(&action);
                    tx_api_from_core.send(action).await.unwrap();
                }
            }
        });

        let mut code = None;
        let mut welcome = None;
        let mut verifier = None;
        let mut versions = None;
        let mut key = None;
        let mut queued_messages = Vec::new();

        tx_api_to_core.unbounded_send(APIEvent::Start).unwrap();

        match code_provider {
            CodeProvider::AllocateCode(num_words) => {
                tx_api_to_core.unbounded_send(APIEvent::AllocateCode(num_words)).unwrap();
            },
            CodeProvider::SetCode(code) => {
                tx_api_to_core.unbounded_send(APIEvent::SetCode(Code(code))).unwrap();
            }
        }

        while let Some(action) = rx_api_from_core.next().await {
            use self::APIAction::*;
            match action {
                GotWelcome(w) => {
                    debug!("Got welcome");
                    welcome = Some(w.to_string());
                },
                action @ GotMessage(_) => {
                    warn!("Got message from other side during initialization. Will deliver it after initialization.");
                    queued_messages.push(action);
                },
                GotCode(c) => {
                    debug!("Got code");
                    code = Some(c.clone());
                },
                GotUnverifiedKey(k) => {
                    /* This shouldn't happen now, but it might */
                    debug!("Got key");
                    key = Some(k.0.clone());
                },
                GotVerifier(v) => {
                    debug!("Got verifier");
                    verifier = Some(v);
                },
                GotVersions(v) => {
                    debug!("Got version");
                    versions = Some(v);
                },
                GotClosed(mood) => {
                    panic!("TODO return error");
                },
            }

            if welcome.is_some() && code.is_some() {
                /* We know enough */
                break;
            }
        }

        (
            Wormhole2 {
                code: code.unwrap(),
                welcome: welcome.unwrap(),
                // verifier,
                // versions,
            },
            WormholeConnector {
                queued_messages,
                tx_api_to_core,
                rx_api_from_core,
                key,
            }
        )
    }
}

#[deprecated]
struct CoreWrapper {
    core: WormholeCore<crate::io::AsyncStdIO>,

    rx_api_to_core: futures::channel::mpsc::UnboundedReceiver<APIEvent>,
    rx_io_to_core: futures::channel::mpsc::UnboundedReceiver<IOEvent>,

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
        use futures::StreamExt;
        use futures::SinkExt;
        loop {
            // // TODO convert back to not-spinwaiting
            // let mut actions = Vec::new();
            // for event in self.rx_api_to_core.try_iter() {
            //     actions.extend(self.core.do_api(event));
            // }
            // for event in self.rx_io_to_core.try_iter() {
            //     actions.extend(self.core.do_io(event));
            // }
            // let actions = match dbg!(self.rx_by_core.recv().unwrap()) {
                // ToCore::API(a) => self.core.do_api(a),
                // ToCore::IO(i) => self.core.do_io(i),
                // ToCore::TimerExpired(handle) => {
                //     if self.timers.contains(&handle) {
                //         self.core.do_io(IOEvent::TimerExpired(handle))
                //     } else {
                //         vec![]
                //     }
                // }
                // ToCore::WebSocketConnectionMade(handle) => {
                //     self.core.do_io(IOEvent::WebSocketConnectionMade(handle))
                // }
                // ToCore::WebSocketMessageReceived(handle, msg) => self
                //     .core
                //     .do_io(IOEvent::WebSocketMessageReceived(handle, msg)),
                // ToCore::WebSocketConnectionLost(handle) => {
                //     self.core.do_io(IOEvent::WebSocketConnectionLost(handle))
                // }
            // };
            async_std::task::block_on(async {
                let actions = futures::select! {
                    action = self.rx_api_to_core.select_next_some() => {
                        debug!("Doing API {:?}", action);
                        self.core.do_api(action)
                    },
                    action = self.rx_io_to_core.select_next_some() => {
                        debug!("Doing IO {:?}", action);
                        self.core.do_io(action)
                    },
                };
                debug!("Done API/IO {:?}", &actions);
                // for action in actions {
                //     dbg!(&action);
                //     tx_api_from_core.send(action).await.unwrap();
                // }
                for action in actions {
                    use self::APIAction::*;
                    debug!("Executing action {:?}", action);
                    match action {
                        GotWelcome(w) => self.tx_welcome_to_app.send(w).await.unwrap(),
                        GotMessage(m) => self.tx_messages_to_app.send(m).await.unwrap(),
                        GotCode(c) => self.tx_code_to_app.send(c.to_string()).await.unwrap(),
                        GotUnverifiedKey(k) => self.tx_key_to_transit.send(k.to_vec()).await.unwrap(),
                        GotVerifier(v) => self.tx_verifier_to_app.send(v).await.unwrap(),
                        GotVersions(v) => self.tx_versions_to_app.send(v).await.unwrap(),
                        GotClosed(mood) => self.tx_close_to_app.send(mood).await.unwrap(),
                    }
                }
            });
        }
    }
}

// we have one channel per API pathway
#[deprecated]
pub struct Wormhole {
    tx_api_to_core: futures::channel::mpsc::UnboundedSender<APIEvent>,

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
        use futures::channel::mpsc::unbounded;
        use futures::SinkExt;

        // the Wormhole object lives in the same thread as the application,
        // and it blocks. We put the core in a separate thread, and use a
        // channel to talk to it.
        let (mut tx_api_to_core, rx_api_to_core) = unbounded();
        let (tx_io_to_core, rx_io_to_core) = unbounded();
        // the inbound messages get their own channel
        let (tx_messages_to_app, rx_messages_from_core) = unbounded();
        let (tx_welcome_to_app, rx_welcome_from_core) = unbounded();
        let (tx_key_to_transit, rx_key_from_transit) = unbounded();
        let (tx_code_to_app, rx_code_from_core) = unbounded();
        let (tx_verifier_to_app, rx_verifier_from_core) = unbounded();
        let (tx_versions_to_app, rx_versions_from_core) = unbounded();
        let (tx_close_to_app, rx_close_from_core) = unbounded();

        let io = crate::io::AsyncStdIO::new(tx_io_to_core);
        let mut cw = CoreWrapper {
            core: WormholeCore::new(appid, relay_url, io),
            rx_api_to_core,
            rx_io_to_core,
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
        async_std::task::block_on(tx_api_to_core.send(APIEvent::Start)).unwrap();

        Wormhole {
            code: None,
            key: None,
            welcome: None,
            versions: None,
            verifier: None,
            tx_api_to_core,
            rx_messages_from_core,
            rx_welcome_from_core,
            rx_key_from_transit,
            rx_code_from_core,
            rx_verifier_from_core,
            rx_versions_from_core,
            rx_close_from_core,
        }
    }

    pub async fn set_code(&mut self, code: &str) {
        // TODO this should wait until the code is actually set
        use futures::SinkExt;
        self.tx_api_to_core
            .send(APIEvent::SetCode(Code(code.to_string())))
            .await
            .unwrap();
    }

    pub async fn allocate_code(&mut self, num_words: usize) {
        use futures::SinkExt;
        self.tx_api_to_core
            .send(APIEvent::AllocateCode(num_words))
            .await
            .unwrap();
    }

    pub async fn send_message(&mut self, msg: &[u8]) {
        use futures::SinkExt;
        self.tx_api_to_core
            .send(APIEvent::Send(msg.to_vec()))
            .await
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
        use futures::SinkExt;
        self.tx_api_to_core
            .send(APIEvent::Close)
            .await
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

    pub fn derive_transit_key2(key: &Vec<u8>, appid: &str) -> Vec<u8> {
        let transit_purpose = format!("{}/transit-key", appid);

        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        let derived_key = derive_key(key, &transit_purpose.as_bytes(), length);
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
                self.send_message(PeerMessage::new_offer_message(&text).serialize().as_bytes()).await;
                debug!("sent..");
                // if we close right away, we won't actually send anything. Wait for at
                // least the verifier to be printed, that ought to give our outbound
                // message a chance to be delivered.
                // TODO this should not be required. If send_message ought to be blocking, the message should be sent
                // once it returns.
                let verifier = self.get_verifier().await;
                trace!("verifier: {}", hex::encode(verifier));
                trace!("got verifier, closing..");
                self.close().await;
                trace!("closed");
            },
            MessageType::File{filename, filesize} => {
                todo!();
                // filetransfer::send_file(self, &filename, app_id, relay_url).await?;
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
                    self.send_message(PeerMessage::new_message_ack("ok").serialize().as_bytes()).await;
                    msg
                }
                OfferType::File { .. } => {
                    debug!("Received file offer {:?}", offer);
                    // TODO: We are doing file_ack without asking user
                    self.send_message(PeerMessage::new_file_ack("ok").serialize().as_bytes()).await;
                    "".to_string()
                }
                OfferType::Directory { .. } => {
                    debug!("Received directory offer: {:?}", offer);
                    // TODO: We are doing file_ack without asking user
                    self.send_message(PeerMessage::new_file_ack("ok").serialize().as_bytes()).await;
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
                todo!();
                // async_std::task::block_on(filetransfer::receive_file(self, transit, app_id, relay_url))?;
                "".to_string()
            }
        };
        debug!("closing..");
        self.close().await;
        debug!("closed");

        //let remote_msg = "foobar".to_string();
        Ok(remote_msg)
    }
}

fn derive_key_from_purpose(key: &[u8], purpose: &str) -> Vec<u8> {
    let length = sodiumoxide::crypto::secretbox::KEYBYTES;
    derive_key(key, &purpose.as_bytes(), length)
}
