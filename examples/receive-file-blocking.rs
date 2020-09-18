use magic_wormhole::io::blocking::Wormhole;
use log::*;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    env_logger::builder().filter_level(LevelFilter::Trace).init();
    let mailbox_server = String::from(MAILBOX_SERVER);

    info!("connecting..");
    let mut w = Wormhole::new(&APPID, &mailbox_server);
    // Hard-code this in every time you test with a new value
    let code = "12-dakota-cement";
    w.set_code(code);
    debug!("using the code: {}", code);
    let verifier = w.get_verifier();
    debug!("verifier: {}", hex::encode(verifier));
    info!("receiving..");

    Wormhole::receive(MAILBOX_SERVER, APPID, code, &RELAY_SERVER.parse().unwrap()).unwrap();
}
