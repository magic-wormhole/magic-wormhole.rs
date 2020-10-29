use magic_wormhole::core::PeerMessage;
use magic_wormhole::MessageType;
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
    
    let sender_thread = std::thread::Builder::new()
        .name("sender".to_owned())
        .spawn(move || {
            async_std::task::block_on(send(code_tx, sender_result_tx));
        })
        .unwrap();
    let receiver_thread = std::thread::Builder::new()
        .name("receiver".to_owned())
        .spawn(move || {
            async_std::task::block_on(receive(code_rx, receiver_result_tx));
        })
        .unwrap();
    
    sender_thread.join().unwrap();
    receiver_thread.join().unwrap();
    
    let original = fs::read("examples/example-file.bin").unwrap();
    let received = fs::read("example-file.bin").unwrap();
    
    if original == received {
        println!("Success!");
    } else {
        println!("Files differ...");
        std::process::exit(1);
    }
    
}

async fn receive(code_rx: mpsc::Receiver<String>, receiver_result_tx: mpsc::Sender<String>) {
    use magic_wormhole::{CodeProvider, filetransfer};
    use futures::{Stream, StreamExt, Sink, SinkExt};

    let code = code_rx.recv().unwrap();
    info!("Got code over local: {}", &code);
    let (welcome, connector) = magic_wormhole::connect_1(APPID, MAILBOX_SERVER, CodeProvider::SetCode(code)).await;
    info!("Got welcome: {}", &welcome.welcome);

    let mut w = connector.connect_2().await;
    info!("Got key: {:x?}", &w.key);
    let msg = w.rx.next().await.unwrap();
    let actual_message = PeerMessage::deserialize(std::str::from_utf8(&msg).unwrap());
    match actual_message {
        PeerMessage::Transit(transit) => {
            filetransfer::receive_file(
                    &mut w,
                    transit,
                    &RELAY_SERVER.parse().unwrap(),
                ).await.unwrap();
        },
        _ => todo!()
    };
}

async fn send(code_tx: mpsc::Sender<String>, _sender_result_tx: mpsc::Sender<String>) {
    use magic_wormhole::{CodeProvider, filetransfer};

    let (welcome, connector) = magic_wormhole::connect_1(APPID, MAILBOX_SERVER, CodeProvider::AllocateCode(2)).await;
    info!("Got welcome: {}", &welcome.welcome);
    info!("This wormhole's code is: {}", &welcome.code);
    code_tx.send(welcome.code.0).unwrap();
    let mut w = connector.connect_2().await;
    info!("Got key: {:x?}", &w.key);
    filetransfer::send_file(
        &mut w,
        "examples/example-file.bin",
        &RELAY_SERVER.parse().unwrap(),
    ).await.unwrap();
}
