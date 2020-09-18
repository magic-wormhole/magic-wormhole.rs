use magic_wormhole::core::{file_ack, message_ack, OfferType, PeerMessage};
use magic_wormhole::io::blocking::Wormhole;
use std::str;
use log::*;

const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Trace)
        .filter_module("mio", LevelFilter::Debug)
        .filter_module("ws", LevelFilter::Info)
        .init();
    let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
    trace!("connecting..");
    w.set_code("8-cumbersome-guidance");// Hard-code this in every time you test with a new value
    let verifier = w.get_verifier();
    trace!("verifier: {}", hex::encode(verifier));
    trace!("receiving..");
    let msg = w.get_message();
    let actual_message =
        PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
    match actual_message {
        PeerMessage::Offer(offer) => match offer {
            OfferType::Message(msg) => {
                trace!("{}", msg);
                w.send_message(message_ack("ok").serialize().as_bytes());
            }
            OfferType::File { .. } => {
                trace!("Received file offer {:?}", offer);
                w.send_message(file_ack("ok").serialize().as_bytes());
            }
            OfferType::Directory { .. } => {
                trace!("Received directory offer: {:?}", offer);
                // TODO: We are doing file_ack without asking user
                w.send_message(file_ack("ok").serialize().as_bytes());
            }
        },
        PeerMessage::Answer(_) => {
            panic!("Should not receive answer type, I'm receiver")
        }
        PeerMessage::Error(err) => trace!("Something went wrong: {}", err),
        PeerMessage::Transit(transit) => {
            // TODO: This should start transit server connection or direct file transfer
            trace!("Transit Message received: {:?}", transit)
        }
    };
    trace!("closing..");
    w.close();
    trace!("closed");
}
