//! Client-to-Client protocol to organize file transfers
//!
//! This gives you the actual capability to transfer files, that feature that Magic Wormhole got known and loved for.
//!
//! It is bound to an [`APPID`](APPID). Only applications using that APPID (and thus this protocol) can interoperate with
//! the original Python implementation (and other compliant implementations).
//!
//! At its core, "peer messages" are exchanged over an established wormhole connection with the other side.
//! They are used to set up a [transit] portal and to exchange a file offer/accept. Then, the file is transmitted over the transit relay.

use futures::{AsyncRead, AsyncSeek, AsyncWrite};
use serde_derive::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::sync::Arc;

use super::{core::WormholeError, transit, AppID, Wormhole};
use futures::Future;
use log::*;
use std::{borrow::Cow, collections::BTreeMap};

#[cfg(not(target_family = "wasm"))]
use std::path::{Path, PathBuf};

use transit::{
    Abilities as TransitAbilities, Transit, TransitConnectError, TransitConnector, TransitError,
};

mod cancel;
#[doc(hidden)]
pub mod offer;
mod v1;
#[cfg(feature = "experimental-transfer-v2")]
mod v2;

#[doc(hidden)]
pub use v1::ReceiveRequest as ReceiveRequestV1;

#[cfg(not(feature = "experimental-transfer-v2"))]
pub use v1::ReceiveRequest;

#[cfg(feature = "experimental-transfer-v2")]
pub use v2::ReceiveRequest as ReceiveRequestV2;

const APPID_RAW: &str = "lothar.com/wormhole/text-or-file-xfer";

/// The App ID associated with this protocol.
pub const APPID: AppID = AppID(Cow::Borrowed(APPID_RAW));

/// An [`crate::AppConfig`] with sane defaults for this protocol.
///
/// You **must not** change `id` and `rendezvous_url` to be interoperable.
/// The `app_version` can be adjusted if you want to disable some features.
pub const APP_CONFIG: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(Cow::Borrowed(APPID_RAW)),
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion::new(),
};

// TODO be more extensible on the JSON enum types (i.e. recognize unknown variants)

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransferError {
    #[error("Transfer was not acknowledged by peer")]
    AckError,
    #[error("Receive checksum error")]
    Checksum,
    #[error("The file contained a different amount of bytes than advertized! Sent {} bytes, but should have been {}", sent_size, file_size)]
    FileSize { sent_size: u64, file_size: u64 },
    #[error("The file(s) to send got modified during the transfer, and thus corrupted")]
    FilesystemSkew,
    // TODO be more specific
    #[error("Unsupported offer type")]
    UnsupportedOffer,
    #[error("Something went wrong on the other side: {}", _0)]
    PeerError(String),

    /// Some deserialization went wrong, we probably got some garbage
    #[error("Corrupt JSON message received")]
    ProtocolJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
    #[error("Corrupt Msgpack message received")]
    ProtocolMsgpack(
        #[from]
        #[source]
        rmp_serde::decode::Error,
    ),
    /// A generic string message for "something went wrong", i.e.
    /// the server sent some bullshit message order
    #[error("Protocol error: {}", _0)]
    Protocol(Box<str>),
    #[error(
        "Unexpected message (protocol error): Expected '{}', but got: '{}'",
        _0,
        _1
    )]
    ProtocolUnexpectedMessage(Box<str>, Box<str>),
    #[error("Wormhole connection error")]
    Wormhole(
        #[from]
        #[source]
        WormholeError,
    ),
    #[error("Error while establishing transit connection")]
    TransitConnect(
        #[from]
        #[source]
        TransitConnectError,
    ),
    #[error("Transit error")]
    Transit(
        #[from]
        #[source]
        TransitError,
    ),
    #[error("IO error")]
    IO(
        #[from]
        #[source]
        std::io::Error,
    ),
}

impl TransferError {
    pub(self) fn unexpected_message(
        expected: impl Into<Box<str>>,
        got: impl std::fmt::Display,
    ) -> Self {
        Self::ProtocolUnexpectedMessage(expected.into(), got.to_string().into())
    }
}

