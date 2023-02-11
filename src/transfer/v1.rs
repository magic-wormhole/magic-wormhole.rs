use futures::{
    io::{AsyncReadExt, AsyncWriteExt},
    StreamExt, TryFutureExt,
};
use sha2::{digest::FixedOutput, Digest, Sha256};

use super::*;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum OfferMessage {
    Message(String),
    File {
        filename: String,
        filesize: u64,
    },
    Directory {
        dirname: String,
        mode: String,
        zipsize: u64,
        numbytes: u64,
        numfiles: u64,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerMessage {
    MessageAck(String),
    FileAck(String),
}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV1 {
    pub abilities_v1: TransitAbilities,
    pub hints_v1: transit::Hints,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
struct TransitAck {
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

pub async fn send(
    wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    offer: OfferSend,
    progress_handler: impl FnMut(u64, u64) + 'static,
    transit_handler: impl FnOnce(transit::TransitInfo),
    _peer_version: AppVersion,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError> {
    if offer.is_multiple() {
        let folder = OfferSendEntry::Directory {
            content: offer.content,
        };
        send_folder(
            wormhole,
            relay_hints,
            "<unnamed folder>".into(),
            folder,
            transit_abilities,
            transit_handler,
            progress_handler,
            cancel,
        )
        .await
    } else if offer.is_directory() {
        let (folder_name, folder) = offer.content.into_iter().next().unwrap();
        send_folder(
            wormhole,
            relay_hints,
            folder_name,
            folder,
            transit_abilities,
            transit_handler,
            progress_handler,
            cancel,
        )
        .await
    } else {
        let (file_name, file) = offer.content.into_iter().next().unwrap();
        let (mut file, file_size) = match file {
            OfferSendEntry::RegularFile { content, size } => {
                /* This must be split into two statements to appease the borrow checker (unfortunate side effect of borrow-through) */
                let content = content();
                let content = content.await?;
                (content, size)
            },
            _ => unreachable!(),
        };
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
        .await
    }
}

pub async fn send_file<F, G, H>(
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    file: &mut F,
    file_name: impl Into<String>,
    file_size: u64,
    transit_abilities: transit::Abilities,
    transit_handler: G,
    progress_handler: H,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError>
where
    F: AsyncRead + Unpin + Send,
    G: FnOnce(transit::TransitInfo),
    H: FnMut(u64, u64) + 'static,
{
    let run = Box::pin(async {
        let connector = transit::init(transit_abilities, None, relay_hints).await?;

        // We want to do some transit
        debug!("Sending transit message '{:?}", connector.our_hints());
        wormhole
            .send_json(&PeerMessage::transit_v1(
                *connector.our_abilities(),
                (**connector.our_hints()).clone(),
            ))
            .await?;

        // Send file offer message.
        debug!("Sending file offer");
        wormhole
            .send_json(&PeerMessage::offer_file_v1(file_name, file_size))
            .await?;

        // Wait for their transit response
        let (their_abilities, their_hints): (transit::Abilities, transit::Hints) =
            match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
                PeerMessage::Transit(transit) => {
                    debug!("Received transit message: {:?}", transit);
                    (transit.abilities_v1, transit.hints_v1)
                },
                other => {
                    bail!(TransferError::unexpected_message("transit", other))
                },
            };

        {
            // Wait for file_ack
            let fileack_msg = wormhole.receive_json::<PeerMessage>().await??;
            debug!("Received file ack message: {:?}", fileack_msg);

            match fileack_msg.check_err()? {
                PeerMessage::Answer(AnswerMessage::FileAck(msg)) => {
                    ensure!(msg == "ok", TransferError::AckError);
                },
                _ => {
                    bail!(TransferError::unexpected_message(
                        "answer/file_ack",
                        fileack_msg
                    ));
                },
            }
        }

        let (mut transit, info) = connector
            .leader_connect(
                wormhole.key().derive_transit_key(wormhole.appid()),
                their_abilities,
                Arc::new(their_hints),
            )
            .await?;
        transit_handler(info);

        debug!("Beginning file transfer");

        // 11. send the file as encrypted records.
        let file = futures::stream::once(futures::future::ready(std::io::Result::Ok(
            Box::new(file) as Box<dyn AsyncRead + Unpin + Send>,
        )));
        let checksum = v1::send_records(&mut transit, file, file_size, progress_handler).await?;

        // 13. wait for the transit ack with sha256 sum from the peer.
        debug!("sent file. Waiting for ack");
        let transit_ack = transit.receive_record().await?;
        let transit_ack_msg = serde_json::from_slice::<TransitAck>(&transit_ack)?;
        ensure!(
            transit_ack_msg.sha256 == hex::encode(checksum),
            TransferError::Checksum
        );
        debug!("Transfer complete!");

        Ok(())
    });

    futures::pin_mut!(cancel);
    let result = cancel::cancellable_2(run, cancel).await;
    cancel::handle_run_result(wormhole, result).await
}

pub async fn send_folder(
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    mut folder_name: String,
    folder: OfferSendEntry,
    transit_abilities: transit::Abilities,
    transit_handler: impl FnOnce(transit::TransitInfo),
    progress_handler: impl FnMut(u64, u64) + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError> {
    let run = Box::pin(async {
        let connector = transit::init(transit_abilities, None, relay_hints).await?;

        // We want to do some transit
        debug!("Sending transit message '{:?}", connector.our_hints());
        wormhole
            .send_json(&PeerMessage::transit_v1(
                *connector.our_abilities(),
                (**connector.our_hints()).clone(),
            ))
            .await?;

        /* We need to know the length of what we are going to send in advance. So we already build
         * all the headers of our file now but without the contents. We know that a file is
         * header + contents + padding
         */
        log::debug!("Estimating the file size");

        // TODO try again but without pinning
        use futures::{
            future::{ready, BoxFuture},
            io::Cursor,
        };
        use std::io::Result as IoResult;

        /* Type tetris :) */
        fn wrap(
            buffer: impl AsRef<[u8]> + Unpin + Send + 'static,
        ) -> BoxFuture<'static, IoResult<Box<dyn AsyncRead + Unpin + Send>>> {
            Box::pin(ready(IoResult::Ok(
                Box::new(Cursor::new(buffer)) as Box<dyn AsyncRead + Unpin + Send>
            ))) as _
        }

        /* Walk our offer recursively, concatenate all our readers into a stream that will build the tar file */
        fn create_offer(
            mut total_content: Vec<
                BoxFuture<'static, IoResult<Box<dyn AsyncRead + Unpin + Send + 'static>>>,
            >,
            total_size: &mut u64,
            offer: OfferSendEntry,
            path: &mut Vec<String>,
        ) -> IoResult<Vec<BoxFuture<'static, IoResult<Box<dyn AsyncRead + Unpin + Send + 'static>>>>>
        {
            match offer {
                OfferSendEntry::Directory { content } => {
                    log::debug!("Adding directory {path:?}");
                    let header = tar_helper::create_header_directory(path)?;
                    *total_size += header.len() as u64;
                    total_content.push(wrap(header));

                    for (name, file) in content {
                        path.push(name);
                        total_content = create_offer(total_content, total_size, file, path)?;
                        path.pop();
                    }
                },
                OfferSendEntry::RegularFile { size, content } => {
                    log::debug!("Adding file {path:?}; {size} bytes");
                    let header = tar_helper::create_header_file(&path, size)?;
                    let padding = tar_helper::padding(size);
                    *total_size += header.len() as u64;
                    *total_size += padding.len() as u64;
                    *total_size += size;

                    total_content.push(wrap(header));
                    let content = content().map_ok(
                        /* Re-box because we can't upcast trait objects */
                        |read| Box::new(read) as Box<dyn AsyncRead + Unpin + Send>,
                    );
                    total_content.push(Box::pin(content) as _);
                    total_content.push(wrap(padding));
                },
                OfferSendEntry::Symlink { .. } => todo!(),
            }
            Ok(total_content)
        }

        let mut total_size = 0;
        let mut content = create_offer(
            Vec::new(),
            &mut total_size,
            folder,
            &mut vec![folder_name.clone()],
        )?;

        /* Finish tar file */
        total_size += 1024;
        content.push(wrap([0; 1024]));

        let content = futures::stream::iter(content).then(|content| async { content.await });

        /* Convert to stream */

        // Send file offer message.
        log::debug!("Sending file offer ({total_size} bytes)");
        folder_name.push_str(".tar");
        wormhole
            .send_json(&PeerMessage::offer_file_v1(folder_name, total_size))
            .await?;

        // Wait for their transit response
        let (their_abilities, their_hints): (transit::Abilities, transit::Hints) =
            match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
                PeerMessage::Transit(transit) => {
                    debug!("received transit message: {:?}", transit);
                    (transit.abilities_v1, transit.hints_v1)
                },
                other => {
                    bail!(TransferError::unexpected_message("transit", other));
                },
            };

        // Wait for file_ack
        match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
            PeerMessage::Answer(AnswerMessage::FileAck(msg)) => {
                ensure!(msg == "ok", TransferError::AckError);
            },
            other => {
                bail!(TransferError::unexpected_message("answer/file_ack", other));
            },
        }

        let (mut transit, info) = connector
            .leader_connect(
                wormhole.key().derive_transit_key(wormhole.appid()),
                their_abilities,
                Arc::new(their_hints),
            )
            .await?;
        transit_handler(info);

        debug!("Beginning file transfer");

        // 11. send the file as encrypted records.
        let checksum =
            v1::send_records(&mut transit, content, total_size, progress_handler).await?;

        // 13. wait for the transit ack with sha256 sum from the peer.
        debug!("sent file. Waiting for ack");
        let transit_ack = transit.receive_record().await?;
        let transit_ack_msg = serde_json::from_slice::<TransitAck>(&transit_ack)?;
        ensure!(
            transit_ack_msg.sha256 == hex::encode(checksum),
            TransferError::Checksum
        );
        debug!("Transfer complete!");

        Ok(())
    });

    futures::pin_mut!(cancel);
    let result = cancel::cancellable_2(run, cancel).await;
    cancel::handle_run_result(wormhole, result).await
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
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    cancel: impl Future<Output = ()>,
) -> Result<Option<ReceiveRequest>, TransferError> {
    // Error handling
    let run = Box::pin(async {
        let connector = transit::init(transit_abilities, None, relay_hints).await?;

        // send the transit message
        debug!("Sending transit message '{:?}", connector.our_hints());
        wormhole
            .send_json(&PeerMessage::transit_v1(
                *connector.our_abilities(),
                (**connector.our_hints()).clone(),
            ))
            .await?;

        // receive transit message
        let (their_abilities, their_hints): (transit::Abilities, transit::Hints) =
            match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
                PeerMessage::Transit(transit) => {
                    debug!("received transit message: {:?}", transit);
                    (transit.abilities_v1, transit.hints_v1)
                },
                other => {
                    bail!(TransferError::unexpected_message("transit", other));
                },
            };

        // 3. receive file offer message from peer
        let (filename, filesize) =
            match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
                PeerMessage::Offer(offer_type) => match offer_type {
                    v1::OfferMessage::File { filename, filesize } => (filename, filesize),
                    v1::OfferMessage::Directory {
                        mut dirname,
                        zipsize,
                        ..
                    } => {
                        dirname.push_str(".zip");
                        (dirname, zipsize)
                    },
                    _ => bail!(TransferError::UnsupportedOffer),
                },
                other => {
                    bail!(TransferError::unexpected_message("offer", other));
                },
            };

        Ok((filename, filesize, connector, their_abilities, their_hints))
    });

    futures::pin_mut!(cancel);
    let result = cancel::cancellable_2(run, cancel).await;
    cancel::handle_run_result_noclose(wormhole, result)
        .await
        .map(|inner: Option<_>| {
            inner.map(
                |((filename, filesize, connector, their_abilities, their_hints), wormhole, _)| {
                    ReceiveRequest {
                        wormhole,
                        filename,
                        filesize,
                        connector,
                        their_abilities,
                        their_hints: Arc::new(their_hints),
                    }
                },
            )
        })
}

/**
 * A pending files send offer from the other side
 *
 * You *should* consume this object, either by calling [`accept`](ReceiveRequest::accept) or [`reject`](ReceiveRequest::reject).
 */
#[must_use]
pub struct ReceiveRequest {
    wormhole: Wormhole,
    connector: TransitConnector,
    /// **Security warning:** this is untrusted and unverified input
    pub filename: String,
    pub filesize: u64,
    their_abilities: transit::Abilities,
    their_hints: Arc<transit::Hints>,
}

impl ReceiveRequest {
    /**
     * Accept the file offer
     *
     * This will transfer the file and save it on disk.
     */
    pub async fn accept<F, G, W>(
        mut self,
        transit_handler: G,
        content_handler: &mut W,
        progress_handler: F,
        cancel: impl Future<Output = ()>,
    ) -> Result<(), TransferError>
    where
        F: FnMut(u64, u64) + 'static,
        G: FnOnce(transit::TransitInfo),
        W: AsyncWrite + Unpin,
    {
        let run = Box::pin(async {
            // send file ack.
            debug!("Sending ack");
            self.wormhole
                .send_json(&PeerMessage::file_ack_v1("ok"))
                .await?;

            let (mut transit, info) = self
                .connector
                .follower_connect(
                    self.wormhole
                        .key()
                        .derive_transit_key(self.wormhole.appid()),
                    self.their_abilities,
                    self.their_hints.clone(),
                )
                .await?;
            transit_handler(info);

            debug!("Beginning file transfer");
            tcp_file_receive(
                &mut transit,
                self.filesize,
                progress_handler,
                content_handler,
            )
            .await?;
            Ok(())
        });

        futures::pin_mut!(cancel);
        let result = cancel::cancellable_2(run, cancel).await;
        cancel::handle_run_result(self.wormhole, result).await
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     */
    pub async fn reject(mut self) -> Result<(), TransferError> {
        self.wormhole
            .send_json(&PeerMessage::error_message("transfer rejected"))
            .await?;
        self.wormhole.close().await?;

        Ok(())
    }
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
pub async fn send_records<'a>(
    transit: &mut Transit,
    files: impl futures::Stream<Item = std::io::Result<Box<dyn AsyncRead + Unpin + Send + 'a>>>,
    file_size: u64,
    mut progress_handler: impl FnMut(u64, u64) + 'static,
) -> Result<Vec<u8>, TransferError> {
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
    futures::pin_mut!(files);
    while let Some(mut file) = files.next().await.transpose()? {
        loop {
            // read a block of up to 4096 bytes
            let n = file.read(&mut plaintext[..]).await?;

            if n == 0 {
                // EOF
                break;
            }

            // send the encrypted record
            transit.send_record(&plaintext[0..n]).await?;
            sent_size += n as u64;
            progress_handler(sent_size, file_size);

            // sha256 of the input
            hasher.update(&plaintext[..n]);

            /* Don't do this. The EOF check above is sufficient */
            // if n < 4096 {
            //     break;
            // }
        }
    }
    transit.flush().await?;

    ensure!(
        sent_size == file_size,
        TransferError::FileSize {
            sent_size,
            file_size
        }
    );

    Ok(hasher.finalize_fixed().to_vec())
}

pub async fn receive_records<F, W>(
    filesize: u64,
    transit: &mut Transit,
    mut progress_handler: F,
    mut content_handler: W,
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
    content_handler.close().await?;

    debug!("done");
    // TODO: 5. write the buffer into a file.
    Ok(hasher.finalize_fixed().to_vec())
}

pub async fn tcp_file_receive<F, W>(
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

/// Custom functions from the `tar` crate to access internals
mod tar_helper {
    /* Imports may depend on target platform */
    #[allow(unused_imports)]
    use std::{
        borrow::Cow,
        io::{self, Read, Write},
        path::Path,
        str,
    };

    pub fn create_header_file(path: &[String], size: u64) -> std::io::Result<Vec<u8>> {
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        let mut data = Vec::with_capacity(1024);
        prepare_header_path(&mut data, &mut header, path.join("/").as_ref())?;
        header.set_mode(0o644);
        header.set_cksum();
        data.write_all(header.as_bytes())?;
        Ok(data)
    }

    pub fn create_header_directory(path: &[String]) -> std::io::Result<Vec<u8>> {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        let mut data = Vec::with_capacity(1024);
        prepare_header_path(&mut data, &mut header, path.join("/").as_ref())?;
        header.set_mode(0o755);
        header.set_cksum();
        data.write_all(header.as_bytes())?;
        // append(&mut data, header, data)?;
        Ok(data)
    }

    pub fn padding(size: u64) -> &'static [u8] {
        const BLOCK: [u8; 512] = [0; 512];
        if size % 512 != 0 {
            &BLOCK[size as usize % 512..]
        } else {
            &[]
        }
    }

    fn append(
        mut dst: &mut dyn std::io::Write,
        header: &tar::Header,
        mut data: &mut dyn std::io::Read,
    ) -> std::io::Result<()> {
        dst.write_all(header.as_bytes())?;
        let len = std::io::copy(&mut data, &mut dst)?;
        dst.write_all(padding(len))?;
        Ok(())
    }

    fn prepare_header(size: u64, entry_type: u8) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        let name = b"././@LongLink";
        header.as_gnu_mut().unwrap().name[..name.len()].clone_from_slice(&name[..]);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        // + 1 to be compliant with GNU tar
        header.set_size(size + 1);
        header.set_entry_type(tar::EntryType::new(entry_type));
        header.set_cksum();
        header
    }

    fn prepare_header_path(
        dst: &mut dyn std::io::Write,
        header: &mut tar::Header,
        path: &str,
    ) -> std::io::Result<()> {
        // Try to encode the path directly in the header, but if it ends up not
        // working (probably because it's too long) then try to use the GNU-specific
        // long name extension by emitting an entry which indicates that it's the
        // filename.
        if let Err(e) = header.set_path(path) {
            let data = path2bytes(&path);
            let max = header.as_old().name.len();
            // Since `e` isn't specific enough to let us know the path is indeed too
            // long, verify it first before using the extension.
            if data.len() < max {
                return Err(e);
            }
            let header2 = prepare_header(data.len() as u64, b'L');
            // null-terminated string
            let mut data2 = data.chain(io::repeat(0).take(1));
            append(dst, &header2, &mut data2)?;

            // Truncate the path to store in the header we're about to emit to
            // ensure we've got something at least mentioned. Note that we use
            // `str`-encoding to be compatible with Windows, but in general the
            // entry in the header itself shouldn't matter too much since extraction
            // doesn't look at it.
            let truncated = match std::str::from_utf8(&data[..max]) {
                Ok(s) => s,
                Err(e) => std::str::from_utf8(&data[..e.valid_up_to()]).unwrap(),
            };
            header.set_path(truncated)?;
        }
        Ok(())
    }

    #[cfg(any(windows, target_arch = "wasm32"))]
    pub fn path2bytes(p: &str) -> Cow<[u8]> {
        let bytes = p.as_bytes();
        if bytes.contains(&b'\\') {
            // Normalize to Unix-style path separators
            let mut bytes = bytes.to_owned();
            for b in &mut bytes {
                if *b == b'\\' {
                    *b = b'/';
                }
            }
            Cow::Owned(bytes)
        } else {
            Cow::Borrowed(bytes)
        }
    }

    #[cfg(unix)]
    pub fn path2bytes(p: &str) -> Cow<[u8]> {
        Cow::Borrowed(p.as_bytes())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_transit_ack() {
        let f1 = TransitAck::new("ok", "deadbeaf");
        assert_eq!(f1.serialize(), "{\"ack\":\"ok\",\"sha256\":\"deadbeaf\"}");
    }
}
