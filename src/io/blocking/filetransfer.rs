use async_std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use crate::core::{
    TransitType,
    PeerMessage,
    AnswerType,
    TransitAck,
    OfferType,
};
use async_std::io::BufReader;
use async_std::io::Write;
use async_std::io::prelude::WriteExt;
use async_std::io::Read;
use async_std::io::ReadExt;
use log::*;
use sha2::{Digest, Sha256};
use sodiumoxide::crypto::secretbox;
use async_std::fs::File;
use anyhow::{Result, ensure, bail, format_err, Context};
use super::Wormhole;
use super::transit;
use super::transit::{RelayUrl, Transit};

pub async fn send_file(
    w: &mut Wormhole,
    filepath: impl AsRef<Path>,
    appid: &str,
    relay_url: &RelayUrl,
) -> Result<()> {
    let filename = filepath.as_ref().file_name().ok_or_else(|| format_err!("You can't send a file without a file name"))?;
    let filesize = File::open(filepath.as_ref()).await?.metadata().await?.len(); // TODO do that somewhere else
    let (mut transit, ()) = Transit::sender_connect(
        w,
        relay_url,
        appid,
        |w| send_file_offer(w, filename, filesize),
    ).await?;
    debug!("Beginning file transfer");

    tcp_file_send(&mut transit, &filepath).await
        .context("Could not send file")
}

pub async fn receive_file(
    w: &mut Wormhole,
    ttype: TransitType,
    appid: &str,
    relay_url: &RelayUrl,
) -> Result<()> {
    let (mut transit, (filename, filesize)) = Transit::receiver_connect(
        w,
        relay_url,
        appid,
        ttype,
        receive_file_offer,
    ).await?;

    debug!("Beginning file transfer");
    // TODO here's the right position for applying the output directory and to check for malicious (relative) file paths
    tcp_file_receive(&mut transit, &filename, filesize).await
       .context("Could not receive file")
}

async fn send_file_offer(w: &mut Wormhole, filename: (impl Into<PathBuf> + AsRef<Path>), filesize: u64) -> Result<()> {
    // 6. send file offer message.
    let offer_msg = PeerMessage::new_offer_file(filename, filesize).serialize();
    w.send_message(offer_msg.as_bytes());

    // 7. wait for file_ack
    let maybe_fileack = w.get_message();
    let fileack_msg = PeerMessage::deserialize(std::str::from_utf8(&maybe_fileack)?);
    debug!("received file ack message: {:?}", fileack_msg);

    match fileack_msg {
        PeerMessage::Answer(AnswerType::FileAck(msg)) => {
            ensure!(msg == "ok", "file ack failed");
        },
        _ => bail!("did not receive file ack")
    }

    Ok(())
}

async fn receive_file_offer(w: &mut Wormhole) -> Result<(PathBuf, u64)>  {
    // 3. receive file offer message from peer
    let msg = w.get_message();
    let maybe_offer = PeerMessage::deserialize(std::str::from_utf8(&msg)?);
    debug!("Received offer message '{:?}'", &maybe_offer);

    let (filename, filesize) = match maybe_offer {
        PeerMessage::Offer(offer_type) => {
            match offer_type {
                OfferType::File{filename, filesize} => (filename, filesize),
                _ => bail!("unexpected offer type"),
            }
        },
        _ => bail!("unexpected message: {:?}", maybe_offer),
    };

    // send file ack.
    let file_ack_msg = PeerMessage::new_file_ack("ok").serialize();
    w.send_message(file_ack_msg.as_bytes());
    
    Ok((filename, filesize))
}

