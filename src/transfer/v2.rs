use futures::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use serde_derive::{Deserialize, Serialize};
use sha2::{digest::FixedOutput, Sha256};

use super::*;

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV2 {
    pub hints_v2: transit::Hints,
}

/**
 * The type of message exchanged over the transit connection, serialized with msgpack
 */
#[derive(Deserialize, Serialize, derive_more::Display, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PeerMessageV2 {
    #[display(fmt = "offer")]
    Offer(Offer),
    #[display(fmt = "answer")]
    Answer(AnswerMessage),
    #[display(fmt = "file-start")]
    FileStart(FileStart),
    #[display(fmt = "payload")]
    Payload(Payload),
    #[display(fmt = "file-end")]
    FileEnd(FileEnd),
    #[display(fmt = "transfer-ack")]
    TransferAck(TransferAck),
    #[display(fmt = "error")]
    Error(String),
    #[display(fmt = "unknown")]
    #[serde(other)]
    Unknown,
}

impl PeerMessageV2 {
    pub fn ser_msgpack(&self) -> Vec<u8> {
        let mut writer = Vec::with_capacity(128);
        let mut ser = rmp_serde::encode::Serializer::new(&mut writer)
            .with_struct_map()
            .with_human_readable();
        serde::Serialize::serialize(self, &mut ser).unwrap();
        writer
    }

    pub fn de_msgpack(data: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_read(&mut &*data)
    }

