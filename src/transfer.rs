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
mod v1;
mod v2;

pub use v1::ReceiveRequest as ReceiveRequestV1;
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
    transfer_v2: Option<AppVersionTransferV2Hint>,
}

// TODO check invariants during deserialization
impl AppVersion {
    const fn new() -> Self {
        Self {
            // Dont advertize v2 for now
            abilities: Cow::Borrowed(&[
                Cow::Borrowed("transfer-v1"), /* Cow::Borrowed("transfer-v2") */
            ]),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppVersionTransferV2Hint {
    supported_formats: Cow<'static, [Cow<'static, str>]>,
    transit_abilities: transit::Abilities,
}

impl AppVersionTransferV2Hint {
    const fn new() -> Self {
        Self {
            supported_formats: Cow::Borrowed(&[Cow::Borrowed("plain"), Cow::Borrowed("tar")]),
            transit_abilities: transit::Abilities::ALL_ABILITIES,
        }
    }
}

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

pub type OfferSend = Offer<OfferContent>;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(bound(deserialize = "T: Default"))]
pub struct Offer<T = ()> {
    content: BTreeMap<String, OfferEntry<T>>,
}

impl OfferSend {
    /// Offer a single path (file or folder)
    #[cfg(not(target_family = "wasm"))]
    pub async fn new_file_or_folder(
        offer_name: String,
        path: impl AsRef<Path>,
    ) -> std::io::Result<Self> {
        let path = path.as_ref();
        log::trace!(
            "OfferSend::new_file_or_folder: {offer_name}, {}",
            path.display()
        );
        let mut content = BTreeMap::new();
        content.insert(offer_name, OfferSendEntry::new(path).await?);
        Ok(Self { content })
    }

    /// Offer list of paths (files and folders)
    /// Panics if any of the paths does not have a name (like `/`).
    /// Panics if any two or more of the paths have the same name.
    #[cfg(not(target_family = "wasm"))]
    pub async fn new_paths(paths: impl IntoIterator<Item = PathBuf>) -> std::io::Result<Self> {
        let mut content = BTreeMap::new();
        for path in paths {
            let offer_name = path.file_name().expect("Path must have a name");
            let offer_name = offer_name
                .to_str()
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!(
                            "{} is not UTF-8 encoded",
                            (offer_name.as_ref() as &Path).display()
                        ),
                    )
                })?
                .to_owned();
            let old = content.insert(offer_name, OfferSendEntry::new(path).await?);
            assert!(old.is_none(), "Duplicate names found");
        }
        Ok(Self { content })
    }

    /// Offer a single file with custom content
    ///
    /// You must ensure that the Reader contains exactly as many bytes
    /// as advertized in file_size.
    pub fn new_file_custom(offer_name: String, size: u64, content: OfferContent) -> Self {
        let mut content_ = BTreeMap::new();
        content_.insert(offer_name, OfferSendEntry::RegularFile { size, content });
        Self { content: content_ }
    }
}