/**
 * The application specific version information for this protocol.
 */
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppVersion {
    #[serde(default)]
    abilities: Cow<'static, [Cow<'static, str>]>,
    #[serde(default)]
    #[cfg(feature = "experimental-transfer-v2")]
    transfer_v2: Option<AppVersionTransferV2Hint>,
}

// TODO check invariants during deserialization
impl AppVersion {
    const fn new() -> Self {
        Self {
            // Dont advertize v2 for now
            abilities: Cow::Borrowed(&[
                Cow::Borrowed("transfer-v1"), /* Cow::Borrowed("experimental-transfer-v2") */
            ]),
            #[cfg(feature = "experimental-transfer-v2")]
            transfer_v2: Some(AppVersionTransferV2Hint::new()),
        }
    }

    #[allow(dead_code)]
    fn supports_v2(&self) -> bool {
        self.abilities.contains(&"transfer-v2".into())
    }
}

impl Default for AppVersion {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "experimental-transfer-v2")]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppVersionTransferV2Hint {
    supported_formats: Cow<'static, [Cow<'static, str>]>,
    transit_abilities: transit::Abilities,
}

#[cfg(feature = "experimental-transfer-v2")]
impl AppVersionTransferV2Hint {
    const fn new() -> Self {
        Self {
            supported_formats: Cow::Borrowed(&[Cow::Borrowed("plain"), Cow::Borrowed("tar")]),
            transit_abilities: transit::Abilities::ALL_ABILITIES,
        }
    }
}

#[cfg(feature = "experimental-transfer-v2")]
impl Default for AppVersionTransferV2Hint {
    fn default() -> Self {
        Self::new()
    }
}

pub trait AsyncReadSeek: AsyncRead + AsyncSeek {}

impl<T> AsyncReadSeek for T where T: AsyncRead + AsyncSeek {}

pub trait AsyncWriteSeek: AsyncWrite + AsyncSeek {}

impl<T> AsyncWriteSeek for T where T: AsyncWrite + AsyncSeek {}

/**
 * The type of message exchanged over the wormhole for this protocol
 */
#[derive(Deserialize, Serialize, derive_more::Display, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PeerMessage {
    /* V1 */
    #[display(fmt = "transit")]
    Transit(v1::TransitV1),
    #[display(fmt = "offer")]
    Offer(v1::OfferMessage),
    #[display(fmt = "answer")]
    Answer(v1::AnswerMessage),
    /* V2 */
    #[cfg(feature = "experimental-transfer-v2")]
    #[display(fmt = "transit-v2")]
    TransitV2(v2::TransitV2),

    /** Tell the other side you got an error */
    #[display(fmt = "error")]
    Error(String),
    #[display(fmt = "unknown")]
    #[serde(other)]
    Unknown,
}

impl PeerMessage {
    #[allow(unused)]
    fn offer_message_v1(msg: impl Into<String>) -> Self {
        PeerMessage::Offer(v1::OfferMessage::Message(msg.into()))
    }

    fn offer_file_v1(name: impl Into<String>, size: u64) -> Self {
        PeerMessage::Offer(v1::OfferMessage::File {
            filename: name.into(),
            filesize: size,
        })
    }

    #[allow(dead_code)]
    fn offer_directory_v1(
        name: impl Into<String>,
        mode: impl Into<String>,
        compressed_size: u64,
        numbytes: u64,
        numfiles: u64,
    ) -> Self {
        PeerMessage::Offer(v1::OfferMessage::Directory {
            dirname: name.into(),
            mode: mode.into(),
            zipsize: compressed_size,
            numbytes,
            numfiles,
        })
    }

    #[allow(dead_code)]
    fn message_ack_v1(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(v1::AnswerMessage::MessageAck(msg.into()))
    }

    fn file_ack_v1(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(v1::AnswerMessage::FileAck(msg.into()))
    }

