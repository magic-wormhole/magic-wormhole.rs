use magic_wormhole::io::blocking::MessageType;
use magic_wormhole::io::blocking::Wormhole;
use log::*;
use std::fs;
use std::sync::mpsc;

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
    
    let (code_tx, code_rx) = std::sync::mpsc::channel();
    let (sender_result_tx, _sender_result_rx) = std::sync::mpsc::channel();
    let (receiver_result_tx, _receiver_result_rx) = std::sync::mpsc::channel();
    
    let sender_thread = std::thread::spawn(move || {
        send(code_tx, sender_result_tx);
    });
    let receiver_thread = std::thread::spawn(move || {
        receive(code_rx, receiver_result_tx);
    });
    
    sender_thread.join().unwrap();
    receiver_thread.join().unwrap();
    
    let original = fs::read("example-file.bin").unwrap();
    let received = fs::read("example-file.bin.rcv").unwrap();
    
    if original == received {
        println!("Success!");
    } else {
        println!("Files differ...");
        std::process::exit(1);
    }
    
}

fn receive(code_rx: mpsc::Receiver<String>, receiver_result_tx: mpsc::Sender<String>) {
    let mailbox_server = String::from(MAILBOX_SERVER);

    info!("connecting..");
    let mut w = Wormhole::new(&APPID, &mailbox_server);
    // Hard-code this in every time you test with a new value
    //let code = "TODO-insert-code-here";
    let code = code_rx.recv().unwrap();
    w.set_code(&code[..]);
    debug!("using the code: {}", code);
    let verifier = w.get_verifier();
    debug!("verifier: {}", hex::encode(verifier));
    info!("receiving..");

    w.receive(APPID, &RELAY_SERVER.parse().unwrap()).unwrap();
    receiver_result_tx.send(String::from("")).unwrap();
}

fn send(code_tx: mpsc::Sender<String>, sender_result_tx: mpsc::Sender<String>) {
    let mailbox_server = String::from(MAILBOX_SERVER);

    let mut w = Wormhole::new(&APPID, &mailbox_server);
    w.allocate_code(2);
    let code = w.get_code();
    info!("got the code: {}", code);
    code_tx.send(code.clone()).unwrap();

    // send a file
    let msg = MessageType::File{ filename: "examples/example-file.bin".to_string(), filesize: 40960 };
    info!("sending..");
    w.send(APPID, &code, msg, &RELAY_SERVER.parse().unwrap()).unwrap();
    sender_result_tx.send(String::from("")).unwrap();
}
