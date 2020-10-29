#![forbid(unsafe_code)]
// #![deny(warnings)]

pub mod core;
pub mod filetransfer;
pub mod transit;

use std::fmt::Display;
use std::pin::Pin;
use crate::core::AppID;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use futures::Sink;
use futures::Stream;
use crate::core::{
    APIAction, APIEvent, Code,
};

use crate::core::key::derive_key;
use std::str;
use log::*;

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

#[derive(Clone, Debug,)]
pub struct WormholeKey {
    key: Vec<u8>,
    // appid: AppID,
}

impl Display for WormholeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Debug;
        self.key.fmt(f)
    }
}

impl WormholeKey {
    pub fn derive_transit_key(&self) -> Vec<u8> {
        // let transit_purpose = format!("{}/transit-key", &self.appid.0);

        // let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        // let derived_key = derive_key(&self.key, &transit_purpose.as_bytes(), length);
        // trace!("Input key: {}, Transit key: {}, Transit purpose: '{}'", hex::encode(&self.key), hex::encode(&derived_key), &transit_purpose);
        // derived_key
        todo!()
    }
}

pub struct WormholeConnector {
    queued_messages: Vec<APIAction>,
    /* In case we got that too early */
    key: Option<Vec<u8>>,
    tx_api_to_core: UnboundedSender<APIEvent>,
    rx_api_from_core: UnboundedReceiver<APIAction>,
}

impl WormholeConnector {
    pub async fn connect_2(mut self) -> Wormhole {
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
                        debug!("Got verifier {:x?}", &v);
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

        Wormhole {
            tx: Box::pin(tx_api_to_core),
            rx: Box::pin(rx_api_from_core),
            key: WormholeKey {
                key: self.key.unwrap(),
            },
        }
    }
}

pub struct Wormhole {
    pub tx: Pin<Box<dyn Sink<Vec<u8>, Error = futures::channel::mpsc::SendError>>>,
    pub rx: Pin<Box<dyn Stream<Item = Vec<u8>>>>,
    pub key: WormholeKey,
}

pub struct WormholeWelcome {
    pub code: Code,
    pub welcome: String,
}

pub async fn connect_1(appid: &str, relay_url: &str, code_provider: CodeProvider, )
-> (
    WormholeWelcome,
    WormholeConnector,
) {
    use futures::StreamExt;

    let (tx_api_to_core, mut rx_api_from_core) = crate::core::run(appid, relay_url);

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
                debug!("Got verifier {:x?}", &v);
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
        WormholeWelcome {
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

#[deprecated]
fn derive_key_from_purpose(key: &[u8], purpose: &str) -> Vec<u8> {
    let length = sodiumoxide::crypto::secretbox::KEYBYTES;
    derive_key(key, &purpose.as_bytes(), length)
}