    fn error_message(msg: impl Into<String>) -> Self {
        PeerMessage::Error(msg.into())
    }

    fn transit_v1(abilities: TransitAbilities, hints: transit::Hints) -> Self {
        PeerMessage::Transit(v1::TransitV1 {
            abilities_v1: abilities,
            hints_v1: hints,
        })
    }

    #[cfg(feature = "experimental-transfer-v2")]
    fn transit_v2(hints_v2: transit::Hints) -> Self {
        PeerMessage::TransitV2(v2::TransitV2 { hints_v2 })
    }

    fn check_err(&self) -> Result<Self, TransferError> {
        match self {
            Self::Error(err) => Err(TransferError::PeerError(err.clone())),
            other => Ok(other.clone()),
        }
    }

    #[allow(dead_code)]
    fn ser_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}

/// Send a previously constructed offer.
///
/// Part of the experimental and unstable transfer-v2 API.
/// Expect some amount of API breakage in the future to adapt to protocol changes and API ergonomics.
#[cfg_attr(not(feature = "experimental-transfer-v2"), doc(hidden))]
pub async fn send(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    offer: offer::OfferSend,
    transit_handler: impl FnOnce(transit::TransitInfo),
    progress_handler: impl FnMut(u64, u64) + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError> {
    let peer_version: AppVersion = serde_json::from_value(wormhole.peer_version().clone())?;

    #[cfg(feature = "experimental-transfer-v2")]
    {
        if peer_version.supports_v2() {
            return v2::send(
                wormhole,
                relay_hints,
                transit_abilities,
                offer,
                progress_handler,
                peer_version,
                cancel,
            )
            .await;
        }
    }

    v1::send(
        wormhole,
        relay_hints,
        transit_abilities,
        offer,
        progress_handler,
        transit_handler,
        peer_version,
        cancel,
    )
    .await
}

/**
 * Wait for a file offer from the other side
 *
 * This method waits for an offer message and builds up a [`ReceiveRequest`](ReceiveRequest).
 * It will also start building a TCP connection to the other side using the transit protocol.
 *
 * Returns `None` if the task got cancelled.
 *
 * Part of the experimental and unstable transfer-v2 API.
 * Expect some amount of API breakage in the future to adapt to protocol changes and API ergonomics.
 */
#[cfg(feature = "experimental-transfer-v2")]
pub async fn request(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    cancel: impl Future<Output = ()>,
) -> Result<Option<ReceiveRequest>, TransferError> {
    #[cfg(feature = "experimental-transfer-v2")]
    {
        let peer_version: AppVersion = serde_json::from_value(wormhole.peer_version().clone())?;
        if peer_version.supports_v2() {
            v2::request(
                wormhole,
                relay_hints,
                peer_version,
                transit_abilities,
                cancel,
            )
            .await
            .map(|req| req.map(ReceiveRequest::V2))
        } else {
            v1::request(wormhole, relay_hints, transit_abilities, cancel)
                .await
                .map(|req| req.map(ReceiveRequest::V1))
        }
    }
}

/// Wait for a file offer from the other side
///
/// This method waits for an offer message and builds up a ReceiveRequest. It will also start building a TCP connection to the other side using the transit protocol.
///
/// Returns None if the task got cancelled.
#[cfg_attr(
    feature = "experimental-transfer-v2",
    deprecated(
        since = "0.7.0",
        note = "transfer::request_file does not support file transfer protocol version 2.
        To continue only supporting version 1, use transfer::v1::request. To support both protocol versions, use transfer::request"
    )
)]
pub async fn request_file(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    cancel: impl Future<Output = ()>,
) -> Result<Option<v1::ReceiveRequest>, TransferError> {
    v1::request(wormhole, relay_hints, transit_abilities, cancel).await
}

