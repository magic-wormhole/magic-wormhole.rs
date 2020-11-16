#![cfg_attr(tarpaulin, skip)]

use super::events::Phase;
use super::Mood;
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

/** Start a connection from both sides, and then close one and check that the event loop stops. */
#[async_std::test]
pub async fn test_eventloop_exit1() {
    init_logger();

    use futures::SinkExt;
    use futures::StreamExt;
    use std::time::Duration;

    let code = "42-something-something"; // TODO dynamic code allocation
    let dummy_task = async_std::task::spawn(async move {
        let (_welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::SetCode(code.into()),
            &mut None,
        )
        .await
        .unwrap();
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
            CodeProvider::SetCode(code.into()),
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

    let code = "41-something-something"; // TODO dynamic code allocation
    let dummy_task = async_std::task::spawn(async move {
        let (_welcome, connector) = crate::connect_to_server(
            APPID,
            serde_json::json!({}),
            MAILBOX_SERVER,
            CodeProvider::SetCode(code.into()),
            &mut None,
        )
        .await
        .unwrap();
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
            CodeProvider::SetCode(code.into()),
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
