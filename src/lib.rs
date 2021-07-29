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
#![allow(clippy::upper_case_acronyms)]

mod core;
pub mod transfer;
pub mod transit;
pub mod util;

use crate::core::APIEvent;
pub use crate::core::{AppID, Code};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    Sink, Stream,
};
use std::{fmt::Display, pin::Pin};

use xsalsa20poly1305 as secretbox;

use crate::core::key::derive_key;
use log::*;
use std::str;

/// Some mailbox server you might use.
///
/// Two applications that want to communicate with each other *must* use the same mailbox server.
pub const DEFAULT_MAILBOX_SERVER: &str = "wss://relay.magic-wormhole.io:4000/v1";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WormholeError {
    #[error(transparent)]
    CoreError(#[from] core::WormholeCoreError),
}

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
#[derive(Debug, Clone)]
// TODO redact value for logs by manually deriving Debug
pub struct Key<P: KeyPurpose>(pub Box<secretbox::Key>, std::marker::PhantomData<P>);

impl<P: KeyPurpose> Display for Key<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::fmt::Debug;
        self.0.fmt(f)
    }
}

impl<P: KeyPurpose> std::ops::Deref for Key<P> {
    type Target = secretbox::Key;

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
        Key(
            Box::new(derive_key(&*self, &purpose.as_bytes())),
            std::marker::PhantomData,
        )
    }
}

/**
 * A partial Wormhole connection
 *
 * A value of this struct represents a Wormhole that has done the server handshake, but is not connected to any
 * "other side" client yet.
 *
 * Use [`connect_to_client`](WormholeConnector::connect_to_client) to finish the setup and get a fully-initialized [`Wormhole`] object.
 * Call [`cancel`](WormholeConnector::cancel) to cleanly disconnect from the server.
 */
#[must_use]
pub struct WormholeConnector {
    appid: AppID,
    tx_api_to_core: UnboundedSender<Vec<u8>>,
    rx_core_to_api: UnboundedReceiver<APIEvent>,
}

impl WormholeConnector {
    /**
     * Connect to the other side client
     *
     * This will perform the PAKE key exchange and all other necessary things to fully connect.
     * It returns a [`Wormhole`] struct over which you can send and receive byte messages with the other side.
     */
    pub async fn connect_to_client(mut self) -> Result<Wormhole, WormholeError> {
        use futures::{SinkExt, StreamExt};

        let key;
        let verifier;
        let peer_version;

        loop {
            use self::APIEvent::*;
            match self.rx_core_to_api.next().await {
                Some(ConnectedToServer { .. }) | Some(GotMessage(_)) => unreachable!(),
                Some(ConnectedToClient {
                    key: k,
                    verifier: v,
                    versions,
                }) => {
                    /* TODO is there better way to avoid variable shadowing here? (And same below) */
                    key = k;
                    verifier = v;
                    peer_version = versions;
                    break;
                },
                Some(GotError(error)) => return Err(error.into()),
                /* We will/should always get an error (above) before the connection closes */
                None => unreachable!("Wormhole unexpectedly closed"),
            }
        }

        let tx_api_to_core = self
            .tx_api_to_core
            .with(|message| async { Result::<_, futures::channel::mpsc::SendError>::Ok(message) });
        let rx_core_to_api = self.rx_core_to_api.map(|action| match action {
            APIEvent::GotMessage(m) => Ok(m),
            APIEvent::GotError(err) => Err(err),
            _ => unreachable!(),
        });

        Ok(Wormhole {
            tx: Box::pin(tx_api_to_core),
            rx: Box::pin(rx_core_to_api),
            key: Key(key, std::marker::PhantomData),
            verifier,
            peer_version,
            appid: self.appid,
        })
    }

