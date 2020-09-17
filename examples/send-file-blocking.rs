use magic_wormhole::io::blocking::MessageType;
use magic_wormhole::io::blocking;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    let mailbox_server = String::from(MAILBOX_SERVER);
    let app_id = String::from(APPID);

    let mut w = blocking::connect(app_id.clone(), mailbox_server);
    let code = blocking::get_code(&mut w);
    println!("got the code: {}", code);

    // parse the relay url
    let relay_url = blocking::parse_relay_url(RELAY_SERVER);

    // send a file
    let msg = MessageType::File{ filename: "foobar".to_string(), filesize: 40960 };
    blocking::send(&mut w, app_id, code, msg, &relay_url);
}