    pub fn check_err(self) -> Result<Self, TransferError> {
        match self {
            Self::Error(err) => Err(TransferError::PeerError(err)),
            other => Ok(other),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct AnswerMessage {
    pub(self) files: Vec<AnswerMessageInner>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
struct AnswerMessageInner {
    pub file: Vec<String>,
    pub offset: u64,
    pub sha256: Option<[u8; 32]>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct FileStart {
    pub file: Vec<String>,
    pub start_at_offset: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Payload {
    payload: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct FileEnd {}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct TransferAck {}

/** The code to establish a transit connection is essentially the same on both sides. */
async fn make_transit(
    wormhole: &mut Wormhole,
    is_leader: bool,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    peer_abilities: transit::Abilities,
) -> Result<(transit::Transit, transit::TransitInfo), TransferError> {
    let connector = transit::init(transit_abilities, Some(peer_abilities), relay_hints).await?;

    /* Send our transit hints */
    wormhole
        .send_json(&PeerMessage::transit_v2((**connector.our_hints()).clone()))
        .await?;

    /* Receive their transit hints */
    let their_hints: transit::Hints =
        match wormhole.receive_json::<PeerMessage>().await??.check_err()? {
            PeerMessage::TransitV2(transit) => {
                debug!("received transit message: {:?}", transit);
                transit.hints_v2
            },
            other => {
                let error = TransferError::unexpected_message("transit-v2", other);
                let _ = wormhole
                    .send_json(&PeerMessage::Error(format!("{}", error)))
                    .await;
                bail!(error)
            },
        };

    /* Get a transit connection */
    let (transit, info) = match connector
        .connect(
            is_leader,
            wormhole.key().derive_transit_key(wormhole.appid()),
            peer_abilities,
            Arc::new(their_hints),
        )
        .await
    {
        Ok(transit) => transit,
        Err(error) => {
            let error = TransferError::TransitConnect(error);
            let _ = wormhole
                .send_json(&PeerMessage::Error(format!("{}", error)))
                .await;
            return Err(error);
        },
    };

    Ok((transit, info))
}

pub async fn send(
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    transit_abilities: transit::Abilities,
    offer: OfferSend,
    progress_handler: impl FnMut(u64, u64) + 'static,
    peer_version: AppVersion,
    cancel: impl Future<Output = ()>,
) -> Result<(), TransferError> {
    let peer_abilities = peer_version.transfer_v2.unwrap();
    futures::pin_mut!(cancel);

    /* Establish transit connection, close the Wormhole and switch to using the transit connection (msgpack instead of json) */
    let (mut transit, mut wormhole, cancel) = cancel::with_cancel_wormhole!(
        wormhole,
        run = async {
            Ok(make_transit(
                &mut wormhole,
                true,
                relay_hints,
                transit_abilities,
                peer_abilities.transit_abilities,
            )
            .await?
            .0)
        },
        cancel,
        ret_cancel = (),
    );

    cancel::with_cancel_transit!(
        transit,
        run = async {
            /* Close the wormhole only here so that the operation may be cancelled */
            wormhole.close().await?;

            send_inner(&mut transit, offer, progress_handler).await
        },
        cancel,
        |err| PeerMessageV2::Error(err.to_string()).ser_msgpack(),
        |msg| match PeerMessageV2::de_msgpack(msg)? {
            PeerMessageV2::Error(err) => Ok(Some(err)),
            _ => Ok(None),
        },
        ret_cancel = (),
    );

    Ok(())
}

/** We've established the transit connection and closed the Wormhole */
async fn send_inner(
    transit: &mut transit::Transit,
    offer: OfferSend,
    mut progress_handler: impl FnMut(u64, u64) + 'static,
) -> Result<(), TransferError> {
    transit.send_record(&{
        /* This must be split into two statements to appease the borrow checker (unfortunate side effect of borrow-through) */
        PeerMessageV2::Offer((&offer).into()).ser_msgpack()
    }).await?;

    let files = match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?.check_err()? {
        PeerMessageV2::Answer(answer) => answer.files,
        other => {
            bail!(TransferError::unexpected_message("answer", other))
        },
    };

    let mut total_size = 0;
    for file in &files {
        if let Some((_, size)) = offer.get_file(&file.file) {
            total_size += size;
        } else {
            bail!(TransferError::Protocol(
                format!("Invalid file request: {}", file.file.join("/")).into()
            ));
        }
    }
    let mut total_sent = 0;

    // use zstd::stream::raw::Encoder;
    // let zstd = Encoder::new(zstd::DEFAULT_COMPRESSION_LEVEL);
    const BUFFER_LEN: usize = 16 * 1024;
    let mut buffer = Box::new([0u8; BUFFER_LEN]);

    for AnswerMessageInner {
        file,
        offset,
        sha256,
    } in &files
    {
        let offset = *offset;
        /* This must be split into two statements to appease the borrow checker (unfortunate side effect of borrow-through) */
        let content = (offer.get_file(file).unwrap().0)();
        let mut content = content.await?;
        let file = file.clone();

        /* If they specified a hash, check our local file's contents */
        if let Some(sha256) = sha256 {
            content.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut hasher = Sha256::default();
            futures::io::copy(
                (&mut content).take(offset),
                &mut futures::io::AllowStdIo::new(&mut hasher),
            )
            .await?;
            let our_hash = hasher.finalize_fixed();

            /* If it doesn't match, start at 0 instead of the originally requested offset */
            if *our_hash == sha256[..] {
                transit
                    .send_record(
                        &PeerMessageV2::FileStart(FileStart {
                            file,
                            start_at_offset: true,
                        })
                        .ser_msgpack(),
                    )
                    .await?;
            } else {
                transit
                    .send_record(
                        &PeerMessageV2::FileStart(FileStart {
                            file,
                            start_at_offset: false,
                        })
                        .ser_msgpack(),
                    )
                    .await?;
                content.seek(std::io::SeekFrom::Start(0)).await?;
                // offset = 0; TODO
            }
        } else {
            content.seek(std::io::SeekFrom::Start(offset)).await?;
            transit
                .send_record(
                    &PeerMessageV2::FileStart(FileStart {
                        file,
                        start_at_offset: true,
                    })
                    .ser_msgpack(),
                )
                .await?;
        }

        progress_handler(total_sent, total_size);
        loop {
            let n = content.read(&mut buffer[..]).await?;
            let buffer = &buffer[..n];

            if n == 0 {
                // EOF
                break;
            }

            transit
                .send_record(
                    &PeerMessageV2::Payload(Payload {
                        payload: buffer.into(),
                    })
                    .ser_msgpack(),
                )
                .await?;
            total_sent += n as u64;
            progress_handler(total_sent, total_size);

            if n < BUFFER_LEN {
                break;
            }
        }

        transit
            .send_record(&PeerMessageV2::FileEnd(FileEnd {}).ser_msgpack())
            .await?;
    }
    transit
        .send_record(&PeerMessageV2::TransferAck(TransferAck {}).ser_msgpack())
        .await?;

    Ok(())
}

pub async fn request(
    mut wormhole: Wormhole,
    relay_hints: Vec<transit::RelayHint>,
    peer_version: AppVersion,
    transit_abilities: transit::Abilities,
    cancel: impl Future<Output = ()>,
) -> Result<Option<ReceiveRequest>, TransferError> {
    let peer_abilities = peer_version.transfer_v2.unwrap();
    futures::pin_mut!(cancel);

    /* Establish transit connection, close the Wormhole and switch to using the transit connection (msgpack instead of json) */
    let ((mut transit, info), mut wormhole, cancel) = cancel::with_cancel_wormhole!(
        wormhole,
        run = async {
            make_transit(
                &mut wormhole,
                false,
                relay_hints,
                transit_abilities,
                peer_abilities.transit_abilities,
            )
            .await
        },
        cancel,
        ret_cancel = None,
    );

    let (offer, transit) = cancel::with_cancel_transit!(
        transit,
        run = async {
            /* Close the wormhole only here so that the `.await` is scoped within cancellation */
            wormhole.close().await?;

            let offer =
                match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?.check_err()? {
                    PeerMessageV2::Offer(offer) => offer,
                    other => {
                        bail!(TransferError::unexpected_message("offer", other))
                    },
                };

            Ok(offer)
        },
        cancel,
        |err| PeerMessageV2::Error(err.to_string()).ser_msgpack(),
        |msg| match PeerMessageV2::de_msgpack(msg)? {
            PeerMessageV2::Error(err) => Ok(Some(err)),
            _ => Ok(None),
        },
        ret_cancel = None,
    );

    Ok(Some(ReceiveRequest::new(transit, offer, info)))
}

/**
 * A pending files send offer from the other side
 *
 * You *should* consume this object, either by calling [`accept`](ReceiveRequest::accept) or [`reject`](ReceiveRequest::reject).
 */
#[must_use]
pub struct ReceiveRequest {
    transit: Transit,
    offer: Arc<Offer>,
    info: transit::TransitInfo,
}

impl ReceiveRequest {
    pub fn new(transit: Transit, offer: Offer, info: transit::TransitInfo) -> Self {
        Self {
            transit,
            offer: Arc::new(offer),
            info,
        }
    }

    /** The offer we got */
    pub fn offer(&self) -> Arc<Offer> {
        self.offer.clone()
    }

    /**
     * Accept the file offer
     *
     * This will transfer the file and save it on disk.
     */
    pub async fn accept(
        self,
        transit_handler: impl FnOnce(transit::TransitInfo),
        answer: OfferAccept,
        progress_handler: impl FnMut(u64, u64) + 'static,
        cancel: impl Future<Output = ()>,
    ) -> Result<(), TransferError> {
        transit_handler(self.info);
        futures::pin_mut!(cancel);

        let mut transit = self.transit;
        cancel::with_cancel_transit!(
            transit,
            run = async {
                transit.send_record(&{
                    /* This must be split into two statements to appease the borrow checker (unfortunate side effect of borrow-through) */
                    let msg = PeerMessageV2::Answer(AnswerMessage {
                    files: answer.iter_files()
                        .map(|(path, inner, _size)| AnswerMessageInner {
                            file: path,
                            offset: inner.offset,
                            sha256: inner.sha256,
                        })
                        .collect(),
                    }).ser_msgpack();
                    msg
                }).await?;

                receive_inner(&mut transit, &self.offer, answer, progress_handler).await
            },
            cancel,
            |err| PeerMessageV2::Error(err.to_string()).ser_msgpack(),
            |msg| match PeerMessageV2::de_msgpack(msg)? {
                PeerMessageV2::Error(err) => Ok(Some(err)),
                _ => Ok(None),
            },
            ret_cancel = (),
        );
        Ok(())
    }

    /**
     * Reject the file offer
     *
     * This will send an error message to the other side so that it knows the transfer failed.
     */
    pub async fn reject(mut self) -> Result<(), TransferError> {
        self.transit
            .send_record(&PeerMessageV2::Error("transfer rejected".into()).ser_msgpack())
            .await?;
        self.transit.flush().await?;

        Ok(())
    }
}

/** We've established the transit connection and closed the Wormhole */
async fn receive_inner(
    transit: &mut transit::Transit,
    offer: &Arc<Offer>,
    our_answer: OfferAccept,
    mut progress_handler: impl FnMut(u64, u64) + 'static,
) -> Result<(), TransferError> {
    /* This does not check for file sizes, but should be good enough
     * (failures will eventually lead to protocol errors later on anyways)
     */
    assert!(
        our_answer
            .iter_file_paths()
            .all(|path| offer.get_file(&path).is_some()),
        "Mismatch between offer and accept: accept must be a true subset of offer"
    );
    let n_accepted = our_answer.iter_file_paths().count();
    let total_size = our_answer
        .iter_files()
        .map(|(_path, _inner, size)| size)
        .sum::<u64>();
    let mut total_received = 0;

    /* The receive loop */
    for (i, (file, answer, size)) in our_answer.into_iter_files().enumerate() {
        let file_start = match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?
            .check_err()?
        {
            PeerMessageV2::FileStart(file_start) => file_start,
            PeerMessageV2::TransferAck(_) => {
                bail!(TransferError::Protocol(format!("Unexpected message: got 'transfer-ack' but expected {} more 'file-start' messages", n_accepted - i).into_boxed_str()))
            },
            other => {
                bail!(TransferError::unexpected_message("file-start", other))
            },
        };
        ensure!(
            file_start.file == file,
            TransferError::Protocol(
                format!(
                    "Unexpected file: got file {} but expected {}",
                    file_start.file.join("/"),
                    file.join("/"),
                )
                .into_boxed_str()
            )
        );

        let mut content;
        let mut received_size = 0;
        if file_start.start_at_offset {
            content = (answer.content)(true).await?;
            let offset = answer.offset;
            received_size = offset;
        } else {
            content = (answer.content)(false).await?;
        }

        progress_handler(total_received, total_size);
        loop {
            let payload =
                match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?.check_err()? {
                    PeerMessageV2::Payload(payload) => payload.payload,
                    PeerMessageV2::FileEnd(_) => {
                        bail!(TransferError::Protocol(
                            format!(
                            "Unexpected message: got 'file-end' but expected {} more payload bytes",
                            size - received_size,
                        )
                            .into_boxed_str()
                        ))
                    },
                    other => {
                        bail!(TransferError::unexpected_message("payload", other))
                    },
                };

            content.write_all(&payload).await?;
            received_size += payload.len() as u64;
            total_received += payload.len() as u64;
            progress_handler(total_received, total_size);

            if received_size == size {
                break;
            } else if received_size >= size {
                /* `received_size` must never become greater than `size` or we might panic on an integer underflow in the next iteration
                 * (only on an unhappy path, but still). Also, the progress bar might not appreciate.
                 */
                bail!(TransferError::Protocol(
                    format!(
                        "File too large: expected only {size} bytes, got at least {} more",
                        size - received_size
                    )
                    .into_boxed_str()
                ))
            }
        }

        content.close().await?;

        let _end = match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?.check_err()? {
            PeerMessageV2::FileEnd(end) => end,
            other => {
                bail!(TransferError::unexpected_message("file-end", other))
            },
        };
    }

    let _transfer_ack =
        match PeerMessageV2::de_msgpack(&transit.receive_record().await?)?.check_err()? {
            PeerMessageV2::TransferAck(transfer_ack) => transfer_ack,
            PeerMessageV2::FileStart(_) => {
                bail!(TransferError::Protocol(
                    "Unexpected message: got 'file-start' but did not expect any more files"
                        .to_string()
                        .into_boxed_str()
                ))
            },
            other => {
                bail!(TransferError::unexpected_message("transfer-ack", other))
            },
        };

    Ok(())
}
