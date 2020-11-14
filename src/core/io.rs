use crate::core::{IOAction, IOEvent, WSHandle};
use log::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum WSControl {
    Data(String),
    Close,
}

async fn ws_connector(
    url: &str,
    handle: WSHandle,
    mut tx: futures::channel::mpsc::UnboundedSender<IOEvent>,
    ws_rx: futures::channel::mpsc::UnboundedReceiver<WSControl>,
) {
    use async_tungstenite::async_std::*;
    use async_tungstenite::tungstenite as ws2;
    use futures::sink::SinkExt;
    use futures::stream::StreamExt;
    use futures::stream::TryStreamExt;

    let (ws_stream, _) = connect_async(url).await.unwrap();
    tx.send(IOEvent::WebSocketConnectionMade(handle))
        .await
        .unwrap();
    let (write, read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    async_std::task::spawn(async move {
        read.try_filter_map(|message| async move {
            debug!("Incoming websockets message '{:?}'", message);
            Ok(match message {
                ws2::Message::Text(text) => Some(IOEvent::WebSocketMessageReceived(handle, text)),
                ws2::Message::Close(_) => Some(IOEvent::WebSocketConnectionLost(handle)),
                ws2::Message::Ping(_) => {
                    warn!("Not responding to pings for now");
                    // TODO
                    None
                }
                ws2::Message::Pong(_) => {
                    warn!("Got a pong without ping?!");
                    // TODO maybe send pings too?
                    None
                }
                ws2::Message::Binary(_) => {
                    error!("Someone is sending binary data, this is not part of the protocol!");
                    None
                }
            })
        })
        .map_err(anyhow::Error::from)
        .forward(tx.sink_map_err(anyhow::Error::from))
        .await
        .unwrap()
    });
    /* Send events from the API to the other websocket side */
    async_std::task::spawn(async move {
        ws_rx
            .map(|c| {
                debug!("Outgoing websockets message '{:?}'", c);
                match c {
                    WSControl::Data(d) => ws2::Message::Text(d),
                    WSControl::Close => ws2::Message::Close(None),
                }
            })
            .map(Ok)
            .forward(write)
            .await
            .unwrap();
    });
}

pub struct WormholeIO {
    tx_to_core: futures::channel::mpsc::UnboundedSender<IOEvent>,
    // timers: HashSet<TimerHandle>,
    websockets: HashMap<WSHandle, futures::channel::mpsc::UnboundedSender<WSControl>>,
}

impl WormholeIO {
    pub fn new(tx_to_core: futures::channel::mpsc::UnboundedSender<IOEvent>) -> Self {
        WormholeIO {
            tx_to_core,
            // timers: HashSet::new(),
            websockets: HashMap::new(),
        }
    }

    pub fn process(&mut self, action: IOAction) {
        use self::IOAction::*;
        use futures::SinkExt;
        match action {
            // StartTimer(handle, duration) => {
            //     let mut tx = self.tx_to_core.clone();
            //     self.timers.insert(handle);
            //     async_std::task::spawn(async move {
            //         // ugh, why can't this just take a float? ok ok,
            //         // Nan, negatives, fine fine
            //         let dur_ms = (duration * 1000.0) as u64;
            //         let dur = time::Duration::from_millis(dur_ms);
            //         async_std::task::sleep(dur).await;
            //         tx.send(IOEvent::TimerExpired(handle)).await.unwrap();
            //     });
            // }
            // CancelTimer(handle) => {
            //     self.timers.remove(&handle);
            // }
            WebSocketOpen(handle, url) => {
                let tx = self.tx_to_core.clone();
                let (ws_tx, ws_rx) = futures::channel::mpsc::unbounded();
                self.websockets.insert(handle, ws_tx);
                async_std::task::block_on(async move {
                    ws_connector(&url, handle, tx, ws_rx).await;
                });
            }
            WebSocketSendMessage(handle, msg) => {
                async_std::task::block_on(
                    self.websockets
                        .get_mut(&handle)
                        .unwrap()
                        .send(WSControl::Data(msg)),
                )
                .unwrap();
            }
            WebSocketClose(handle) => {
                async_std::task::block_on(
                    self.websockets
                        .get_mut(&handle)
                        .unwrap()
                        .send(WSControl::Close),
                )
                .unwrap();
                self.websockets.remove(&handle);
            }
        }
    }
}
