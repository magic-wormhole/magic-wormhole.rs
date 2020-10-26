use magic_wormhole::core::PeerMessage;
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
    use magic_wormhole::io::blocking::{Wormhole2, CodeProvider, filetransfer};
    use futures::{Stream, StreamExt, Sink, SinkExt};

    let code = code_rx.recv().unwrap();
    info!("Got code over local: {}", &code);
    let (welcome, connector) = Wormhole2::new(APPID, MAILBOX_SERVER, CodeProvider::SetCode(code)).await;
    info!("Got welcome: {}", &welcome.welcome);

    let (mut tx, mut rx, key) = connector.do_pake().await;
    info!("Got key: {:x?}", key);
    let msg = rx.next().await.unwrap();
    let actual_message = PeerMessage::deserialize(std::str::from_utf8(&msg).unwrap());
    match actual_message {
        PeerMessage::Transit(transit) => {
            filetransfer::receive_file(
                    &key,
                    &mut tx,
                    &mut rx,
                    transit,
                    APPID,
                    &RELAY_SERVER.parse().unwrap(),
                ).await.unwrap();
        },
        _ => todo!()
    };


    // let mailbox_server = String::from(MAILBOX_SERVER);

    // info!("connecting..");
    // let mut w = Wormhole::new(&APPID, &mailbox_server);
    // // Hard-code this in every time you test with a new value
    // //let code = "TODO-insert-code-here";
    // let code = code_rx.recv().unwrap();
    // w.set_code(&code[..]);
    // debug!("using the code: {}", code);
    // let verifier = w.get_verifier().await;
    // debug!("verifier: {}", hex::encode(verifier));
    // info!("receiving..");

    // w.receive(APPID, &RELAY_SERVER.parse().unwrap()).await.unwrap();
    // receiver_result_tx.send(String::from("")).unwrap();
}

async fn send(code_tx: mpsc::Sender<String>, _sender_result_tx: mpsc::Sender<String>) {
    use magic_wormhole::io::blocking::{Wormhole2, CodeProvider, filetransfer};

    let (welcome, connector) = Wormhole2::new(APPID, MAILBOX_SERVER, CodeProvider::AllocateCode(2)).await;
    info!("Got welcome: {}", &welcome.welcome);
    info!("This wormhole's code is: {}", &welcome.code);
    code_tx.send(welcome.code.0).unwrap();
    let (mut tx, mut rx, key) = connector.do_pake().await;
    info!("Got key: {:x?}", key);
    filetransfer::send_file(
        &key,
        &mut tx,
        &mut rx,
        "examples/example-file.bin",
        APPID,
        &RELAY_SERVER.parse().unwrap(),
    ).await.unwrap();

    // let mailbox_server = String::from(MAILBOX_SERVER);

    // let mut w = Wormhole::new(&APPID, &mailbox_server);
    // w.allocate_code(2);
    // let code = w.get_code().await;
    // info!("got the code: {}", code);
    // code_tx.send(code.clone()).unwrap();

    // // send a file
    // let msg = MessageType::File{ filename: "examples/example-file.bin".to_string(), filesize: 40960 };
    // info!("sending..");
    // w.send(APPID, &code, msg, &RELAY_SERVER.parse().unwrap()).await.unwrap();
    // sender_result_tx.send(String::from("")).unwrap();
}
