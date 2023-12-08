//! In reality, there is no one "Magic Wormhole" protocol. What makes Wormhole work is a handful of different protocols
//! and handshakes, layered on another and weaved together. This allows other applications to build upon the parts they want
//! and then add new ones special to their needs.
//!
//! At the core, there is a rendezvous server with a message box that allows clients to connect to and perform a PAKE.
//! Protocol wise, this is split into the "client-server" part (connect to a server, allocate nameplates, send and receive messages)
//! and a "client-client" part (do a key exchange).
//!
//! Two clients that are connected to each other need to know beforehand how to communicate with each other once the connection is established.
//! This why they have an [`AppID`]. The protocol they use to talk to each other is bound to the AppID. Clients with different AppIDs cannot communicate.
//!
//! Magic Wormhole is known for its ability to transfer files. This is implemented in the [`transfer`] module, which builds upon the wormhole
//! protocol and thus requires a [`Wormhole`].
//!
//! As an alternative to file transfer, there is the [`forwarding`] module, which allows to forward arbitrary TCP connections over the Wormhole/Transit tunnel.
//!
//! Transferring large amounts of data should not be done over the rendezvous server. Instead, you have to set up a [`transit`]
//! connection. A transit is little more than an encrypted TcpConnection. If a direct connection between both clients is not possible,
//! a relay server will transparently connect them together. Transit is used by the file transfer for example, but any other AppID protocol
//! might make use of it as well.

#![forbid(unsafe_code)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::too_many_arguments)]

#[macro_use]
mod util;
pub mod core;
#[cfg(feature = "dilation")]
pub mod dilated_transfer;
#[cfg(feature = "dilation")]
pub mod dilation;
#[cfg(feature = "forwarding")]
pub mod forwarding;
#[cfg(feature = "transfer")]
pub mod transfer;
#[cfg(feature = "transit")]
pub mod transit;
#[cfg(feature = "transfer")]
pub mod uri;

pub use crate::core::{
    key::{GenericKey, Key, KeyPurpose, WormholeKey},
    rendezvous,
    wordlist::PgpWordList,
    AppConfig, AppID, Code, MailboxConnection, Mood, Nameplate, Wormhole, WormholeError,
};

#[cfg(feature = "dilation")]
pub use crate::dilation::DilatedWormhole;
