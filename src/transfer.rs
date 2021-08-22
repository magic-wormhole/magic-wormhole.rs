//! Client-to-Client protocol to organize file transfers
//!
//! This gives you the actual capability to transfer files, that feature that Magic Wormhole got known and loved for.
//!
//! It is bound to an [`APPID`](APPID). Only applications using that APPID (and thus this protocol) can interoperate with
//! the original Python implementation (and other compliant implementations).
//!
//! At its core, "peer messages" are exchanged over an established wormhole connection with the other side.
//! They are used to set up a [transit] portal and to exchange a file offer/accept. Then, the file is transmitted over the transit relay.

use futures::{AsyncRead, AsyncWrite};
use serde_derive::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::sync::Arc;

use super::{
    core::WormholeError,
    transit,
    transit::{RelayUrl, Transit},
    AppID, Wormhole,
};
use async_std::io::{prelude::WriteExt, ReadExt};
use log::*;
use sha2::{digest::FixedOutput, Digest, Sha256};
use std::path::PathBuf;
use transit::{TransitConnectError, TransitConnector, TransitError};

mod messages;
use messages::*;

const APPID_RAW: &str = "lothar.com/wormhole/text-or-file-xfer";

/// The App ID associated with this protocol.
pub const APPID: AppID = AppID(std::borrow::Cow::Borrowed(APPID_RAW));

/// An [`crate::AppConfig`] with sane defaults for this protocol.
///
/// You **must not** change `id` and `rendezvous_url` to be interoperable.
/// The `app_version` can be adjusted if you want to disable some features.
pub const APP_CONFIG: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(std::borrow::Cow::Borrowed(APPID_RAW)),
    rendezvous_url: std::borrow::Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion {},
};

// TODO be more extensible on the JSON enum types (i.e. recognize unknown variants)

// TODO send peer errors when something went wrong (if possible)
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
    #[error("Corrupt message received")]
    ProtocolJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
    /// A generic string message for "something went wrong", i.e.
    /// the server sent some bullshit message order
    #[error("Protocol error: {}", _0)]
    Protocol(Box<str>),
    #[error(
        "Unexpected message (protocol error): Expected '{}', but got: {:?}",
        _0,
        _1
    )]
    ProtocolUnexpectedMessage(Box<str>, Box<dyn std::fmt::Debug + Send + Sync>),
    #[error("Wormhole connection error")]
    Wormhole(
        #[from]
        #[source]
        WormholeError,
    ),
    #[error("Internal error: wormhole core died")]
    WormholeClosed(
        #[from]
        #[source]
        futures::channel::mpsc::SendError,
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
    pub(self) fn protocol(error: impl Into<Box<str>>) -> Self {
        Self::Protocol(error.into())
    }

    pub(self) fn unexpected_message(
        expected: impl Into<Box<str>>,
        got: impl std::fmt::Debug + Send + Sync + 'static,
    ) -> Self {
        Self::ProtocolUnexpectedMessage(expected.into(), Box::new(got))
    }
}

/**
 * The application specific version information for this protocol.
 *
 * At the moment, this always is an empty object, but this will likely change in the future.
 */
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AppVersion {}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitAck {
    pub ack: String,
    pub sha256: String,
}

impl TransitAck {
    pub fn new(msg: impl Into<String>, sha256: impl Into<String>) -> Self {
        TransitAck {
            ack: msg.into(),
            sha256: sha256.into(),
        }
    }

