// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use crate::core::{server_messages::OutboundMessage, Event};
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

type WebsocketSender = Pin<
    Box<
        dyn futures::sink::Sink<IOAction, Error = async_tungstenite::tungstenite::Error>
            + std::marker::Send,
    >,
>;
type WebsocketReceiver = Pin<
    Box<
        dyn futures::stream::FusedStream<
            Item = Result<IOEvent, async_tungstenite::tungstenite::Error>,
        > + std::marker::Send,
    >,
>;

pub struct WormholeIO {
    state: State,
    ws_tx: WebsocketSender,
    pub ws_rx: WebsocketReceiver,
}

impl WormholeIO {
    pub async fn new(relay_url: &str) -> Self {
        let (ws_tx, ws_rx) = ws_connector(&relay_url).await;
        WormholeIO {
            state: State::Connected,
            ws_tx,
            ws_rx,
        }
    }

    pub fn process_io(&mut self, event: IOEvent) -> anyhow::Result<Event> {
        use State::*;
        let action: Event;
        self.state = match self.state {
            Connected => match event {
                IOEvent::WebSocketMessageReceived(message) => {
                    action = Event::FromIO(super::server_messages::deserialize(&message));
                    Connected
                },
                IOEvent::WebSocketConnectionLost => {
                    anyhow::bail!("Initial WebSocket connection lost");
                },
            },
            Disconnecting => match event {
                IOEvent::WebSocketMessageReceived(message) => {
                    log::warn!("Received message while closing: {:?}", message);
                    action = Event::FromIO(super::server_messages::deserialize(&message));
                    Disconnecting
                },
                IOEvent::WebSocketConnectionLost => {
                    action = Event::WebsocketClosed;
                    Stopped
                },
            },
            Stopped => panic!("I don't accept events after having stopped"),
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

async fn ws_connector(url: &str) -> (WebsocketSender, WebsocketReceiver) {
    use async_tungstenite::{async_std::*, tungstenite as ws2};
    use futures::{
        sink::SinkExt,
        stream::{StreamExt, TryStreamExt},
    };

    // TODO error handling here
    let (ws_stream, _) = connect_async(url).await.unwrap();
    let (write, read) = ws_stream.split();

    /* Receive websockets event and forward them to the API */
    let ws_rx = read.try_filter_map(|message| async move {
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
    });

    /* Send events from the API to the other websocket side */
    let ws_tx = write.with(move |c| async {
        match c {
            IOAction::WebSocketSendMessage(d) => Ok(ws2::Message::Text(d)),
            IOAction::WebSocketClose => Ok(ws2::Message::Close(None)),
        }
    });

    (Box::pin(ws_tx), Box::pin(ws_rx.fuse()))
}
