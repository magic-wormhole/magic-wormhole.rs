extern crate hex;
extern crate magic_wormhole_core;
extern crate magic_wormhole_io_blocking;

use magic_wormhole_core::message;
use magic_wormhole_io_blocking::Wormhole;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &'static str = "ws://127.0.0.1:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
    println!("connecting..");
    // w.set_code("4-purple-sausages");
    w.allocate_code(2);
    let code = w.get_code();
    println!("code is: {}", code);
    println!("sending..");
    w.send_message(message("hello from rust!").serialize().as_bytes());
    println!("sent..");
    // if we close right away, we won't actually send anything. Wait for at
    // least the verifier to be printed, that ought to give our outbound
    // message a chance to be delivered.
    let verifier = w.get_verifier();
    println!("verifier: {}", hex::encode(verifier));
    println!("got verifier, closing..");
    w.close();
    println!("closed");
}