impl<T> Offer<T> {
    pub fn top_level_paths(&self) -> impl Iterator<Item = &String> + '_ {
        self.content.keys()
    }

    pub fn get(&self, path: &[String]) -> Option<&OfferEntry<T>> {
        match path {
            [] => None,
            [start, rest @ ..] => self.content.get(start).and_then(|inner| inner.get(rest)),
        }
    }

    pub fn get_file(&self, path: &[String]) -> Option<(&T, u64)> {
        match path {
            [] => None,
            [start, rest @ ..] => self
                .content
                .get(start)
                .and_then(|inner| inner.get_file(rest)),
        }
    }

    /** Recursively list all file paths, without directory names or symlinks. */
    pub fn iter_file_paths(&self) -> impl Iterator<Item = Vec<String>> + '_ {
        self.iter_files().map(|val| val.0)
    }

    /** Recursively list all files, without directory names or symlinks. */
    pub fn iter_files(&self) -> impl Iterator<Item = (Vec<String>, &T, u64)> + '_ {
        self.content.iter().flat_map(|(name, offer)| {
            let name = name.clone();
            offer.iter_files().map(move |mut val| {
                val.0.insert(0, name.clone());
                val
            })
        })
    }

    pub fn total_size(&self) -> u64 {
        self.iter_files().map(|v| v.2).sum()
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn accept_all(&self, target_dir: &Path) -> OfferAccept {
        self.set_content(|path| {
            let full_path: PathBuf = target_dir.join(path.join("/"));
            let content = new_accept_content(move |append| {
                let full_path = full_path.clone();
                async_std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .append(append)
                    .truncate(!append)
                    .open(full_path)
            });
            AcceptInner {
                content: Box::new(content) as _,
                offset: 0,
                sha256: None,
            }
        })
    }

    #[cfg(not(target_family = "wasm"))]
    pub async fn create_directories(&self, target_path: &Path) -> std::io::Result<()> {
        // TODO this could be made more efficient by passing around just one buffer
        for (name, file) in &self.content {
            file.create_directories(&target_path.join(name)).await?;
        }
        Ok(())
    }

    // #[cfg(not(target_family = "wasm"))]
    // pub async fn create_symlinks(&self, target_path: &Path) -> std::io::Result<()> {
    //     // TODO this could be made more efficient by passing around just one buffer
    //     for (name, file) in &self.content {
    //         file.create_symlinks(&target_path.join(name)).await?;
    //     }
    //     Ok(())
    // }

    pub fn offer_name(&self) -> String {
        let (name, entry) = self.content.iter().next().unwrap();
        if self.is_multiple() {
            format!(
                "{name} and {} other files or directories",
                self.content.len() - 1
            )
        } else if self.is_directory() {
            let count = entry.iter_files().count();
            format!("{name} with {count} files inside")
        } else {
            name.clone()
        }
    }

    pub fn is_multiple(&self) -> bool {
        self.content.len() > 1
    }

    pub fn is_directory(&self) -> bool {
        self.is_multiple()
            || self
                .content
                .values()
                .any(|f| matches!(f, OfferEntry::Directory { .. }))
    }

    pub fn set_content<U>(&self, mut f: impl FnMut(&[String]) -> U) -> Offer<U> {
        Offer {
            content: self
                .content
                .iter()
                .map(|(k, v)| (k.clone(), v.set_content(&mut vec![k.clone()], &mut f)))
                .collect(),
        }
    }
}

impl<T: 'static + Send> Offer<T> {
    /** Recursively list all files, without directory names or symlinks. */
    pub fn into_iter_files(self) -> impl Iterator<Item = (Vec<String>, T, u64)> + Send {
        self.content.into_iter().flat_map(|(name, offer)| {
            offer.into_iter_files().map(move |mut val| {
                val.0.insert(0, name.clone());
                val
            })
        })
    }
}

impl<T> From<&Offer<T>> for Offer {
    fn from(from: &Offer<T>) -> Self {
        from.set_content(|_| ())
    }
}

/// The signature is basically just `() -> io::Result<dyn AsyncRead + AsyncSeek>`, but in async
///
/// This may be called multiple times during the send process, an imlementations that generate their
/// output dynamically must ensure all invocations produce the same result — independently of each other
/// (things may be concurrent).
pub type OfferContent = Box<
    dyn Fn() -> futures::future::BoxFuture<
            'static,
            std::io::Result<Box<dyn AsyncReadSeek + Unpin + Send>>,
        > + Send,
>;

pub fn new_offer_content<F, G, H>(content: F) -> OfferContent
where
    F: Fn() -> G + Send + 'static,
    G: Future<Output = std::io::Result<H>> + Send + 'static,
    H: AsyncReadSeek + Unpin + Send + 'static,
{
    let wrap_fun = move || {
        use futures::TryFutureExt;

        let fut = content();
        let wrap_fut = fut.map_ok(|read| Box::new(read) as Box<dyn AsyncReadSeek + Unpin + Send>);

        Box::pin(wrap_fut) as futures::future::BoxFuture<'static, _>
    };
    Box::new(wrap_fun) as _
}

pub type OfferSendEntry = OfferEntry<OfferContent>;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
#[serde(bound(deserialize = "T: Default"))]
pub enum OfferEntry<T = ()> {
    RegularFile {
        size: u64,
        #[serde(skip)]
        content: T,
    },
    Directory {
        content: BTreeMap<String, Self>,
    },
    // Symlink {
    //     target: String,
    // },
}

