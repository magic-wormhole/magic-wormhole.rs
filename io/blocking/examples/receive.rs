extern crate magic_wormhole_io_ws;
extern crate hex;
use magic_wormhole_io_ws::Wormhole;

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
    use std::str;
    println!(
        "message received: {}",
        str::from_utf8(&msg).unwrap()
    );
    println!("closing..");
    w.close();
    println!("closed");
}
