//! Client-to-Client protocol to organize file transfers
//!
//! This gives you the actual capability to transfer files, that feature that Magic Wormhole got known and loved for.
//!
//! It is bound to an [`APPID`](APPID). Only applications using that APPID (and thus this protocol) can interoperate with
//! the original Python implementation (and other compliant implementations).
//!
//! At its core, [`PeerMessage`s](PeerMessage) are exchanged over an established wormhole connection with the other side.
//! They are used to set up a [transit] portal and to exchange a file offer/accept. Then, the file is transmitted over the transit relay.

use futures::{AsyncRead, AsyncWrite};
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::{
    bail,
    core::WormholeCoreError,
    ensure, transit,
    transit::{RelayUrl, Transit},
    Wormhole,
};
use async_std::io::{prelude::WriteExt, ReadExt};
use futures::{SinkExt, StreamExt};
use log::*;
use sha2::{digest::FixedOutput, Digest, Sha256};
use std::path::PathBuf;
use transit::{TransitConnectError, TransitConnector, TransitError};

/// The App ID associated with this protocol.
pub const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

// TODO be more extensible on the JSON enum types (i.e. recognize unknown variants)

// TODO send peer errors when something went wrong (if possible)
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransferError {
    #[error("Transfer was not acknowledged by peer")]
    AckError,
    #[error("Receive checksum error")]
    Checksum,
    #[error("The file contained a different amount of bytes than advertized!")]
    FileSize,
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
        WormholeCoreError,
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

/**
 * The type of message exchanged over the wormhole for this protocol
 */
#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PeerMessage {
    Offer(OfferType),
    Answer(AnswerType),
    /** Tell the other side you got an error */
    Error(String),
    /** Used to set up a transit channel */
    Transit(Arc<transit::TransitType>),
}

impl PeerMessage {
    pub fn new_offer_message(msg: impl Into<String>) -> Self {
        PeerMessage::Offer(OfferType::Message(msg.into()))
    }

    pub fn new_offer_file(name: impl Into<PathBuf>, size: u64) -> Self {
        PeerMessage::Offer(OfferType::File {
            filename: name.into(),
            filesize: size,
        })
    }

    pub fn new_message_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::MessageAck(msg.into()))
    }

    pub fn new_file_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::FileAck(msg.into()))
    }

    pub fn new_error_message(msg: impl Into<String>) -> Self {
        PeerMessage::Error(msg.into())
    }

    pub fn new_offer_directory(
        name: impl Into<PathBuf>,
        mode: impl Into<String>,
        compressed_size: u64,
        numbytes: u64,
        numfiles: u64,
    ) -> Self {
        PeerMessage::Offer(OfferType::Directory {
            dirname: name.into(),
            mode: mode.into(),
            zipsize: compressed_size,
            numbytes,
            numfiles,
        })
    }
    pub fn new_transit(abilities: Vec<transit::Ability>, hints: Vec<transit::Hint>) -> Self {
        PeerMessage::Transit(Arc::new(transit::TransitType {
            abilities_v1: abilities,
            hints_v1: hints,
        }))
    }

    #[cfg(test)]
    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    pub fn serialize_vec(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OfferType {
    Message(String),
    File {
        filename: PathBuf,
        filesize: u64,
    },
    Directory {
        dirname: PathBuf,
        mode: String,
        zipsize: u64,
        numbytes: u64,
        numfiles: u64,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    MessageAck(String),
    FileAck(String),
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
    debug!("Sending transit message '{:?}", connector.our_side_ttype());
    wormhole
        .tx
        .send(PeerMessage::Transit(connector.our_side_ttype().clone()).serialize_vec())
        .await?;

    // Send file offer message.
    debug!("Sending file offer");
    wormhole
        .tx
        .send(PeerMessage::new_offer_file(file_name, file_size).serialize_vec())
        .await?;

    // Wait for their transit response
    let other_side_ttype = {
        let maybe_transit = serde_json::from_slice(&wormhole.rx.next().await.unwrap()?)?;
        debug!("received transit message: {:?}", maybe_transit);

        match maybe_transit {
            PeerMessage::Transit(tmsg) => tmsg,
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            _ => {
                let error = TransferError::unexpected_message("transit", maybe_transit);
                let _ = wormhole
                    .tx
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        }
    };

    {
        // Wait for file_ack
        let fileack_msg = serde_json::from_slice(&wormhole.rx.next().await.unwrap()?)?;
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
                    .tx
                    .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                    .await;
                bail!(error)
            },
        }
    }

    let mut transit = connector
        .leader_connect(
            wormhole.key.derive_transit_key(&wormhole.appid),
            Arc::try_unwrap(other_side_ttype).unwrap(),
        )
        .await?;

    debug!("Beginning file transfer");

    // 11. send the file as encrypted records.
    let checksum = send_records(&mut transit, file, file_size, progress_handler).await?;

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
    debug!("Sending transit message '{:?}", connector.our_side_ttype());
    wormhole
        .tx
        .send(
            crate::transfer::PeerMessage::Transit(connector.our_side_ttype().clone())
                .serialize_vec(),
        )
        .await?;

    // receive transit message
    let other_side_ttype = match serde_json::from_slice(&wormhole.rx.next().await.unwrap()?)? {
        PeerMessage::Transit(transit) => {
            debug!("received transit message: {:?}", transit);
            transit
        },
        PeerMessage::Error(err) => {
            bail!(TransferError::PeerError(err));
        },
        other => {
            let error = TransferError::unexpected_message("transit", other);
            let _ = wormhole
                .tx
                .send(PeerMessage::Error(format!("{}", error)).serialize_vec())
                .await;
            bail!(error)
        },
    };

    // 3. receive file offer message from peer
    let maybe_offer = serde_json::from_slice(&wormhole.rx.next().await.unwrap()?)?;
    debug!("Received offer message '{:?}'", &maybe_offer);

    let (filename, filesize) = match maybe_offer {
        PeerMessage::Offer(offer_type) => match offer_type {
            OfferType::File { filename, filesize } => (filename, filesize),
            _ => bail!(TransferError::UnsupportedOffer),
        },
        PeerMessage::Error(err) => {
            bail!(TransferError::PeerError(err));
        },
        _ => {
            let error = TransferError::unexpected_message("offer", maybe_offer);
            let _ = wormhole
                .tx
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
        other_side_ttype,
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
    other_side_ttype: Arc<transit::TransitType>,
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
            .tx
            .send(PeerMessage::new_file_ack("ok").serialize_vec())
            .await?;

        let mut transit = self
            .connector
            .follower_connect(
                self.wormhole.key.derive_transit_key(&self.wormhole.appid),
                self.other_side_ttype.clone(),
            )
            .await?;

        debug!("Beginning file transfer");
        // TODO here's the right position for applying the output directory and to check for malicious (relative) file paths
        tcp_file_receive(
            &mut transit,
            self.filesize,
            progress_handler,
            content_handler,
        )
        .await
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     * You can close the wormhole afterwards.
     */
    pub async fn reject(self) -> Result<(), TransferError> {
        self.wormhole
            .tx
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
        debug!("sent {} bytes out of {} bytes", sent_size, file_size);
        progress_handler(sent_size, file_size);

        // sha256 of the input
        hasher.update(&plaintext[..n]);

        if n < 4096 {
            break;
        }
    }

    ensure!(sent_size == file_size, TransferError::FileSize);

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