/// Send a file to the other side
///
/// You must ensure that the Reader contains exactly as many bytes as advertized in file_size.
///
/// This API will be deprecated in the future.
#[cfg_attr(
    feature = "experimental-transfer-v2",
    deprecated(
        since = "0.7.0",
        note = "transfer::send_file does not support file transfer protocol version 2, use transfer::send"
    )
)]
#[cfg(not(target_family = "wasm"))]
pub async fn send_file<F, N, G, H>(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    file: &mut F,
    file_name: N,
    file_size: u64,
    transit_abilities: transit::Abilities,
    transit_handler: G,
    progress_handler: H,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError>
where
    F: AsyncRead + Unpin + Send,
    N: Into<PathBuf>,
    G: FnOnce(transit::TransitInfo),
    H: FnMut(u64, u64) + 'static,
{
    v1::send_file(
        wormhole,
        relay_hints,
        file,
        file_name.into().to_string_lossy(),
        file_size,
        transit_abilities,
        transit_handler,
        progress_handler,
        cancel,
    )
    .await
}

/// Send a file or folder
#[cfg_attr(
    feature = "experimental-transfer-v2",
    deprecated(
        since = "0.7.0",
        note = "transfer::send_file_or_folder does not support file transfer protocol version 2, use transfer::send"
    )
)]
#[allow(deprecated)]
#[cfg(not(target_family = "wasm"))]
pub async fn send_file_or_folder<N, M, G, H>(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    file_path: N,
    file_name: M,
    transit_abilities: transit::Abilities,
    transit_handler: G,
    progress_handler: H,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError>
where
    N: AsRef<Path>,
    M: AsRef<Path>,
    G: FnOnce(transit::TransitInfo),
    H: FnMut(u64, u64) + 'static,
{
    use async_std::fs::File;
    let file_path = file_path.as_ref();
    let file_name = file_name.as_ref();

    let mut file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    if metadata.is_dir() {
        send_folder(
            wormhole,
            relay_hints,
            file_path,
            file_name,
            transit_abilities,
            transit_handler,
            progress_handler,
            cancel,
        )
        .await?;
    } else {
        let file_size = metadata.len();
        send_file(
            wormhole,
            relay_hints,
            &mut file,
            file_name,
            file_size,
            transit_abilities,
            transit_handler,
            progress_handler,
            cancel,
        )
        .await?;
    }
    Ok(())
}

/// Send a folder to the other side
/// This isn’t a proper folder transfer as per the Wormhole protocol because it sends it in a way so
/// that the receiver still has to manually unpack it. But it’s better than nothing
#[cfg_attr(
    feature = "experimental-transfer-v2",
    deprecated(
        since = "0.7.0",
        note = "transfer::send_folder does not support file transfer protocol version 2, use transfer::send"
    )
)]
#[cfg(not(target_family = "wasm"))]
pub async fn send_folder<N, M, G, H>(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    folder_path: N,
    folder_name: M,
    transit_abilities: transit::Abilities,
    transit_handler: G,
    progress_handler: H,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError>
where
    N: Into<PathBuf>,
    M: Into<PathBuf>,
    G: FnOnce(transit::TransitInfo),
    H: FnMut(u64, u64) + 'static,
{
    let offer = offer::OfferSendEntry::new(folder_path.into()).await?;

    v1::send_folder(
        wormhole,
        relay_hints,
        folder_name.into().to_string_lossy().to_string(),
        offer,
        transit_abilities,
        transit_handler,
        progress_handler,
        cancel,
    )
    .await
}

/**
 * A pending files send offer from the other side
 *
 * You *should* consume this object, by matching on the protocol version and then calling either `accept` or `reject`.
 */
#[must_use]
#[cfg(feature = "experimental-transfer-v2")]
pub enum ReceiveRequest {
    V1(ReceiveRequestV1),
    V2(ReceiveRequestV2),
}

#[cfg(feature = "experimental-transfer-v2")]
impl ReceiveRequest {
    pub async fn accept<F, G, W>(
        self,
        transit_handler: G,
        progress_handler: F,
        mut answer: offer::OfferAccept,
        cancel: impl Future<Output = ()>,
    ) -> Result<(), TransferError>
    where
        F: FnMut(u64, u64) + 'static,
        G: FnOnce(transit::TransitInfo),
        W: AsyncWrite + Unpin,
    {
        match self {
            ReceiveRequest::V1(request) => {
                // Desynthesize the previously synthesized offer to make transfer v1 more similar to transfer v2
                let (_name, entry) = answer.content.pop_first().expect(
                    "must call accept(..) with an offer that contains at least one element",
                );

                let mut acceptor = match entry {
                    offer::OfferEntry::RegularFile { content, .. } => {
                        (content.content)(true).await?
                    },
                    _ => panic!(
                        "when using transfer v1 you must call accept(..) with file offers only",
                    ),
                };

                request
                    .accept(transit_handler, progress_handler, &mut acceptor, cancel)
                    .await
            },
            ReceiveRequest::V2(request) => {
                request
                    .accept(transit_handler, answer, progress_handler, cancel)
                    .await
            },
        }
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     */
    pub async fn reject(self) -> Result<(), TransferError> {
        match self {
            ReceiveRequest::V1(request) => request.reject().await,
            ReceiveRequest::V2(request) => request.reject().await,
        }
    }

    pub fn offer(&self) -> Arc<offer::Offer> {
        match self {
            ReceiveRequest::V1(req) => req.offer(),
            ReceiveRequest::V2(req) => req.offer(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use transit::{Abilities, DirectHint, RelayHint};

    #[test]
    fn test_transit() {
        let abilities = Abilities::ALL_ABILITIES;
        let hints = transit::Hints::new(
            [DirectHint::new("192.168.1.8", 46295)],
            [RelayHint::new(
                None,
                [DirectHint::new("magic-wormhole-transit.debian.net", 4001)],
                [],
            )],
        );
        assert_eq!(
            serde_json::json!(crate::transfer::PeerMessage::transit_v1(abilities, hints)),
            serde_json::json!({
                "transit": {
                    "abilities-v1": [{"type":"direct-tcp-v1"},{"type":"relay-v1"}],
                    "hints-v1": [
                        {"hostname":"192.168.1.8","port":46295,"type":"direct-tcp-v1"},
                        {
                            "type": "relay-v1",
                            "hints": [
                                {"type": "direct-tcp-v1", "hostname": "magic-wormhole-transit.debian.net", "port": 4001}
                            ],
                            "name": null
                        }
                    ],
                }
            })
        );
    }

    #[test]
    fn test_message() {
        let m1 = PeerMessage::offer_message_v1("hello from rust");
        assert_eq!(
            serde_json::json!(m1).to_string(),
            "{\"offer\":{\"message\":\"hello from rust\"}}"
        );
    }

    #[test]
    fn test_offer_file() {
        let f1 = PeerMessage::offer_file_v1("somefile.txt", 34556);
        assert_eq!(
            serde_json::json!(f1).to_string(),
            "{\"offer\":{\"file\":{\"filename\":\"somefile.txt\",\"filesize\":34556}}}"
        );
    }

    #[test]
    fn test_offer_directory() {
        let d1 = PeerMessage::offer_directory_v1("somedirectory", "zipped", 45, 1234, 10);
        assert_eq!(
            serde_json::json!(d1).to_string(),
            "{\"offer\":{\"directory\":{\"dirname\":\"somedirectory\",\"mode\":\"zipped\",\"numbytes\":1234,\"numfiles\":10,\"zipsize\":45}}}"
        );
    }

    #[test]
    fn test_message_ack() {
        let m1 = PeerMessage::message_ack_v1("ok");
        assert_eq!(
            serde_json::json!(m1).to_string(),
            "{\"answer\":{\"message_ack\":\"ok\"}}"
        );
    }

    #[test]
    fn test_file_ack() {
        let f1 = PeerMessage::file_ack_v1("ok");
        assert_eq!(
            serde_json::json!(f1).to_string(),
            "{\"answer\":{\"file_ack\":\"ok\"}}"
        );
    }
}
