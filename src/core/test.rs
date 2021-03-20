#![cfg_attr(tarpaulin, skip)]

use super::{events::Phase, Mood};
use crate::CodeProvider;

const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const APPID: &str = "lothar.com/wormhole/rusty-wormhole-test";

fn init_logger() {
    /* Ignore errors from succeedent initialization tries */
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("magic_wormhole", log::LevelFilter::Trace)
        .filter_module("magic_wormhole::transfer", log::LevelFilter::Trace)
        .filter_module("magic_wormhole::transit", log::LevelFilter::Trace)
        .filter_module("mio", log::LevelFilter::Debug)
        .try_init();
}

/** Send a file using the Rust implementation. This does not guarantee compatibility with Python! ;) */
#[async_std::test]
pub async fn test_file_rust2rust() -> anyhow::Result<()> {
    init_logger();
    use crate as magic_wormhole;
    use magic_wormhole::{transfer, CodeProvider};

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let (welcome, connector) = magic_wormhole::connect_to_server(
                magic_wormhole::transfer::APPID,
                magic_wormhole::transfer::AppVersion::default(),
                magic_wormhole::DEFAULT_MAILBOX_SERVER,
                CodeProvider::AllocateCode(2),
                &mut None,
            )
            .await?;
            log::info!("Got welcome: {}", &welcome.welcome);
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code.0).unwrap();
            let mut w = connector.connect_to_client().await?;
            log::info!("Got key: {}", &w.key);
            transfer::send_file(
                &mut w,
                "examples/example-file.bin",
                &magic_wormhole::transit::DEFAULT_RELAY_SERVER
                    .parse()
                    .unwrap(),
            )
            .await
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let code = code_rx.await?;
            log::info!("Got code over local: {}", &code);
            let (welcome, connector) = magic_wormhole::connect_to_server(
                magic_wormhole::transfer::APPID,
                magic_wormhole::transfer::AppVersion::default(),
                magic_wormhole::DEFAULT_MAILBOX_SERVER,
                CodeProvider::SetCode(code),
                &mut None,
            )
            .await?;
            log::info!("Got welcome: {}", &welcome.welcome);

            let mut w = connector.connect_to_client().await?;
            log::info!("Got key: {}", &w.key);
            let req = transfer::request_file(
                &mut w,
                &magic_wormhole::transit::DEFAULT_RELAY_SERVER
                    .parse()
                    .unwrap(),
            )
            .await?;

            req.accept(false).await
        })?;
    sender_task.await?;
    receiver_task.await?;

    let original = std::fs::read("examples/example-file.bin")?;
    let received = std::fs::read("example-file.bin")?;
    assert_eq!(original, received, "Files differ");
    Ok(())
}

/** Start a connection from both sides, and then close one and check that the event loop stops. */
#[async_std::test]
pub async fn test_eventloop_exit1() {
    init_logger();

    use futures::{SinkExt, StreamExt};
    use std::time::Duration;

    let (code_tx, code_rx) = futures::channel::oneshot::channel();
    let dummy_task = async_std::task::spawn(async move {
        let (welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::default(),
            &mut None,
        )
        .await
        .unwrap();
        code_tx.send(welcome.code).unwrap();
        let _wormhole = connector.connect_to_client().await.unwrap();
        log::info!("A Connected.");
        async_std::task::sleep(Duration::from_secs(5)).await;
        log::info!("A Done sleeping.");
    });
    async_std::future::timeout(Duration::from_secs(5), async {
        let (_welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::SetCode(code_rx.await.unwrap().to_string()),
            &mut None,
        )
        .await
        .unwrap();
        let mut wormhole = connector.connect_to_client().await.unwrap();
        log::info!("B Connected.");
        wormhole.tx.close().await.unwrap();
        log::info!("B Closed sender");
        wormhole
            .rx
            .for_each(|e| async move {
                log::info!("Received {:?}", e);
            })
            .await;
    })
    .await
    .expect("Test failed");

    dummy_task.cancel().await;
}

/** Start a connection from both sides, and then drop one and check that the event loop stops. */
#[async_std::test]
pub async fn test_eventloop_exit2() {
    init_logger();

    use futures::StreamExt;
    use std::time::Duration;

    let (code_tx, code_rx) = futures::channel::oneshot::channel();
    let dummy_task = async_std::task::spawn(async move {
        let (welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::default(),
            &mut None,
        )
        .await
        .unwrap();
        code_tx.send(welcome.code).unwrap();
        let _wormhole = connector.connect_to_client().await;
        log::info!("A Connected.");
        async_std::task::sleep(Duration::from_secs(5)).await;
        log::info!("A Done sleeping.");
    });
    async_std::future::timeout(Duration::from_secs(5), async {
        let (_welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::SetCode(code_rx.await.unwrap().to_string()),
            &mut None,
        )
        .await
        .unwrap();
        let wormhole = connector.connect_to_client().await.unwrap();
        log::info!("B Connected.");
        std::mem::drop(wormhole.tx);
        wormhole
            .rx
            .for_each(|e| async move {
                log::info!("Received {:?}", e);
            })
            .await;
        log::info!("B Closed.");
    })
    .await
    .expect("Test failed");

    dummy_task.cancel().await;
}

/** Start a connection only to the server (no other client), drop the connector and assert that the event loop stops */
#[async_std::test]
pub async fn test_eventloop_exit3() {
    init_logger();

    use std::time::Duration;

    async_std::future::timeout(Duration::from_secs(5), async {
        let mut eventloop_task = None;
        let (_welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::AllocateCode(2),
            &mut eventloop_task,
        )
        .await
        .unwrap();
        let eventloop_task = eventloop_task.unwrap();

        log::info!("Connected.");
        connector.cancel().await;
        eventloop_task.await;
        log::info!("Closed.");
    })
    .await
    .expect("Test failed");
}

#[test]
fn test_phase() {
    let p = Phase(String::from("pake"));
    assert!(p.is_pake()); // Order looks for "pake"
}

#[test]
fn test_mood() {
    // The serialized forms of these variants are part of the wire protocol,
    // so they must be spelled exactly as shown (they must match the strings
    // used in the Python version in src/wormhole/_boss.py , in calls to
    // self._T.close())
    assert_eq!(
        String::from(r#""happy""#),
        serde_json::to_string(&Mood::Happy).unwrap()
    );
    assert_eq!(
        String::from(r#""lonely""#),
        serde_json::to_string(&Mood::Lonely).unwrap()
    );
    assert_eq!(
        String::from(r#""errory""#),
        serde_json::to_string(&Mood::Errory).unwrap()
    );
    assert_eq!(
        String::from(r#""scary""#),
        serde_json::to_string(&Mood::Scared).unwrap()
    );
    assert_eq!(
        String::from(r#""unwelcome""#),
        serde_json::to_string(&Mood::Unwelcome).unwrap()
    );
}
