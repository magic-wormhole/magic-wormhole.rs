use magic_wormhole::io::blocking::MessageType;
use magic_wormhole::io::blocking::Wormhole;
use log::*;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .filter_module("mio", LevelFilter::Debug)
        .filter_module("ws", LevelFilter::Info)
        .init();
    let mailbox_server = String::from(MAILBOX_SERVER);

    let mut w = Wormhole::new(&APPID, &mailbox_server);
    w.allocate_code(2);
    let code = async_std::task::block_on(w.get_code());
    info!("got the code: {}", code);

    // send a file
    let msg = MessageType::File{ filename: "examples/example-file.bin".to_string(), filesize: 40960 };
    info!("sending..");
    async_std::task::block_on(w.send(APPID, &code, msg, &RELAY_SERVER.parse().unwrap())).unwrap();
}