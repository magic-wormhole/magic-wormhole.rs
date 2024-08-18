use std::collections::BTreeMap;
#[cfg(not(target_family = "wasm"))]
use std::path::{Path, PathBuf};

use futures::{AsyncRead, AsyncSeek, AsyncWrite, Future};
use serde::{Deserialize, Serialize};

pub type OfferSend = Offer<OfferContent>;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(bound(deserialize = "T: Default"))]
pub struct Offer<T = ()> {
    pub(super) content: BTreeMap<String, OfferEntry<T>>,
}

impl OfferSend {
    /// Offer a single path (file or folder)
    #[cfg(not(target_family = "wasm"))]
    pub async fn new_file_or_folder(
        offer_name: String,
        path: impl AsRef<Path>,
    ) -> std::io::Result<Self> {
        let path = path.as_ref();
        tracing::trace!(
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

pub trait AsyncReadSeek: AsyncRead + AsyncSeek {}
impl<T> AsyncReadSeek for T where T: AsyncRead + AsyncSeek {}

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

pub fn new_offer_content<F, G, H>(content_provider: F) -> OfferContent
where
    F: Fn() -> G + Send + 'static,
    G: Future<Output = std::io::Result<H>> + Send + 'static,
    H: AsyncReadSeek + Unpin + Send + 'static,
{
    let wrap_fun = move || {
        use futures::TryFutureExt;

        let fut = content_provider();
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
    pub(super) async fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
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
            tracing::trace!("OfferSendEntry::new {path:?} is file");
            let path = path.to_owned();
            Ok(Self::RegularFile {
                size: metadata.len(),
                content: new_offer_content(move || {
                    let path = path.clone();
                    async_std::fs::File::open(path)
                }),
            })
        // } else if metadata.is_symlink() {
        //     tracing::trace!("OfferSendEntry::new {path:?} is symlink");
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
            tracing::trace!("OfferSendEntry::new {path:?} is directory");

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
/// The signature is basically just `bool -> io::Result<dyn AsyncWrite>`, but in async
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

pub fn new_accept_content<F, G, H>(content_handler: F) -> AcceptContent
where
    F: Fn(bool) -> G + Send + 'static,
    G: Future<Output = std::io::Result<H>> + Send + 'static,
    H: AsyncWrite + Unpin + Send + 'static,
{
    let wrap_fun = move |append| {
        use futures::TryFutureExt;

        let fut = content_handler(append);
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
