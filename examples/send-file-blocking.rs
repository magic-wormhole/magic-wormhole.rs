use magic_wormhole::io::blocking::MessageType;
use magic_wormhole::io::blocking::Wormhole;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    env_logger::try_init().unwrap();
    let mailbox_server = String::from(MAILBOX_SERVER);
    let app_id = String::from(APPID);

    let mut w = Wormhole::new(&app_id, &mailbox_server);
    w.allocate_code(2);
    let code = w.get_code();
    println!("got the code: {}", code);

    // send a file
    let msg = MessageType::File{ filename: "foobar".to_string(), filesize: 40960 };
    w.send(app_id, code, msg, &RELAY_SERVER.parse().unwrap());
}
