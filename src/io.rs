use async_trait::async_trait;
use crate::core::{
    IOAction,
    IOEvent,
    WSHandle,
    TimerHandle,
};
use std::collections::{HashMap, HashSet};
use std::time;
use std::sync::mpsc::{channel, Receiver, Sender};
use log::*;
use anyhow::{Result, Error, ensure, bail, format_err, Context};

#[cfg(feature = "io-blocking")]
pub mod blocking;

#[cfg(feature = "io-tokio")]
pub mod tokio;

#[cfg(feature = "io-blocking")]
#[async_trait]
pub trait WormholeIO {
    fn process(&mut self, action: IOAction);
}

#[cfg(feature = "io-blocking")]
enum WSControl {
    Data(String),
    Close,
}

#[cfg(feature = "io-blocking")]
async fn ws_connector(
    url: &str,
    handle: WSHandle,
    tx: Sender<IOEvent>,
    ws_rx: futures::channel::mpsc::UnboundedReceiver<WSControl>,
) {
    use async_tungstenite::async_std::*;
    use futures::stream::StreamExt;
    // use futures::sink::SinkExt;
    use async_tungstenite::tungstenite as ws2;

    let (ws_stream, _) = connect_async(url).await.unwrap();
    tx.send(IOEvent::WebSocketConnectionMade(handle)).unwrap();
    let (mut write, mut read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    async_std::task::spawn(async move {
        while let Some(message) = read.next().await {
            match message.unwrap() {
                ws2::Message::Text(text) => {
                    tx.send(IOEvent::WebSocketMessageReceived(handle, text)).unwrap();
                },
                ws2::Message::Close(_) => {
                    tx.send(IOEvent::WebSocketConnectionLost(handle)).unwrap();
                },
                ws2::Message::Ping(_) => {
                    warn!("Not responding to pings for now");
                    // TODO
                },
                ws2::Message::Pong(_) => {
                    warn!("Got a pong without ping?!");
                    // TODO maybe send pings too?
                },
                ws2::Message::Binary(_) => {
                    error!("Someone is sending binary data, this is not part of the protocol!");
                },
            }
        }
        // read.for_each(|message| async {
        //     match message.unwrap() {
        //         ws2::Message::Text(text) => {
        //             println!("B");
        //             tx.send(ToCore::WebSocketMessageReceived(handle, text)).unwrap();
        //         },
        //         ws2::Message::Close(_) => {
        //             println!("C");
        //             tx.send(ToCore::WebSocketConnectionLost(handle)).unwrap();
        //         },
        //         _ => panic!()
        //     }
        // }).await;
    });
    /* Send events from the API to the other websocket side */
    async_std::task::spawn(async move {
        ws_rx
        .map(|c| {
            match c {
                WSControl::Data(d) => {
                    ws2::Message::Text(d)
                },
                WSControl::Close => ws2::Message::Close(None),
            }
        })
        .map(Ok)
        .forward(write)
        .await
        .unwrap();
    });
}

#[cfg(feature = "io-blocking")]
pub struct AsyncStdIO {
    tx_to_core: Sender<IOEvent>,
    timers: HashSet<TimerHandle>,
    websockets: HashMap<WSHandle, futures::channel::mpsc::UnboundedSender<WSControl>>,
}

impl AsyncStdIO {
    pub fn new(tx_to_core: Sender<IOEvent>) -> Self {
        AsyncStdIO {
            tx_to_core,
            timers: HashSet::new(),
            websockets: HashMap::new(),
        }
    }
}

#[cfg(feature = "io-blocking")]
#[async_trait]
impl WormholeIO for AsyncStdIO {
    fn process(&mut self, action: IOAction) {
        use futures::SinkExt;
        use self::IOAction::*;
        match action {
            StartTimer(handle, duration) => {
                let tx = self.tx_to_core.clone();
                self.timers.insert(handle);
                std::thread::spawn(move || {
                    // ugh, why can't this just take a float? ok ok,
                    // Nan, negatives, fine fine
                    let dur_ms = (duration * 1000.0) as u64;
                    let dur = time::Duration::from_millis(dur_ms);
                    std::thread::sleep(dur);
                    tx.send(IOEvent::TimerExpired(handle)).unwrap();
                });
            },
            CancelTimer(handle) => {
                self.timers.remove(&handle);
            },
            WebSocketOpen(handle, url) => {
                let tx = self.tx_to_core.clone();
                let (ws_tx, ws_rx) = futures::channel::mpsc::unbounded();
                self.websockets.insert(handle, ws_tx);
                async_std::task::block_on(async move {
                    ws_connector(&url, handle, tx, ws_rx).await;
                });
            },
            WebSocketSendMessage(handle, msg) => {
                async_std::task::block_on(self.websockets.get_mut(&handle).unwrap()
                    .send(WSControl::Data(msg))).unwrap();
            },
            WebSocketClose(handle) => {
                async_std::task::block_on(self.websockets.get_mut(&handle).unwrap()
                    .send(WSControl::Close)).unwrap();
                self.websockets.remove(&handle);
            },
        }
    }
}
