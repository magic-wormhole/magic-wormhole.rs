use crate::core::{IOAction, IOEvent};
use log::*;

async fn ws_connector(
    url: &str,
    tx: futures::channel::mpsc::UnboundedSender<IOEvent>,
    ws_rx: futures::channel::mpsc::UnboundedReceiver<IOAction>,
) {
    use async_tungstenite::async_std::*;
    use async_tungstenite::tungstenite as ws2;
    use futures::sink::SinkExt;
    use futures::stream::StreamExt;
    use futures::stream::TryStreamExt;

    // TODO error handling here
    let (ws_stream, _) = connect_async(url).await.unwrap();
    let (write, read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    async_std::task::spawn(async move {
        read.try_filter_map(|message| async move {
            debug!("Incoming websockets message '{:?}'", message);
            Ok(match message {
                ws2::Message::Text(text) => Some(IOEvent::WebSocketMessageReceived(text)),
                ws2::Message::Close(_) => Some(IOEvent::WebSocketConnectionLost),
                ws2::Message::Ping(_) => {
                    warn!("Not responding to pings for now");
                    // TODO
                    None
                },
                ws2::Message::Pong(_) => {
                    warn!("Got a pong without ping?!");
                    // TODO maybe send pings too?
                    None
                },
                ws2::Message::Binary(_) => {
                    error!("Someone is sending binary data, this is not part of the protocol!");
                    None
                },
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
                    IOAction::WebSocketSendMessage(d) => ws2::Message::Text(d),
                    IOAction::WebSocketClose => ws2::Message::Close(None),
                }
            })
            .map(Ok)
            .forward(write)
            .await
            .unwrap();
    });
}

pub struct WormholeIO {
    websocket: futures::channel::mpsc::UnboundedSender<IOAction>,
}

impl WormholeIO {
    pub async fn new(relay_url: String, tx_to_core: futures::channel::mpsc::UnboundedSender<IOEvent>) -> Self {
        let (ws_tx, ws_rx) = futures::channel::mpsc::unbounded();
        ws_connector(&relay_url, tx_to_core, ws_rx).await;

        WormholeIO {
            websocket: ws_tx,
        }
    }

    pub fn process(&mut self, action: IOAction) {
        self.websocket
            .unbounded_send(action)
            .unwrap();
    }
}
