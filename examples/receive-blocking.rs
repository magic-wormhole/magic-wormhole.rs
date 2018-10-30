extern crate hex;
extern crate magic_wormhole_core;
extern crate magic_wormhole_io_blocking;

use magic_wormhole_core::{file_ack, message_ack, OfferType, PeerMessage};
use magic_wormhole_io_blocking::Wormhole;
use std::str;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &'static str = "ws://127.0.0.1:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
    println!("connecting..");
    w.set_code("4-purple-sausages");
    let verifier = w.get_verifier();
    println!("verifier: {}", hex::encode(verifier));
    println!("receiving..");
    let msg = w.get_message();
    let actual_message =
        PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
    match actual_message {
        PeerMessage::Offer(offer) => match offer {
            OfferType::Message(msg) => {
                println!("{}", msg);
                w.send_message(message_ack("ok").serialize().as_bytes());
            }
            OfferType::File { .. } => {
                println!("Received file offer {:?}", offer);
                w.send_message(file_ack("ok").serialize().as_bytes());
            }
            OfferType::Directory { .. } => {
                println!("Received directory offer: {:?}", offer);
                // TODO: We are doing file_ack without asking user
                w.send_message(file_ack("ok").serialize().as_bytes());
            }
        },
        PeerMessage::Answer(_) => {
            panic!("Should not receive answer type, I'm receiver")
        }
        PeerMessage::Error(err) => println!("Something went wrong: {}", err),
        PeerMessage::Transit(transit) => {
            // TODO: This should start transit server connection or direct file transfer
            println!("Transit Message received: {:?}", transit)
        }
    };
    println!("closing..");
    w.close();
    println!("closed");
}