impl OfferSendEntry {
    #[cfg(not(target_family = "wasm"))]
    async fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        // Workaround for https://github.com/rust-lang/rust/issues/78649
        #[inline(always)]
        fn new_recurse<'a>(
            path: impl AsRef<Path> + 'a + Send,
        ) -> futures::future::BoxFuture<'a, std::io::Result<OfferSendEntry>> {
            Box::pin(OfferSendEntry::new(path))
        }

        let path = path.as_ref();
        // let metadata = async_std::fs::symlink_metadata(path).await?;
        let metadata = async_std::fs::metadata(path).await?;
        // let mtime = metadata.modified()?
        //     .duration_since(std::time::SystemTime::UNIX_EPOCH)
        //     .unwrap_or_default()
        //     .as_secs();
        if metadata.is_file() {
            log::trace!("OfferSendEntry::new {path:?} is file");
            let path = path.to_owned();
            Ok(Self::RegularFile {
                size: metadata.len(),
                content: new_offer_content(move || {
                    let path = path.clone();
                    async_std::fs::File::open(path)
                }),
            })
        // } else if metadata.is_symlink() {
        //     log::trace!("OfferSendEntry::new {path:?} is symlink");
        //     let target = async_std::fs::read_link(path).await?;
        //     Ok(Self::Symlink {
        //         target: target
        //             .to_str()
        //             .ok_or_else(|| {
        //                 std::io::Error::new(
        //                     std::io::ErrorKind::Other,
        //                     format!("{} is not UTF-8 encoded", target.display()),
        //                 )
        //             })?
        //             .to_string(),
        //     })
        } else if metadata.is_dir() {
            use futures::TryStreamExt;
            log::trace!("OfferSendEntry::new {path:?} is directory");

            let content: BTreeMap<String, Self> = async_std::fs::read_dir(path)
                .await?
                .and_then(|file| async move {
                    let path = file.path();
                    let name = path
                        .file_name()
                        .expect("Internal error: non-root paths should always have a name")
                        .to_str()
                        .ok_or_else(|| {
                            std::io::Error::new(
                                std::io::ErrorKind::Other,
                                format!("{} is not UTF-8 encoded", path.display()),
                            )
                        })?
                        .to_owned();
                    let offer = new_recurse(path).await?;
                    Ok((name, offer))
                })
                .try_collect()
                .await?;
            Ok(Self::Directory { content })
        } else {
            unreachable!()
        }
    }
}

impl<T> OfferEntry<T> {
    /** Recursively list all files, without directory names or symlinks. */
    fn iter_files(&self) -> impl Iterator<Item = (Vec<String>, &T, u64)> + '_ {
        // TODO I couldn't think up a less efficient way to do this ^^
        match self {
            Self::Directory { content, .. } => {
                let iter = content.iter().flat_map(|(name, offer)| {
                    let name = name.clone();
                    offer.iter_files().map(move |mut val| {
                        val.0.insert(0, name.clone());
                        val
                    })
                });
                Box::new(iter) as Box<dyn Iterator<Item = _>>
            },
            Self::RegularFile { content, size } => {
                Box::new(std::iter::once((vec![], content, *size))) as Box<dyn Iterator<Item = _>>
            },
            // Self::Symlink { .. } => Box::new(std::iter::empty()) as Box<dyn Iterator<Item = _>>,
        }
    }

    fn get(&self, path: &[String]) -> Option<&Self> {
        match path {
            [] => Some(self),
            [start, rest @ ..] => match self {
                Self::Directory { content, .. } => {
                    content.get(start).and_then(|inner| inner.get(rest))
                },
                _ => None,
            },
        }
    }

    fn get_file(&self, path: &[String]) -> Option<(&T, u64)> {
        match path {
            [] => match self {
                Self::RegularFile { content, size } => Some((content, *size)),
                _ => None,
            },
            [start, rest @ ..] => match self {
                Self::Directory { content, .. } => {
                    content.get(start).and_then(|inner| inner.get_file(rest))
                },
                _ => None,
            },
        }
    }

    #[cfg(not(target_family = "wasm"))]
    async fn create_directories(&self, target_path: &Path) -> std::io::Result<()> {
        #[inline(always)]
        fn recurse<'a, T>(
            this: &'a OfferEntry<T>,
            path: &'a Path,
        ) -> futures::future::LocalBoxFuture<'a, std::io::Result<()>> {
            Box::pin(OfferEntry::create_directories(this, path))
        }
        match self {
            Self::Directory { content, .. } => {
                async_std::fs::create_dir(target_path).await?;
                for (name, file) in content {
                    recurse(file, &target_path.join(name)).await?;
                }
                Ok(())
            },
            _ => Ok(()),
        }
    }

    // #[cfg(not(target_family = "wasm"))]
    // async fn create_symlinks(&self, target_path: &Path) -> std::io::Result<()> {
    //     #[inline(always)]
    //     fn recurse<'a, T>(
    //         this: &'a OfferEntry<T>,
    //         path: &'a Path,
    //     ) -> futures::future::LocalBoxFuture<'a, std::io::Result<()>> {
    //         Box::pin(OfferEntry::create_symlinks(this, path))
    //     }
    //     match self {
    //         Self::Symlink { target } => {
    //             todo!()
    //         },
    //         Self::Directory { content, .. } => {
    //             for (name, file) in content {
    //                 recurse(file, &target_path.join(name)).await?;
    //             }
    //             Ok(())
    //         },
    //         _ => Ok(()),
    //     }
    // }

    fn set_content<U>(
        &self,
        base_path: &mut Vec<String>,
        f: &mut impl FnMut(&[String]) -> U,
    ) -> OfferEntry<U> {
        match self {
            OfferEntry::RegularFile { size, .. } => OfferEntry::RegularFile {
                size: *size,
                content: f(base_path),
            },
            OfferEntry::Directory { content } => OfferEntry::Directory {
                content: content
                    .iter()
                    .map(|(k, v)| {
                        base_path.push(k.clone());
                        let v = v.set_content(base_path, f);
                        base_path.pop();
                        (k.clone(), v)
                    })
                    .collect(),
            },
            // OfferEntry::Symlink { target } => OfferEntry::Symlink {
            //     target: target.clone(),
            // },
        }
    }
}