    /// Cancel everything and close the connection
    pub async fn cancel(self) {
        use futures::StreamExt;
        self.tx_api_to_core.close_channel();
        let _ = self
            .rx_core_to_api
            .map(Result::Ok)
            .forward(futures::sink::drain())
            .await;
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
/* TODO
 * Maybe a better way to handle application level protocols is to create a trait for them and then
 * to paramterize over them.
 */
pub struct Wormhole {
    pub tx:
        Pin<Box<dyn Sink<Vec<u8>, Error = futures::channel::mpsc::SendError> + std::marker::Send>>,
    pub rx:
        Pin<Box<dyn Stream<Item = Result<Vec<u8>, core::WormholeCoreError>> + std::marker::Send>>,
    pub key: Key<WormholeKey>,
    pub verifier: Box<secretbox::Key>,
    /**
     * The `AppID` this wormhole is bound to.
     * This determines the upper-layer protocol. Only wormholes with the same value can talk to each other.
     */
    pub appid: AppID,
    /**
     * Protocol version information from the other side.
     * This is bound by the `AppID`'s protocol and thus shall be handled on a higher level.
     */
    pub peer_version: serde_json::Value,
}

impl Wormhole {
    /** The recommended way to close a Wormhole
     *
     * While you can simply drop it, that won't catch any errors, and more importantly,
     * it won't wait until the inner thread has finished. If this is the last thing your
     * program does, it will exit before any remaining messages are processed. Using `close`
     * will wait for everything and give you error messages.
     *
     * ## Panics
     *
     * If the underlying Wormhole has already been closed or shut down.
     */
    pub async fn close(mut self) -> Result<(), WormholeError> {
        use futures::{SinkExt, StreamExt};
        /* Close the sender */
        self.tx.close().await.expect("Wormhole already closed");
        /* Wait until the wormhole thread stops */
        self.rx
            .forward(futures::sink::drain::<Vec<u8>>().sink_err_into())
            .await?;
        Ok(())
    }
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
 * The `appid` and `versions` parameters are bound to the application level protocol (e.g. [`transfer`]).
 *
 * This method returns a [`WormholeWelcome`] containing the server initialization result and a [`WormholeConnector`]
 * that can be used to finish the other part of the handshake.
 */
pub async fn connect_to_server(
    appid: impl Into<String>,
    versions: impl serde::Serialize,
    relay_url: impl Into<String>,
    code_provider: CodeProvider,
    #[cfg(test)] eventloop_task: &mut Option<async_std::task::JoinHandle<()>>,
) -> Result<(WormholeWelcome, WormholeConnector), WormholeError> {
    let relay_url = relay_url.into();
    let appid: AppID = AppID::new(appid);
    let versions = serde_json::to_value(versions).expect("Could not serialize versions");
    let (tx_core_to_api, mut rx_core_to_api) = futures::channel::mpsc::unbounded();
    let (tx_api_to_core, rx_api_to_core) = futures::channel::mpsc::unbounded();

    {
        /* Spawn the main loop */
        let appid = appid.clone();
        #[allow(unused_variables)]
        let task = async_std::task::spawn(async move {
            crate::core::run(
                &appid,
                versions,
                /* TODO if we do dependency injection on WormholeIO we can elide that to_string clone */
                &relay_url,
                code_provider,
                tx_core_to_api,
                rx_api_to_core,
            ).await
        });
        #[cfg(test)]
        {
            *eventloop_task = Some(task);
        }
    }

    let code;
    let welcome;

    use futures::StreamExt;

    loop {
        use self::APIEvent::*;
        match rx_core_to_api.next().await {
            Some(ConnectedToServer {
                welcome: w,
                code: c,
            }) => {
                debug!("Got welcome");
                welcome = w;
                code = c;
                break;
            },
            Some(ConnectedToClient { .. }) | Some(GotMessage(_)) => unreachable!(),
            Some(GotError(error)) => return Err(error.into()),
            /* We will/should always get an error (above) before the connection closes */
            None => unreachable!("Wormhole unexpectedly closed"),
        }
    }

    Ok((
        WormholeWelcome {
            code,
            welcome: welcome.to_string(), // TODO don't do that
        },
        WormholeConnector {
            tx_api_to_core,
            rx_core_to_api,
            appid,
        },
    ))
}
