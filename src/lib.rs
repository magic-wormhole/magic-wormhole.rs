//! In reality, there is no "Magic Wormhole" protocol. What makes Wormhole work is a handful of different protocols
//! and handshakes, layered on another and weaved together. This allows other applications to build upon the parts they want
//! and then add new ones special to their needs.
//! 
//! At the core, there is a rendezvous server with a message box that allows clients to connect to and perform a PAKE.
//! Protocol wise, this is split into the "client-server" part (connect to a server, allocate nameplates, send and receive messages)
//! and a "client-client" part (do a key exchange). Code wise, both are implemented in the [`core`] module. There is a "sane" API
//! wrapping them in the root module (see the methods below). Use it. It'll guide you through the handshakes and provide a channel pair
//! to exchange encrypted messages with the other side.
//! 
//! Two clients that are connected to each other need to know beforehand how to communicate with each other once the connection is established.
//! This why they have an [`AppID`]. The protocol they use to talk to each other is bound to the AppID. Clients with different AppIDs cannot communicate.
//! 
//! Magic wormhole is known for its ability to transfer files. This is implemented in the [`transfer`] module.
//! 
//! Transferring large amounts of data should not be done over the rendezvous server. Instead, you have to set up a [`transit`]
//! connection. A transit is little more than an encrypted TcpConnection. If a direct connection between both clients is not possible,
//! a relay server will transparently connect them together. Transit is used by the file transfer for example, but any other AppID protocol
//! might make use of it as well.

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

/// Set a code, or allocate one
#[non_exhaustive]
pub enum CodeProvider {
    /// Allocate a code with n random words
    AllocateCode(usize),
    /// Set a fixed code
    SetCode(String),
}

impl Default for CodeProvider {
    fn default() -> Self {
        CodeProvider::AllocateCode(2)
    }
}

/// Marker trait to give keys a "purpose"
// TODO Once const generics are stabilized, try out if a const string generic may replace this.
pub trait KeyPurpose {}

/// The type of main key of the Wormhole
pub struct WormholeKey;
impl KeyPurpose for WormholeKey {}

/// A generic key purpose for ad-hoc subkeys or if you don't care.
pub struct GenericKey;
impl KeyPurpose for GenericKey {}

/**
 * The symmetric encryption key used to communicate with the other side.
 *
 * You don't need to do any crypto, but you might need it to derive subkeys for sub-protocols.
 */
#[derive(Clone, Debug)]
pub struct Key<P: KeyPurpose>(pub Vec<u8>, std::marker::PhantomData<P>);

impl<P: KeyPurpose> Display for Key<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Debug;
        self.0.fmt(f)
    }
}

impl<P: KeyPurpose> std::ops::Deref for Key<P> {
    type Target = Vec<u8>;

    /// Dereferences the value.
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Key<WormholeKey> {
    /**
     * Derive the sub-key used for transit
     * 
     * This one's a bit special, since the Wormhole's AppID is included in the purpose. Different kinds of applications
     * can't talk to each other, not even accidentally, by design.
     * 
     * The new key is derived with the `"{appid}/transit-key"` purpose.
     */
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

impl<P: KeyPurpose> Key<P> {
    /**
     * Derive a new sub-key from this one
     */
    pub fn derive_subkey_from_purpose<NewP: KeyPurpose>(&self, purpose: &str) -> Key<NewP> {
        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        Key(
            derive_key(&*self, &purpose.as_bytes(), length),
            std::marker::PhantomData,
        )
    }
}

/**
 * A partial Wormhole connection
 * 
 * A value of this struct represents a Wormhole that has done the server handshake, but is not connected to any
 * "other side" client yet.
 */
pub struct WormholeConnector {
    queued_messages: Vec<APIAction>,
    /* In case we got that too early */
    key: Option<Vec<u8>>,
    appid: AppID,
    tx_api_to_core: UnboundedSender<APIEvent>,
    rx_api_from_core: UnboundedReceiver<APIAction>,
}

impl WormholeConnector {
    /**
     * Connect to the other side client
     * 
     * This will perform the PAKE key exchange and all other necessary things to fully connect.
     * It returns a [`Wormhole`] struct over which you can send and receive byte messages with the other side.
     */
    // TODO why doesn't this return a `Result`? We are surely missing some error handling here!
    pub async fn connect_to_client(mut self) -> Wormhole {
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

/**
 * The connected Wormhole object
 *
 * You can send and receive arbitrary messages in form of byte slices over it. Everything else
 * (including encryption) will be handled for you.
 *
 * # Clean shutdown
 *
 * Closing the sender will send a close event to the server and shut down the event loop. After this, the
 * receiver gets disconnected (returning `None`). It is illegal to close the receiver end before the sender and
 * will result in a panic on the event loop. You can also simply drop the struct as a whole, but you might miss
 * some pending events that way.
 */
pub struct Wormhole {
    pub tx:
        Pin<Box<dyn Sink<Vec<u8>, Error = futures::channel::mpsc::SendError> + std::marker::Send>>,
    pub rx: Pin<Box<dyn Stream<Item = Vec<u8>> + std::marker::Send>>,
    pub key: Key<WormholeKey>,
    pub appid: AppID,
}

/**
 * The result of the client-server handshake
 */
pub struct WormholeWelcome {
    pub code: Code,
    /** A welcome message from the server (think of "message of the day"). Display it to the user if you wish. */
    pub welcome: String,
}

/**
 * Do the first part of the connection setup
 *
 * This establishes a connection to the mailbox server and handles setting or allocating a code.
 * It will spawn an event loop task that will run for the whole lifetime of the Wormhole.
 *
 * This method returns a [`WormholeWelcome`] containing the server initialization result and a [`WormholeConnector`]
 * that can be used to finish the other part of the handshake. Dropping the connector closes the connection.
 */
pub async fn connect_to_server(
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