impl<T: 'static + Send> OfferEntry<T> {
    /** Recursively list all files, without directory names or symlinks. */
    fn into_iter_files(self) -> impl Iterator<Item = (Vec<String>, T, u64)> + Send {
        // TODO I couldn't think up a less efficient way to do this ^^
        match self {
            Self::Directory { content, .. } => {
                let iter = content.into_iter().flat_map(|(name, offer)| {
                    offer.into_iter_files().map(move |mut val| {
                        val.0.insert(0, name.clone());
                        val
                    })
                });
                Box::new(iter) as Box<dyn Iterator<Item = _> + Send>
            },
            Self::RegularFile { content, size } => {
                Box::new(std::iter::once((vec![], content, size)))
                    as Box<dyn Iterator<Item = _> + Send>
            },
            // Self::Symlink { .. } => {
            //     Box::new(std::iter::empty()) as Box<dyn Iterator<Item = _> + Send>
            // },
        }
    }
}

impl<T> From<&OfferEntry<T>> for OfferEntry {
    fn from(from: &OfferEntry<T>) -> Self {
        /* Note: this violates some invariants and only works because our mapper discards the path argument */
        from.set_content(&mut vec![], &mut |_| ())
    }
}

/// The signature is basically just `bool -> io::Result<dyn AsyncRead + AsyncSeek>`, but in async
///
/// The boolean parameter dictates whether we start from scratch or not:
/// true: Append to existing files
/// false: Truncate if necessary
pub type AcceptContent = Box<
    dyn FnOnce(
            bool,
        ) -> futures::future::BoxFuture<
            'static,
            std::io::Result<Box<dyn AsyncWrite + Unpin + Send>>,
        > + Send,
>;

pub fn new_accept_content<F, G, H>(content: F) -> AcceptContent
where
    F: Fn(bool) -> G + Send + 'static,
    G: Future<Output = std::io::Result<H>> + Send + 'static,
    H: AsyncWrite + Unpin + Send + 'static,
{
    let wrap_fun = move |append| {
        use futures::TryFutureExt;

        let fut = content(append);
        let wrap_fut = fut.map_ok(|write| Box::new(write) as Box<dyn AsyncWrite + Unpin + Send>);

        Box::pin(wrap_fut) as futures::future::BoxFuture<'static, _>
    };
    Box::new(wrap_fun) as _
}

pub type OfferAccept = Offer<AcceptInner>;

pub struct AcceptInner {
    pub offset: u64,
    pub sha256: Option<[u8; 32]>,
    pub content: AcceptContent,
}

pub async fn send(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    offer: OfferSend,
    transit_handler: impl FnOnce(transit::TransitInfo),
    progress_handler: impl FnMut(u64, u64) + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError> {
    let peer_version: AppVersion = serde_json::from_value(wormhole.peer_version().clone())?;
    if peer_version.supports_v2() {
        v2::send(
            wormhole,
            relay_hints,
            transit_abilities,
            offer,
            progress_handler,
            peer_version,
            cancel,
        )
        .await
    } else {
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
}

/**
 * Wait for a file offer from the other side
 *
 * This method waits for an offer message and builds up a [`ReceiveRequest`](ReceiveRequest).
 * It will also start building a TCP connection to the other side using the transit protocol.
 *
 * Returns `None` if the task got cancelled.
 */
pub async fn request(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    cancel: impl Future<Output = ()>,
) -> Result<Option<ReceiveRequest>, TransferError> {
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

/**
 * A pending files send offer from the other side
 *
 * You *should* consume this object, by matching on the protocol version and then calling either `accept` or `reject`.
 */
#[must_use]
pub enum ReceiveRequest {
    V1(ReceiveRequestV1),
    V2(ReceiveRequestV2),
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
