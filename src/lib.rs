#![forbid(unsafe_code)]
// #![deny(warnings)]

pub mod core;
pub mod transfer;
pub mod transit;

use crate::core::AppID;
use crate::core::{APIAction, APIEvent, Code};
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use futures::Sink;
use futures::Stream;
use std::fmt::Display;
use std::pin::Pin;

use crate::core::key::derive_key;
use log::*;
use std::str;

/// Some mailbox server you might use.
///
/// Two applications that want to communicate with each other *must* use the same mailbox server.
pub const DEFAULT_MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";

#[non_exhaustive]
pub enum CodeProvider {
    AllocateCode(usize),
    SetCode(String),
}

impl Default for CodeProvider {
    fn default() -> Self {
        CodeProvider::AllocateCode(2)
    }
}

// TODO Once const generics are stabilized, try out if a const string generic may replace this.
pub trait KeyPurpose {}

pub struct WormholeKey;
impl KeyPurpose for WormholeKey {}
pub struct GenericKey;
impl KeyPurpose for GenericKey {}

/**
 * The symmetric encryption key used to communicate with the other side.
 * 
 * You don't need to do any crypto, but you might need it to derive subkeys for sub-protocols.
 */
#[derive(Clone, Debug)]
pub struct Key<P: KeyPurpose>(pub Vec<u8>, std::marker::PhantomData<P>);

impl <P: KeyPurpose> Display for Key<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Debug;
        self.0.fmt(f)
    }
}

impl <P: KeyPurpose> std::ops::Deref for Key<P> {
    type Target=Vec<u8>;

    /// Dereferences the value.
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Key<WormholeKey> {
    pub fn derive_transit_key(&self, appid: &AppID) -> Key<transit::TransitKey> {
        let transit_purpose = format!("{}/transit-key", &**appid);

        let derived_key = self.derive_subkey_from_purpose(&transit_purpose);
        trace!(
            "Input key: {}, Transit key: {}, Transit purpose: '{}'",
            hex::encode(&**self),
            hex::encode(&**derived_key),
            &transit_purpose
        );
        derived_key
    }
}

impl <P: KeyPurpose> Key<P> {
    fn derive_subkey_from_purpose<NewP: KeyPurpose>(&self, purpose: &str) -> Key<NewP> {
        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        Key(derive_key(&*self, &purpose.as_bytes(), length), std::marker::PhantomData)
    }
}

pub struct WormholeConnector {
    queued_messages: Vec<APIAction>,
    /* In case we got that too early */
    key: Option<Vec<u8>>,
    appid: AppID,
    tx_api_to_core: UnboundedSender<APIEvent>,
    rx_api_from_core: UnboundedReceiver<APIAction>,
}

impl WormholeConnector {
    pub async fn connect_2(mut self) -> Wormhole {
        use futures::SinkExt;
        use futures::StreamExt;

        let mut verifier = None;
        let mut versions = None;

        if self.key.is_none() {
            while let Some(action) = self.rx_api_from_core.next().await {
                use self::APIAction::*;
                match action {
                    GotUnverifiedKey(k) => {
                        debug!("Got key");
                        self.key = Some(k.0.clone());
                    }
                    GotVerifier(v) => {
                        debug!("Got verifier {:x?}", &v);
                        verifier = Some(v);
                    }
                    GotVersions(v) => {
                        debug!("Got version");
                        versions = Some(v);
                    }
                    action @ GotMessage(_) => {
                        warn!("Got message from other side during initialization. Will deliver it after initialization.");
                        self.queued_messages.push(action);
                    }
                    GotWelcome(_) | GotCode(_) => {
                        panic!("TODO I don't want this what is this?!");
                    }
                    GotClosed(mood) => {
                        panic!("TODO return error");
                    }
                }

                if self.key.is_some() {
                    /* We know enough */
                    break;
                }
            }
        }

        let tx_api_to_core = self.tx_api_to_core.with(|message| async {
            Result::<_, futures::channel::mpsc::SendError>::Ok(APIEvent::Send(message))
        });
        let rx_api_from_core = futures::stream::iter(self.queued_messages)
            .chain(self.rx_api_from_core)
            .filter_map(|action| async {
                match action {
                    APIAction::GotMessage(m) => Some(m),
                    APIAction::GotClosed(_) => {
                        todo!("close streams");
                    }
                    action => {
                        warn!(
                            "Received unexpected action after initialization: '{:?}'",
                            action
                        );
                        // todo!("error handling"); // TODO
                        None
                    }
                }
            });

        Wormhole {
            tx: Box::pin(tx_api_to_core),
            rx: Box::pin(rx_api_from_core),
            key: Key(self.key.unwrap(), std::marker::PhantomData),
            appid: self.appid,
        }
    }
}

pub struct Wormhole {
    pub tx:
        Pin<Box<dyn Sink<Vec<u8>, Error = futures::channel::mpsc::SendError> + std::marker::Send>>,
    pub rx: Pin<Box<dyn Stream<Item = Vec<u8>> + std::marker::Send>>,
    pub key: Key<WormholeKey>,
    pub appid: AppID,
}

pub struct WormholeWelcome {
    pub code: Code,
    pub welcome: String,
}

pub async fn connect_1(
    appid: impl Into<String>,
    relay_url: &str,
    code_provider: CodeProvider,
    #[cfg(test)] eventloop_task: &mut Option<async_std::task::JoinHandle<()>>,
) -> (WormholeWelcome, WormholeConnector) {
    let appid: AppID = AppID::new(appid);
    let (tx_api_to_core, mut rx_api_from_core) = {
        #[cfg(test)]
        {
            crate::core::run(appid.clone(), relay_url, eventloop_task)
        }
        #[cfg(not(test))]
        {
            crate::core::run(appid.clone(), relay_url)
        }
    };

    let mut code = None;
    let mut welcome = None;
    let mut verifier = None;
    let mut versions = None;
    let mut key = None;
    let mut queued_messages = Vec::new();

    tx_api_to_core.unbounded_send(APIEvent::Start).unwrap();

    match code_provider {
        CodeProvider::AllocateCode(num_words) => {
            tx_api_to_core
                .unbounded_send(APIEvent::AllocateCode(num_words))
                .unwrap();
        }
        CodeProvider::SetCode(code) => {
            tx_api_to_core
                .unbounded_send(APIEvent::SetCode(Code(code)))
                .unwrap();
        }
    }

    use futures::StreamExt;

    while let Some(action) = rx_api_from_core.next().await {
        use self::APIAction::*;
        match action {
            GotWelcome(w) => {
                debug!("Got welcome");
                welcome = Some(w.to_string());
            }
            action @ GotMessage(_) => {
                warn!("Got message from other side during initialization. Will deliver it after initialization.");
                queued_messages.push(action);
            }
            GotCode(c) => {
                debug!("Got code");
                code = Some(c.clone());
            }
            GotUnverifiedKey(k) => {
                /* This shouldn't happen now, but it might */
                debug!("Got key");
                key = Some(k.0.clone());
            }
            GotVerifier(v) => {
                debug!("Got verifier {:x?}", &v);
                verifier = Some(v);
            }
            GotVersions(v) => {
                debug!("Got version");
                versions = Some(v);
            }
            GotClosed(mood) => {
                panic!("TODO return error");
            }
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
            appid,
        },
    )
}