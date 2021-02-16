//! Client-to-Client protocol to organize file transfers
//!
//! This gives you the actual capability to transfer files, that feature that Magic Wormhole got known and loved for.
//!
//! It is bound to an [`APPID`](APPID). Only applications using that APPID (and thus this protocol) can interoperate with
//! the original Python implementation (and other compliant implementations).
//!
//! At its core, [`PeerMessage`s](PeerMessage) are exchanged over an established wormhole connection with the other side.
//! They are used to set up a [transit] portal and to exchange a file offer/accept. Then, the file is transmitted over the transit relay.

use async_std::fs::OpenOptions;
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

use super::{
    transit,
    transit::{RelayUrl, Transit},
    Wormhole,
};
use anyhow::{bail, ensure, format_err, Context, Result};
use async_std::{
    fs::File,
    io::{prelude::WriteExt, ReadExt},
};
use futures::{SinkExt, StreamExt};
use log::*;
use sha2::{digest::FixedOutput, Digest, Sha256};
use std::path::{Path, PathBuf};
use transit::TransitConnector;

/// The App ID associated with this protocol.
pub const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

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

    pub fn serialize(&self) -> String {
        json!(self).to_string()
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

    pub fn serialize(&self) -> String {
        json!(self).to_string()
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
pub async fn send_file(
    wormhole: &mut Wormhole,
    filepath: impl AsRef<Path>,
    relay_url: &RelayUrl,
) -> Result<()> {
    let filename = filepath
        .as_ref()
        .file_name()
        .ok_or_else(|| format_err!("You can't send a file without a file name"))?;
    let filesize = File::open(filepath.as_ref()).await?.metadata().await?.len(); // TODO do that somewhere else

    let connector = transit::init(transit::Ability::all_abilities(), relay_url).await?;

    // We want to do some transit
    debug!("Sending transit message '{:?}", connector.our_side_ttype());
    wormhole
        .tx
        .send(
            PeerMessage::Transit(connector.our_side_ttype().clone())
                .serialize()
                .as_bytes()
                .to_vec(),
        )
        .await?;

    // Send file offer message.
    debug!("Sending file offer");
    wormhole
        .tx
        .send(
            PeerMessage::new_offer_file(filename, filesize)
                .serialize()
                .as_bytes()
                .to_vec(),
        )
        .await?;

    // Wait for their transit response
    let other_side_ttype = {
        let maybe_transit =
            serde_json::from_str(std::str::from_utf8(&wormhole.rx.next().await.unwrap()?)?)?;
        debug!("received transit message: {:?}", maybe_transit);

        match maybe_transit {
            PeerMessage::Transit(tmsg) => tmsg,
            _ => bail!(format_err!("unexpected message: {:?}", maybe_transit)),
        }
    };

    {
        // Wait for file_ack
        let fileack_msg =
            serde_json::from_str(std::str::from_utf8(&wormhole.rx.next().await.unwrap()?)?)?;
        debug!("received file ack message: {:?}", fileack_msg);

        match fileack_msg {
            PeerMessage::Answer(AnswerType::FileAck(msg)) => {
                ensure!(msg == "ok", "file ack failed");
            },
            _ => bail!("did not receive file ack"),
        }
    }

    let mut transit = connector
        .leader_connect(
            wormhole.key.derive_transit_key(&wormhole.appid),
            Arc::try_unwrap(other_side_ttype).unwrap(),
        )
        .await?;
    debug!("Beginning file transfer");

    tcp_file_send(&mut transit, &filepath)
        .await
        .context("Could not send file")
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
) -> Result<ReceiveRequest<'a>> {
    let connector = transit::init(transit::Ability::all_abilities(), relay_url).await?;

    // send the transit message
    debug!("Sending transit message '{:?}", connector.our_side_ttype());
    let transit_msg =
        crate::transfer::PeerMessage::Transit(connector.our_side_ttype().clone()).serialize();
    wormhole.tx.send(transit_msg.as_bytes().to_vec()).await?;

    // receive transit message
    let other_side_ttype =
        match serde_json::from_str(std::str::from_utf8(&wormhole.rx.next().await.unwrap()?)?)? {
            PeerMessage::Transit(transit) => {
                debug!("received transit message: {:?}", transit);
                transit
            },
            PeerMessage::Error(err) => {
                anyhow::bail!("Something went wrong on the other side: {}", err);
            },
            other => {
                anyhow::bail!(
                    "Got an unexpected message type, is the other side all right? Got: '{:?}'",
                    other
                );
            },
        };

    // 3. receive file offer message from peer
    let maybe_offer =
        serde_json::from_str(std::str::from_utf8(&wormhole.rx.next().await.unwrap()?)?)?;
    debug!("Received offer message '{:?}'", &maybe_offer);

    let (filename, filesize) = match maybe_offer {
        PeerMessage::Offer(offer_type) => match offer_type {
            OfferType::File { filename, filesize } => (filename, filesize),
            _ => bail!("unsupported offer type"),
        },
        _ => bail!("unexpected message: {:?}", maybe_offer),
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
    pub async fn accept(self, overwrite: bool) -> Result<()> {
        // send file ack.
        debug!("Sending ack");
        self.wormhole
            .tx
            .send(
                PeerMessage::new_file_ack("ok")
                    .serialize()
                    .as_bytes()
                    .to_vec(),
            )
            .await?;

        let mut transit = self
            .connector
            .follower_connect(
                self.wormhole.key.derive_transit_key(&self.wormhole.appid),
                Arc::try_unwrap(self.other_side_ttype.clone()).unwrap(),
            )
            .await?;

        debug!("Beginning file transfer");
        // TODO here's the right position for applying the output directory and to check for malicious (relative) file paths
        tcp_file_receive(&mut transit, &self.filename, self.filesize, overwrite)
            .await
            .context("Could not receive file")
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     * You can close the wormhole afterwards.
     */
    pub async fn reject(self) -> Result<()> {
        self.wormhole
            .tx
            .send(
                PeerMessage::new_error_message("transfer rejected")
                    .serialize()
                    .as_bytes()
                    .to_vec(),
            )
            .await?;

        Ok(())
    }
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
async fn send_records(filepath: impl AsRef<Path>, transit: &mut Transit) -> Result<Vec<u8>> {
    // rough plan:
    // 1. Open the file
    // 2. read a block of N bytes
    // 3. calculate a rolling sha256sum.
    // 4. AEAD with skey and with nonce as a counter from 0.
    // 5. send the encrypted buffer to the socket.
    // 6. go to step #2 till eof.
    // 7. if eof, return sha256 sum.

    let mut file = File::open(&filepath.as_ref())
        .await
        .context(format!("Could not open {}", &filepath.as_ref().display()))?;
    debug!("Sending file size {}", file.metadata().await?.len());

    let mut hasher = Sha256::default();

    // Yeah, maybe don't allocate 4kiB on the stack…
    let mut plaintext = Box::new([0u8; 4096]);
    loop {
        // read a block of 4096 bytes
        let n = file.read(&mut plaintext[..]).await?;
        debug!("sending {} bytes", n);

        // send the encrypted record
        transit.send_record(&plaintext[0..n]).await?;

        // sha256 of the input
        hasher.update(&plaintext[..n]);

        if n < 4096 {
            break;
        }
    }
    Ok(hasher.finalize_fixed().to_vec())
}

async fn receive_records(
    filepath: impl AsRef<Path>,
    filesize: u64,
    transit: &mut Transit,
    overwrite: bool,
) -> Result<Vec<u8>> {
    let mut hasher = Sha256::default();
    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true)
    } else {
        options.create_new(true)
    };
    let mut f = options.open(filepath.as_ref()).await?;
    let mut remaining_size = filesize as usize;

    while remaining_size > 0 {
        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = transit.receive_record().await?;

        f.write_all(&plaintext).await?;

        // 4. calculate a rolling sha256 sum of the decrypted output.
        hasher.update(&plaintext);

        remaining_size -= plaintext.len();
    }

    debug!("done");
    // TODO: 5. write the buffer into a file.
    Ok(hasher.finalize_fixed().to_vec())
}

async fn tcp_file_send(transit: &mut Transit, filepath: impl AsRef<Path>) -> Result<()> {
    // 11. send the file as encrypted records.
    let checksum = send_records(filepath, transit).await?;

    // 13. wait for the transit ack with sha256 sum from the peer.
    debug!("sent file. Waiting for ack");
    let transit_ack = transit.receive_record().await?;
    let transit_ack_msg = serde_json::from_str::<TransitAck>(std::str::from_utf8(&transit_ack)?)?;
    ensure!(
        transit_ack_msg.sha256 == hex::encode(checksum),
        "receive checksum error"
    );
    debug!("transfer complete!");
    Ok(())
}

async fn tcp_file_receive(
    transit: &mut Transit,
    filepath: impl AsRef<Path>,
    filesize: u64,
    overwrite: bool,
) -> Result<()> {
    // 5. receive encrypted records
    // now skey and rkey can be used. skey is used by the tx side, rkey is used
    // by the rx side for symmetric encryption.
    let checksum = receive_records(filepath, filesize, transit, overwrite).await?;

    let sha256sum = hex::encode(checksum.as_slice());
    debug!("sha256 sum: {:?}", sha256sum);

    // 6. verify sha256 sum by sending an ack message to peer along with checksum.
    let plaintext = TransitAck::new("ok", &sha256sum).serialize();
    transit.send_record(plaintext.as_bytes()).await?;

    // 7. close socket.
    // well, no need, it gets dropped when it goes out of scope.
    debug!("Transfer complete");
    Ok(())
}

#[cfg_attr(tarpaulin, skip)]
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
