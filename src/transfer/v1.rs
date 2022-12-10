use futures::{AsyncReadExt, AsyncWriteExt};
use log::*;
use sha2::{digest::FixedOutput, Digest, Sha256};
use std::path::PathBuf;

use super::*;

pub async fn send_file<F, N, G, H>(
    mut wormhole: Wormhole,
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
    F: AsyncRead + Unpin,
    N: Into<PathBuf>,
    G: FnOnce(transit::TransitInfo, std::net::SocketAddr),
    H: FnMut(u64, u64) + 'static,
{
    let run = Box::pin(async {
        let connector = transit::init(transit_abilities, None, relay_hints).await?;

        // We want to do some transit
        debug!("Sending transit message '{:?}", connector.our_hints());
        wormhole
            .send_json(&PeerMessage::transit(
                *connector.our_abilities(),
                (**connector.our_hints()).clone(),
            ))
            .await?;

        // Send file offer message.
        debug!("Sending file offer");
        wormhole
            .send_json(&PeerMessage::offer_file(file_name, file_size))
            .await?;

        // Wait for their transit response
        let (their_abilities, their_hints): (transit::Abilities, transit::Hints) =
            match wormhole.receive_json().await?? {
                PeerMessage::Transit(transit) => {
                    debug!("Received transit message: {:?}", transit);
                    (transit.abilities_v1, transit.hints_v1)
                },
                PeerMessage::Error(err) => {
                    bail!(TransferError::PeerError(err));
                },
                other => {
                    bail!(TransferError::unexpected_message("transit", other))
                },
            };

        {
            // Wait for file_ack
            let fileack_msg = wormhole.receive_json().await??;
            debug!("Received file ack message: {:?}", fileack_msg);

            match fileack_msg {
                PeerMessage::Answer(Answer::FileAck(msg)) => {
                    ensure!(msg == "ok", TransferError::AckError);
                },
                PeerMessage::Error(err) => {
                    bail!(TransferError::PeerError(err));
                },
                _ => {
                    bail!(TransferError::unexpected_message(
                        "answer/file_ack",
                        fileack_msg
                    ));
                },
            }
        }

        let (mut transit, info, addr) = connector
            .leader_connect(
                wormhole.key().derive_transit_key(wormhole.appid()),
                their_abilities,
                Arc::new(their_hints),
            )
            .await?;
        transit_handler(info, addr);

        debug!("Beginning file transfer");

        // 11. send the file as encrypted records.
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
    let result = crate::util::cancellable_2(run, cancel).await;
    super::handle_run_result(wormhole, result).await
}

pub async fn send_folder<N, M, G, H>(
    mut wormhole: Wormhole,
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
    G: FnOnce(transit::TransitInfo, std::net::SocketAddr),
    H: FnMut(u64, u64) + 'static,
{
    let run = Box::pin(async {
        let connector = transit::init(transit_abilities, None, relay_hints).await?;
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
            .send_json(&PeerMessage::transit(
                *connector.our_abilities(),
                (**connector.our_hints()).clone(),
            ))
            .await?;

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
        log::info!("Calculating the size of '{}'", folder_path.display());
        let folder_path2 = folder_path.clone();
        let (length, sha256sum_initial) = {
            let mut hasher = Sha256::new();
            let mut counter = CountWrite {
                inner: &mut hasher,
                count: 0,
            };
            let mut builder = async_tar::Builder::new(futures::io::AllowStdIo::new(&mut counter));

            builder.mode(async_tar::HeaderMode::Deterministic);
            builder.follow_symlinks(false);
            /* A hasher should never fail writing */
            builder.append_dir_all("", folder_path2).await.unwrap();
            builder.finish().await.unwrap();

            std::mem::drop(builder);
            let count = counter.count;
            std::mem::drop(counter);
            (count, hasher.finalize_fixed())
        };

        // Send file offer message.
        debug!("Sending file offer");
        wormhole
            .send_json(&PeerMessage::offer_file(folder_name, length))
            .await?;

        // Wait for their transit response
        let (their_abilities, their_hints): (transit::Abilities, transit::Hints) =
            match wormhole.receive_json().await?? {
                PeerMessage::Transit(transit) => {
                    debug!("received transit message: {:?}", transit);
                    (transit.abilities_v1, transit.hints_v1)
                },
                PeerMessage::Error(err) => {
                    bail!(TransferError::PeerError(err));
                },
                other => {
                    bail!(TransferError::unexpected_message("transit", other));
                },
            };

        // Wait for file_ack
        match wormhole.receive_json().await?? {
            PeerMessage::Answer(Answer::FileAck(msg)) => {
                ensure!(msg == "ok", TransferError::AckError);
            },
            PeerMessage::Error(err) => {
                bail!(TransferError::PeerError(err));
            },
            other => {
                bail!(TransferError::unexpected_message("answer/file_ack", other));
            },
        }

        let (mut transit, info, addr) = connector
            .leader_connect(
                wormhole.key().derive_transit_key(wormhole.appid()),
                their_abilities,
                Arc::new(their_hints),
            )
            .await?;
        transit_handler(info, addr);

        debug!("Beginning file transfer");

        /* Inspired by https://github.com/RustCrypto/traits/pull/1159/files */
        pub struct HashWriter<D: sha2::digest::Update, W: futures::io::AsyncWrite + Unpin> {
            writer: W,
            hasher: D,
        }

        use std::{
            pin::Pin,
            task::{Context, Poll},
        };
        impl<D: sha2::digest::Update + Unpin, W: futures::io::AsyncWrite + Unpin>
            futures::io::AsyncWrite for HashWriter<D, W>
        {
            fn poll_write(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::io::Result<usize>> {
                // log::debug!("Poll write, {}", buf.len());
                match Pin::new(&mut self.writer).poll_write(cx, buf) {
                    Poll::Ready(Ok(n)) => {
                        self.hasher.update(&buf[..n]);
                        Poll::Ready(Ok(n))
                    },
                    res => res,
                }
            }

            fn poll_flush(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                // log::debug!("Poll flush");
                Pin::new(&mut self.writer).poll_flush(cx)
            }

            fn poll_close(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<()>> {
                // log::debug!("Poll close");
                Pin::new(&mut self.writer).poll_close(cx)
            }
        }

        // 11. send the file as encrypted records.
        let (mut reader, writer) = futures_ringbuf::RingBuffer::new(4096).split();

        let file_sender = async_std::task::spawn(async move {
            let mut hash_writer = HashWriter {
                writer,
                hasher: Sha256::new(),
            };
            let mut builder = async_tar::Builder::new(&mut hash_writer);

            builder.mode(async_tar::HeaderMode::Deterministic);
            builder.follow_symlinks(false);
            builder.append_dir_all("", folder_path).await?;
            builder.finish().await?;
            std::mem::drop(builder);

            hash_writer.flush().await?;
            hash_writer.close().await?;
            let hasher = hash_writer.hasher;

            std::io::Result::Ok(hasher.finalize_fixed())
        });

        let (checksum, sha256sum) =
            match v1::send_records(&mut transit, &mut reader, length, progress_handler).await {
                Ok(checksum) => (checksum, file_sender.await?),
                Err(err) => {
                    log::debug!("Some more error {err}");
                    if let Some(Err(err)) = file_sender.cancel().await {
                        log::warn!("Error in background task: {err}");
                    }
                    return Err(err);
                },
            };

        /* Check if the hash sum still matches what we advertized. Otherwise, tell the other side and bail out */
        ensure!(
            sha256sum == sha256sum_initial,
            TransferError::FilesystemSkew
        );

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
    let result = crate::util::cancellable_2(run, cancel).await;
    super::handle_run_result(wormhole, result).await
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
pub async fn send_records<F>(
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
        log::debug!("Read {n}");

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