    #[cfg(test)]
    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    pub fn serialize_vec(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}

pub async fn send_file_or_folder<N, M, H>(
    wormhole: &mut Wormhole,
    relay_url: &RelayUrl,
    file_path: N,
    file_name: M,
    progress_handler: H,
) -> Result<(), TransferError>
where
    N: AsRef<async_std::path::Path>,
    M: AsRef<async_std::path::Path>,
    H: FnMut(u64, u64) + 'static,
{
    use async_std::fs::File;
    let file_path = file_path.as_ref();
    let file_name = file_name.as_ref();

    let mut file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    if metadata.is_dir() {
        send_folder(wormhole, relay_url, file_path, file_name, progress_handler).await?;
    } else {
        let file_size = metadata.len();
        send_file(
            wormhole,
            relay_url,
            &mut file,
            file_name,
            file_size,
            progress_handler,
        )
        .await?;
    }
    Ok(())
}

/// Send a file to the other side
///
/// You must ensure that the Reader contains exactly as many bytes
/// as advertized in file_size.
pub async fn send_file<F, N, H>(
    wormhole: &mut Wormhole,
    relay_url: &RelayUrl,
    file: &mut F,
    file_name: N,
    file_size: u64,
    progress_handler: H,
) -> Result<(), TransferError>
where
    F: AsyncRead + Unpin,
    N: Into<PathBuf>,
    H: FnMut(u64, u64) + 'static,
{
    let connector = transit::init(transit::Ability::all_abilities(), relay_url).await?;

    // We want to do some transit
    debug!("Sending transit message '{:?}", connector.our_hints());
    wormhole
        .send(
            PeerMessage::new_transit(
                connector.our_abilities().to_vec(),
                (**connector.our_hints()).clone().into(),
            )
            .serialize_vec(),
        )
        .await?;

    // Send file offer message.
    debug!("Sending file offer");
    wormhole
        .send(PeerMessage::new_offer_file(file_name, file_size).serialize_vec())
        .await?;

    // Wait for their transit response
    let (their_abilities, their_hints): (Vec<transit::Ability>, transit::Hints) =
        match serde_json::from_slice(&wormhole.receive().await?)? {
            PeerMessage::Transit(transit) => {
                debug!("received transit message: {:?}", transit);
                (transit.abilities_v1, transit.hints_v1.into())
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            other => {
                let error = TransferError::unexpected_message("transit", other);
                let _ = wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        };

    {
        // Wait for file_ack
        let fileack_msg = serde_json::from_slice(&wormhole.receive().await?)?;
        debug!("received file ack message: {:?}", fileack_msg);

        match fileack_msg {
            PeerMessage::Answer(AnswerType::FileAck(msg)) => {
                ensure!(msg == "ok", TransferError::AckError);
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            _ => {
                let error = TransferError::unexpected_message("answer/file_ack", fileack_msg);
                let _ = wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        }
    }

    let mut transit = match connector
        .leader_connect(
            wormhole.key().derive_transit_key(wormhole.appid()),
            Arc::new(their_hints),
        )
        .await
    {
        Ok(transit) => transit,
        Err(error) => {
            let error = TransferError::TransitConnect(error);
            let _ = wormhole
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            return Err(error);
        },
    };

    debug!("Beginning file transfer");

    // 11. send the file as encrypted records.
    let checksum = match send_records(&mut transit, file, file_size, progress_handler).await {
        Err(TransferError::Transit(error)) => {
            let _ = wormhole
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            Err(TransferError::Transit(error))
        },
        other => other,
    }?;

    // 13. wait for the transit ack with sha256 sum from the peer.
    debug!("sent file. Waiting for ack");
    let transit_ack = transit.receive_record().await?;
    let transit_ack_msg = serde_json::from_slice::<TransitAck>(&transit_ack)?;
    ensure!(
        transit_ack_msg.sha256 == hex::encode(checksum),
        TransferError::Checksum
    );
    debug!("transfer complete!");
    Ok(())
}

/// Send a folder to the other side
///
/// This isn't a proper folder transfer as per the Wormhole protocol
/// because it sends it in a way so that the receiver still has to manually
/// unpack it. But it's better than nothing
pub async fn send_folder<N, M, H>(
    wormhole: &mut Wormhole,
    relay_url: &RelayUrl,
    folder_path: N,
    folder_name: M,
    progress_handler: H,
) -> Result<(), TransferError>
where
    N: Into<PathBuf>,
    M: Into<PathBuf>,
    H: FnMut(u64, u64) + 'static,
{
    let connector = transit::init(transit::Ability::all_abilities(), relay_url).await?;
    let folder_path = folder_path.into();

    if !folder_path.is_dir() {
        panic!(
            "You should only call this method with directory paths, but '{}' is not",
            folder_path.display()
        );
    }

    // We want to do some transit
    debug!("Sending transit message '{:?}", connector.our_hints());
    wormhole
        .send(
            PeerMessage::new_transit(
                connector.our_abilities().to_vec(),
                (**connector.our_hints()).clone().into(),
            )
            .serialize_vec(),
        )
        .await?;

    use tar::Builder;
    // use sha2::{digest::FixedOutput, Digest, Sha256};

    /* Helper struct stolen from https://docs.rs/count-write/0.1.0 */
    struct CountWrite<W> {
        inner: W,
        count: u64,
    }

    impl<W: std::io::Write> std::io::Write for CountWrite<W> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let written = self.inner.write(buf)?;
            self.count += written as u64;
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner.flush()
        }
    }

    /* We need to know the length of what we are going to send in advance. So we build the
     * tar file once, stream it into the void, and the second time we stream it over the
     * wire. Also hashing for future reference.
     */
    log::info!(
        "Tar'ing '{}' to see how big it'll be :)",
        folder_path.display()
    );
    let folder_path2 = folder_path.clone();
    let (length, sha256sum_initial) = async_std::task::spawn_blocking(move || {
        let mut hasher = Sha256::new();
        let mut counter = CountWrite {
            inner: &mut hasher,
            count: 0,
        };
        let mut builder = Builder::new(&mut counter);

        builder.mode(tar::HeaderMode::Deterministic);
        builder.follow_symlinks(false);
        /* A hasher should never fail writing */
        builder.append_dir_all("", folder_path2).unwrap();
        builder.finish().unwrap();

        std::mem::drop(builder);
        let count = counter.count;
        std::mem::drop(counter);
        (count, hasher.finalize_fixed())
    })
    .await;

    // Send file offer message.
    debug!("Sending file offer");
    wormhole
        .send(PeerMessage::new_offer_file(folder_name, length).serialize_vec())
        .await?;

    // Wait for their transit response
    let (their_abilities, their_hints): (Vec<transit::Ability>, transit::Hints) =
        match serde_json::from_slice(&wormhole.receive().await?)? {
            PeerMessage::Transit(transit) => {
                debug!("received transit message: {:?}", transit);
                (transit.abilities_v1, transit.hints_v1.into())
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            other => {
                let error = TransferError::unexpected_message("transit", other);
                let _ = wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        };

    {
        // Wait for file_ack
        let fileack_msg = serde_json::from_slice(&wormhole.receive().await?)?;
        debug!("received file ack message: {:?}", fileack_msg);

        match fileack_msg {
            PeerMessage::Answer(AnswerType::FileAck(msg)) => {
                ensure!(msg == "ok", TransferError::AckError);
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            _ => {
                let error = TransferError::unexpected_message("answer/file_ack", fileack_msg);
                let _ = wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        }
    }

    let mut transit = match connector
        .leader_connect(
            wormhole.key().derive_transit_key(wormhole.appid()),
            Arc::new(their_hints),
        )
        .await
    {
        Ok(transit) => transit,
        Err(error) => {
            let error = TransferError::TransitConnect(error);
            let _ = wormhole
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            return Err(error);
        },
    };

    debug!("Beginning file transfer");

    /* Helper struct stolen from https://github.com/softprops/broadcast/blob/master/src/lib.rs */
    pub struct BroadcastWriter<A: std::io::Write, B: std::io::Write> {
        primary: A,
        secondary: B,
    }

    impl<A: std::io::Write, B: std::io::Write> std::io::Write for BroadcastWriter<A, B> {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            let n = self.primary.write(data).unwrap();
            self.secondary.write_all(&data[..n]).unwrap();
            Ok(n)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.primary.flush().and(self.secondary.flush())
        }
    }

    // 11. send the file as encrypted records.
    use futures::{AsyncReadExt, AsyncWriteExt};
    let (mut reader, mut writer) = futures_ringbuf::RingBuffer::new(4096).split();

    struct BlockingWrite<W>(std::pin::Pin<Box<W>>);

    impl<W: AsyncWrite + Send + Unpin> std::io::Write for BlockingWrite<W> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut writer = self.0.as_mut();
            futures::executor::block_on(AsyncWriteExt::write(&mut writer, buf))
        }

        fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
            let mut writer = self.0.as_mut();
            futures::executor::block_on(AsyncWriteExt::flush(&mut writer))
        }
    }

    let file_sender = async_std::task::spawn_blocking(move || {
        let mut hasher = Sha256::new();
        let mut hash_writer = BroadcastWriter {
            primary: BlockingWrite(Box::pin(&mut writer)),
            secondary: &mut hasher,
        };
        let mut builder = Builder::new(&mut hash_writer);

        builder.mode(tar::HeaderMode::Deterministic);
        builder.follow_symlinks(false);
        builder.append_dir_all("", folder_path).unwrap();
        builder.finish().unwrap();

        std::mem::drop(builder);
        std::mem::drop(hash_writer);

        std::io::Result::Ok(hasher.finalize_fixed())
    });

    let checksum = match send_records(&mut transit, &mut reader, length, progress_handler).await {
        Err(TransferError::Transit(error)) => {
            let _ = wormhole
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            Err(TransferError::Transit(error))
        },
        other => other,
    }?;
    /* This should always be ready by now, but just in case */
    let sha256sum = file_sender.await.unwrap();

    /* Check if the hash sum still matches what we advertized. Otherwise, tell the other side and bail out */
    if sha256sum != sha256sum_initial {
        let error = TransferError::FilesystemSkew;
        let _ = wormhole
            .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
            .await;
        bail!(error)
    }

    // 13. wait for the transit ack with sha256 sum from the peer.
    debug!("sent file. Waiting for ack");
    let transit_ack = transit.receive_record().await?;
    let transit_ack_msg = serde_json::from_slice::<TransitAck>(&transit_ack)?;
    ensure!(
        transit_ack_msg.sha256 == hex::encode(checksum),
        TransferError::Checksum
    );
    debug!("transfer complete!");
    Ok(())
}

/**
 * Wait for a file offer from the other side
 *
 * This method waits for an offer message and builds up a [`ReceiveRequest`](ReceiveRequest).
 * It will also start building a TCP connection to the other side using the transit protocol.
 */
pub async fn request_file<'a>(
    wormhole: &'a mut Wormhole,
    relay_url: &RelayUrl,
) -> Result<ReceiveRequest<'a>, TransferError> {
    let connector = transit::init(transit::Ability::all_abilities(), relay_url).await?;

    // send the transit message
    debug!("Sending transit message '{:?}", connector.our_hints());
    wormhole
        .send(
            PeerMessage::new_transit(
                connector.our_abilities().to_vec(),
                (**connector.our_hints()).clone().into(),
            )
            .serialize_vec(),
        )
        .await?;

    // receive transit message
    let (their_abilities, their_hints): (Vec<transit::Ability>, transit::Hints) =
        match serde_json::from_slice(&wormhole.receive().await?)? {
            PeerMessage::Transit(transit) => {
                debug!("received transit message: {:?}", transit);
                (transit.abilities_v1, transit.hints_v1.into())
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            other => {
                let error = TransferError::unexpected_message("transit", other);
                let _ = wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        };

    // 3. receive file offer message from peer
    let maybe_offer = serde_json::from_slice(&wormhole.receive().await?)?;
    debug!("Received offer message '{:?}'", &maybe_offer);

    let (filename, filesize) = match maybe_offer {
        PeerMessage::Offer(offer_type) => match offer_type {
            OfferType::File { filename, filesize } => (filename, filesize),
            OfferType::Directory {
                mut dirname,
                zipsize,
                ..
            } => {
                dirname.set_extension("zip");
                (dirname, zipsize)
            },
            _ => bail!(TransferError::UnsupportedOffer),
        },
        PeerMessage::Error(err) => {
            bail!(TransferError::PeerError(err));
        },
        _ => {
            let error = TransferError::unexpected_message("offer", maybe_offer);
            let _ = wormhole
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            bail!(error)
        },
    };

    let req = ReceiveRequest {
        wormhole,
        filename,
        filesize,
        connector,
        their_hints: Arc::new(their_hints),
    };

    Ok(req)
}

/**
 * A pending files send offer from the other side
 *
 * You *should* consume this object, either by calling [`accept`](ReceiveRequest::accept) or [`reject`](ReceiveRequest::reject).
 */
#[must_use]
pub struct ReceiveRequest<'a> {
    wormhole: &'a mut Wormhole,
    connector: TransitConnector,
    /// **Security warning:** this is untrusted and unverified input
    pub filename: PathBuf,
    pub filesize: u64,
    their_hints: Arc<transit::Hints>,
}

impl<'a> ReceiveRequest<'a> {
    /**
     * Accept the file offer
     *
     * This will transfer the file and save it on disk.
     */
    pub async fn accept<F, W>(
        self,
        progress_handler: F,
        content_handler: &mut W,
    ) -> Result<(), TransferError>
    where
        F: FnMut(u64, u64) + 'static,
        W: AsyncWrite + Unpin,
    {
        // send file ack.
        debug!("Sending ack");
        self.wormhole
            .send(PeerMessage::new_file_ack("ok").serialize_vec())
            .await?;

        let mut transit = match self
            .connector
            .follower_connect(
                self.wormhole
                    .key()
                    .derive_transit_key(self.wormhole.appid()),
                self.their_hints.clone(),
            )
            .await
        {
            Ok(transit) => transit,
            Err(error) => {
                let error = TransferError::TransitConnect(error);
                let _ = self
                    .wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                return Err(error);
            },
        };

        debug!("Beginning file transfer");
        // TODO here's the right position for applying the output directory and to check for malicious (relative) file paths
        match tcp_file_receive(
            &mut transit,
            self.filesize,
            progress_handler,
            content_handler,
        )
        .await
        {
            Err(TransferError::Transit(error)) => {
                let _ = self
                    .wormhole
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                Err(TransferError::Transit(error))
            },
            other => other,
        }
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     * You can close the wormhole afterwards.
     */
    pub async fn reject(self) -> Result<(), TransferError> {
        self.wormhole
            .send(PeerMessage::new_error_message("transfer rejected").serialize_vec())
            .await?;

        Ok(())
    }
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
async fn send_records<F>(
    transit: &mut Transit,
    file: &mut (impl AsyncRead + Unpin),
    file_size: u64,
    mut progress_handler: F,
) -> Result<Vec<u8>, TransferError>
where
    F: FnMut(u64, u64) + 'static,
{
    // rough plan:
    // 1. Open the file
    // 2. read a block of N bytes
    // 3. calculate a rolling sha256sum.
    // 4. AEAD with skey and with nonce as a counter from 0.
    // 5. send the encrypted buffer to the socket.
    // 6. go to step #2 till eof.
    // 7. if eof, return sha256 sum.

    // Report at 0 to allow clients to configure as necessary.
    progress_handler(0, file_size);

    let mut hasher = Sha256::default();

    // Yeah, maybe don't allocate 4kiB on the stack…
    let mut plaintext = Box::new([0u8; 4096]);
    let mut sent_size = 0;
    loop {
        // read a block of up to 4096 bytes
        let n = file.read(&mut plaintext[..]).await?;

        // send the encrypted record
        transit.send_record(&plaintext[0..n]).await?;
        sent_size += n as u64;
        progress_handler(sent_size, file_size);

        // sha256 of the input
        hasher.update(&plaintext[..n]);

        if n < 4096 {
            break;
        }
    }

    ensure!(
        sent_size == file_size,
        TransferError::FileSize {
            sent_size,
            file_size
        }
    );

    Ok(hasher.finalize_fixed().to_vec())
}

async fn receive_records<F, W>(
    filesize: u64,
    transit: &mut Transit,
    mut progress_handler: F,
    content_handler: &mut W,
) -> Result<Vec<u8>, TransferError>
where
    F: FnMut(u64, u64) + 'static,
    W: AsyncWrite + Unpin,
{
    let mut hasher = Sha256::default();
    let total = filesize;

    let mut remaining_size = filesize as usize;

    // Might not need to do this here, since `accept()` is where they'd know the filesize
    // already...
    progress_handler(0, total);

    while remaining_size > 0 {
        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = transit.receive_record().await?;

        content_handler.write_all(&plaintext).await?;

        // 4. calculate a rolling sha256 sum of the decrypted output.
        hasher.update(&plaintext);

        remaining_size -= plaintext.len();

        let remaining = remaining_size as u64;
        progress_handler(total - remaining, total);
    }

    debug!("done");
    // TODO: 5. write the buffer into a file.
    Ok(hasher.finalize_fixed().to_vec())
}

async fn tcp_file_receive<F, W>(
    transit: &mut Transit,
    filesize: u64,
    progress_handler: F,
    content_handler: &mut W,
) -> Result<(), TransferError>
where
    F: FnMut(u64, u64) + 'static,
    W: AsyncWrite + Unpin,
{
    // 5. receive encrypted records
    // now skey and rkey can be used. skey is used by the tx side, rkey is used
    // by the rx side for symmetric encryption.
    let checksum = receive_records(filesize, transit, progress_handler, content_handler).await?;

    let sha256sum = hex::encode(checksum.as_slice());
    debug!("sha256 sum: {:?}", sha256sum);

    // 6. verify sha256 sum by sending an ack message to peer along with checksum.
    transit
        .send_record(&TransitAck::new("ok", &sha256sum).serialize_vec())
        .await?;

    // 7. close socket.
    // well, no need, it gets dropped when it goes out of scope.
    debug!("Transfer complete");
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_message() {
        let m1 = PeerMessage::new_offer_message("hello from rust");
        assert_eq!(
            m1.serialize(),
            "{\"offer\":{\"message\":\"hello from rust\"}}"
        );
    }

    #[test]
    fn test_offer_file() {
        let f1 = PeerMessage::new_offer_file("somefile.txt", 34556);
        assert_eq!(
            f1.serialize(),
            "{\"offer\":{\"file\":{\"filename\":\"somefile.txt\",\"filesize\":34556}}}"
        );
    }

    #[test]
    fn test_offer_directory() {
        let d1 = PeerMessage::new_offer_directory("somedirectory", "zipped", 45, 1234, 10);
        assert_eq!(
            d1.serialize(),
            "{\"offer\":{\"directory\":{\"dirname\":\"somedirectory\",\"mode\":\"zipped\",\"numbytes\":1234,\"numfiles\":10,\"zipsize\":45}}}"
        );
    }

    #[test]
    fn test_message_ack() {
        let m1 = PeerMessage::new_message_ack("ok");
        assert_eq!(m1.serialize(), "{\"answer\":{\"message_ack\":\"ok\"}}");
    }

    #[test]
    fn test_file_ack() {
        let f1 = PeerMessage::new_file_ack("ok");
        assert_eq!(f1.serialize(), "{\"answer\":{\"file_ack\":\"ok\"}}");
    }

    #[test]
    fn test_transit_ack() {
        let f1 = TransitAck::new("ok", "deadbeaf");
        assert_eq!(f1.serialize(), "{\"ack\":\"ok\",\"sha256\":\"deadbeaf\"}");
    }
}
