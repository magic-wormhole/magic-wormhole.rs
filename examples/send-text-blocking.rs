use magic_wormhole::core::message;
use magic_wormhole::io::blocking::Wormhole;
use log::*;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
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
    // w.set_code("4-purple-sausages");
    w.allocate_code(2);
    let code = w.get_code();
    trace!("code is: {}", code);
    trace!("sending..");
    w.send_message(message("hello from rust!").serialize().as_bytes());
    trace!("sent..");
    // if we close right away, we won't actually send anything. Wait for at
    // least the verifier to be printed, that ought to give our outbound
    // message a chance to be delivered.
    let verifier = w.get_verifier();
    trace!("verifier: {}", hex::encode(verifier));
    trace!("got verifier, closing..");
    w.close();
    trace!("closed");
}