// encrypt and send the file to tcp stream and return the sha256 sum
// of the file before encryption.
async fn send_records(filepath: impl AsRef<Path>, stream: &mut TcpStream, skey: &[u8]) -> Result<Vec<u8>> {
    // rough plan:
    // 1. Open the file
    // 2. read a block of N bytes
    // 3. calculate a rolling sha256sum.
    // 4. AEAD with skey and with nonce as a counter from 0.
    // 5. send the encrypted buffer to the socket.
    // 6. go to step #2 till eof.
    // 7. if eof, return sha256 sum.

    let mut file = File::open(&filepath.as_ref()).await
        .context(format!("Could not open {}", &filepath.as_ref().display()))?;
    debug!("Sending file size {}", file.metadata().await?.len());

    let mut hasher = Sha256::default();

    let nonce_slice: [u8; sodiumoxide::crypto::secretbox::NONCEBYTES]
        = [0; sodiumoxide::crypto::secretbox::NONCEBYTES];
    let mut nonce = secretbox::Nonce::from_slice(&nonce_slice[..])
        .ok_or(format_err!("Could not parse nonce".to_string()))?;

    loop {
        // read a block of 4096 bytes
        let mut plaintext = [0u8; 4096];
        let n = file.read(&mut plaintext[..]).await?;
        debug!("sending {} bytes", n);

        let ciphertext = transit::encrypt_record(&plaintext[0..n], nonce, &skey)?;

        // send the encrypted record
        transit::send_record(stream, &ciphertext).await?;

        // increment nonce
        nonce.increment_le_inplace();

        // sha256 of the input
        hasher.input(&plaintext[..n]);

        if n < 4096 {
            break;
        }
        else {
            continue;
        }
    }
    Ok(hasher.result().to_vec())
}

async fn receive_records(filepath: impl AsRef<Path>, filesize: u64, tcp_conn: &mut TcpStream, skey: &[u8]) -> Result<Vec<u8>> {
    let mut stream = BufReader::new(tcp_conn);
    let mut hasher = Sha256::default();
    let mut f = File::create(filepath.as_ref()).await?; // TODO overwrite flags & checks & stuff
    let mut remaining_size = filesize as usize;

    while remaining_size > 0 {
        debug!("remaining size: {:?}", remaining_size);

        let enc_packet = transit::receive_record(&mut stream).await?;

        // enc_packet.truncate(enc_packet_length);
        debug!("length of the ciphertext: {:?}", enc_packet.len());

        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = transit::decrypt_record(&enc_packet, &skey)?;

        debug!("decryption succeeded");
        f.write_all(&plaintext).await?;

        // 4. calculate a rolling sha256 sum of the decrypted output.
        hasher.input(&plaintext);

        remaining_size -= plaintext.len();
    }

    debug!("done");
    // TODO: 5. write the buffer into a file.
    Ok(hasher.result().to_vec())
}

async fn tcp_file_send(transit: &mut Transit, filepath: impl AsRef<Path>) -> Result<()> {
    // 11. send the file as encrypted records.
    let checksum = send_records(filepath, &mut transit.socket, &transit.skey).await?;

    // 13. wait for the transit ack with sha256 sum from the peer.
    debug!("sent file. Waiting for ack");
    let enc_transit_ack = transit::receive_record(&mut BufReader::new(&mut transit.socket)).await?;
    let transit_ack = transit::decrypt_record(&enc_transit_ack, &transit.rkey)?;
    let transit_ack_msg = TransitAck::deserialize(std::str::from_utf8(&transit_ack)?);
    ensure!(transit_ack_msg.sha256 == hex::encode(checksum), "receive checksum error");
    
    debug!("transfer complete!");
    Ok(())
}

async fn tcp_file_receive(transit: &mut Transit, filepath: impl AsRef<Path>, filesize: u64) -> Result<()> {
    // 5. receive encrypted records
    // now skey and rkey can be used. skey is used by the tx side, rkey is used
    // by the rx side for symmetric encryption.
    let checksum = receive_records(filepath, filesize, &mut transit.socket, &transit.skey).await?;

    let sha256sum = hex::encode(checksum.as_slice());
    debug!("sha256 sum: {:?}", sha256sum);

    // 6. verify sha256 sum by sending an ack message to peer along with checksum.
    let ack_msg = transit::make_transit_ack_msg(&sha256sum, &transit.rkey)?;
    transit::send_record(&mut transit.socket, &ack_msg).await?;

    // 7. close socket.
    // well, no need, it gets dropped when it goes out of scope.
    debug!("Transfer complete");
    Ok(())
}
