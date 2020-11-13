#![cfg_attr(tarpaulin, skip)]

use crate::CodeProvider;
use super::api::Mood;
use super::events::{Event, Events, Phase};

const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const APPID: &str = "lothar.com/wormhole/rusty-wormhole-test";

#[async_std::test]
pub async fn test_eventloop_exit() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("mio", log::LevelFilter::Debug)
        .filter_module("ws", log::LevelFilter::Error)
        .init();

    use futures::StreamExt;
    use futures::SinkExt;
    use std::time::Duration;

    let code = "42-something-something"; // TODO dynamic code allocation
    let dummy_task = async_std::task::spawn(async move {
        let (_welcome, connector) = crate::connect_to_server(APPID, serde_json::json!({}), MAILBOX_SERVER, CodeProvider::SetCode(code.into()), &mut None).await;
        let _wormhole = connector.connect_to_client().await;
        log::info!("A Connected.");
        async_std::task::sleep(Duration::from_secs(5)).await;
        log::info!("A Done sleeping.");
    });
    async_std::future::timeout(Duration::from_secs(5), async {
        let (_welcome, connector) = crate::connect_to_server(APPID, serde_json::json!({}), MAILBOX_SERVER, CodeProvider::SetCode(code.into()), &mut None).await;
        let mut wormhole = connector.connect_to_client().await;
        log::info!("B Connected.");
        wormhole.tx.close().await.unwrap();
        log::info!("B Closed sender");
        wormhole.rx.for_each(|e| async move {log::info!("Received {:?}", e);}).await;
    })
    .await
    .expect("Test failed");

    dummy_task.cancel().await;
}

#[async_std::test]
pub async fn test_eventloop_exit2() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("mio", log::LevelFilter::Debug)
        .filter_module("ws", log::LevelFilter::Error)
        .init();

    use futures::StreamExt;
    use futures::SinkExt;
    use std::time::Duration;

    let code = "42-something-something"; // TODO dynamic code allocation
    let dummy_task = async_std::task::spawn(async move {
        let (_welcome, connector) = crate::connect_to_server(APPID, serde_json::json!({}), MAILBOX_SERVER, CodeProvider::SetCode(code.into()), &mut None).await;
        let _wormhole = connector.connect_to_client().await;
        log::info!("A Connected.");
        async_std::task::sleep(Duration::from_secs(5)).await;
        log::info!("A Done sleeping.");
    });
    async_std::future::timeout(Duration::from_secs(5), async {
        let (_welcome, connector) = crate::connect_to_server(APPID, serde_json::json!({}), MAILBOX_SERVER, CodeProvider::SetCode(code.into()), &mut None).await;
        let wormhole = connector.connect_to_client().await;
        log::info!("B Connected.");
        std::mem::drop(wormhole.tx);
        wormhole.rx.for_each(|e| async move {log::info!("Received {:?}", e);}).await;
        log::info!("B Closed.");
    })
    .await
    .expect("Test failed");

    dummy_task.cancel().await;
}

#[async_std::test]
pub async fn test_eventloop_exit3() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("mio", log::LevelFilter::Debug)
        .filter_module("ws", log::LevelFilter::Error)
        .init();

    use std::time::Duration;

    async_std::future::timeout(Duration::from_secs(5), async {
        let mut eventloop_task = None;
        let (_welcome, connector) = crate::connect_to_server(APPID, serde_json::json!({}), MAILBOX_SERVER, CodeProvider::AllocateCode(2), &mut eventloop_task).await;
        let eventloop_task = eventloop_task.unwrap();

        log::info!("Connected.");
        std::mem::drop(connector);
        eventloop_task.await;
        log::info!("Closed.");
    })
    .await
    .expect("Test failed");
}

pub fn filt(ev: Events) -> Events {
    ev.into_iter()
        .filter(|e| match e {
            Event::Timing(_) => false,
            _ => true,
        })
        .collect()
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

// use super::{IOAction, IOEvent, TimerHandle, WSHandle, WormholeCore};
// use crate::core::server_messages::{deserialize_outbound, OutboundMessage};

#[test]
fn create() {
    // TODO fix this test and make it work again

    // env_logger::try_init().unwrap();
    // let url: &str = "url";
    // let mut w = WormholeCore::new("appid", url);

    // let wsh = WSHandle::new(1);
    // let th = TimerHandle::new(1);
    // let mut _got_side: &str;

    // let ios = w.start();
    // assert_eq!(ios.len(), 1);
    // assert_eq!(
    //     ios,
    //     vec![Action::IO(IOAction::WebSocketOpen(wsh, url.to_string()))]
    // );

    // let actions = w.do_io(IOEvent::WebSocketConnectionMade(wsh));
    // assert_eq!(actions.len(), 1);
    // match &actions[0] {
    //     Action::IO(IOAction::WebSocketSendMessage(handle, m)) => {
    //         assert_eq!(handle, &wsh);
    //         if let OutboundMessage::Bind { appid, side } =
    //             deserialize_outbound(&m)
    //         {
    //             assert_eq!(&appid.0, "appid");
    //             _got_side = &side.0; // random
    //         } else {
    //             panic!();
    //         }
    //     }
    //     _ => panic!(),
    // }

    // let actions = w.do_io(IOEvent::WebSocketConnectionLost(wsh));
    // assert_eq!(actions.len(), 1);
    // match &actions[0] {
    //     Action::IO(IOAction::StartTimer(handle, delay)) => {
    //         assert_eq!(handle, &th);
    //         assert_eq!(delay, &5.0);
    //     }
    //     _ => panic!(),
    // }

    // let actions = w.do_io(IOEvent::TimerExpired(th));
    // assert_eq!(actions.len(), 1);
    // assert_eq!(
    //     actions,
    //     vec![Action::IO(IOAction::WebSocketOpen(
    //         WSHandle::new(2),
    //         url.to_string()
    //     ))]
    // );
}
