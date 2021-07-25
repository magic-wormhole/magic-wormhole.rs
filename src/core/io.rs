// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use crate::core::{server_messages::OutboundMessage, Event, WormholeCoreError};
use log::*;
use std::pin::Pin;

/// Receive an event from the IO task loop
#[derive(Debug, PartialEq)]
pub enum IOEvent {
    WebSocketMessageReceived(String),
    WebSocketConnectionLost,
}

/// Send an action to the IO task loop
#[derive(Debug, PartialEq)]
enum IOAction {
    WebSocketSendMessage(String),
    WebSocketClose,
}

#[derive(Debug, PartialEq)]
enum State {
    Connected,
    Disconnecting, // -> Stopped
    Stopped,
}

type WebsocketSender =
    Pin<Box<dyn futures::sink::Sink<IOAction, Error = WormholeCoreError> + std::marker::Send>>;
type WebsocketReceiver = Pin<
    Box<
        dyn futures::stream::FusedStream<Item = Result<IOEvent, WormholeCoreError>>
            + std::marker::Send,
    >,
>;

pub struct WormholeIO {
    state: State,
    ws_tx: WebsocketSender,
    pub ws_rx: WebsocketReceiver,
}

impl WormholeIO {
    pub async fn new(relay_url: &str) -> Result<Self, async_tungstenite::tungstenite::Error> {
        let (ws_tx, ws_rx) = ws_connector(&relay_url).await?;
        Ok(WormholeIO {
            state: State::Connected,
            ws_tx,
            ws_rx,
        })
    }

    pub fn process_io(&mut self, event: IOEvent) -> Result<Event, WormholeCoreError> {
        use State::*;
        let action: Event;
        self.state = match self.state {
            Connected => match event {
                IOEvent::WebSocketMessageReceived(message) => {
                    action = Event::FromIO(super::server_messages::deserialize(&message)?);
                    Connected
                },
                IOEvent::WebSocketConnectionLost => {
                    return Err(WormholeCoreError::protocol(
                        "Unexpectedly lost connection (WebSocket closed)",
                    ));
                },
            },
            Disconnecting => match event {
                IOEvent::WebSocketMessageReceived(message) => {
                    log::warn!("Received message while closing: {:?}", message);
                    action = Event::FromIO(super::server_messages::deserialize(&message)?);
                    Disconnecting
                },
                IOEvent::WebSocketConnectionLost => {
                    action = Event::WebsocketClosed;
                    Stopped
                },
            },
            Stopped => {
                return Err(WormholeCoreError::protocol(
                    "Received a WebSocket event after having stopped",
                ));
            },
        };
        Ok(action)
    }

    pub async fn send(&mut self, m: OutboundMessage) {
        if let State::Connected = self.state {
            use futures::sink::SinkExt;
            let message = serde_json::to_string(&m).unwrap();
            self.ws_tx
                .send(IOAction::WebSocketSendMessage(message))
                .await
                .unwrap();
        } else {
            unreachable!();
        }
    }

    pub async fn stop(&mut self) {
        if let State::Connected = self.state {
            use futures::sink::SinkExt;
            self.ws_tx.send(IOAction::WebSocketClose).await.unwrap();
            self.state = State::Disconnecting;
        };
    }
}

async fn ws_connector(
    url: &str,
) -> Result<(WebsocketSender, WebsocketReceiver), async_tungstenite::tungstenite::Error> {
    use async_tungstenite::{async_std::*, tungstenite as ws2};
    use futures::{
        sink::SinkExt,
        stream::{StreamExt, TryStreamExt},
    };

    let (ws_stream, _) = connect_async(url).await?;
    let (write, read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    let ws_rx = read
        .map_err(WormholeCoreError::IO)
        .try_filter_map(|message| async move {
            match message {
                ws2::Message::Text(text) => Ok(Some(IOEvent::WebSocketMessageReceived(text))),
                ws2::Message::Close(_) => Ok(Some(IOEvent::WebSocketConnectionLost)),
                ws2::Message::Ping(_) => {
                    warn!("Not responding to pings for now");
                    // TODO
                    Ok(None)
                },
                ws2::Message::Pong(_) => {
                    warn!("Got a pong without ping?!");
                    // TODO maybe send pings too?
                    Ok(None)
                },
                ws2::Message::Binary(_) => Err(WormholeCoreError::protocol(
                    "Received some binary data; this is not part of the protocol!",
                )),
            }
        });

    /* Send events from the API to the other websocket side */
    let ws_tx = write
        .sink_map_err(WormholeCoreError::IO)
        .with(move |c| async {
            match c {
                IOAction::WebSocketSendMessage(d) => Ok(ws2::Message::Text(d)),
                IOAction::WebSocketClose => Ok(ws2::Message::Close(None)),
            }
        });

    Ok((Box::pin(ws_tx), Box::pin(ws_rx.fuse())))
}
