//! Implementation of the Client-to-Server part
//!
//! Wormhole builds upon this, so you usually don't need to bother.

#[cfg(not(target_family = "wasm"))]
use async_tungstenite::tungstenite as ws2;
use futures::prelude::*;
use std::collections::VecDeque;

use crate::core::{
    server_messages::{InboundMessage, OutboundMessage, PermissionRequired, SubmitPermission},
    AppID, EncryptedMessage, Mailbox, Mood, MySide, Nameplate, Phase,
};

/// Some rendezvous server you might use.
///
/// Two applications that want to communicate with each other *must* use the same rendezvous server.
pub const DEFAULT_RENDEZVOUS_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RendezvousError {
    /// Some deserialization went wrong, we probably got some garbage
    #[error("Corrupt message received")]
    ProtocolJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
    /// A generic string message for "something went wrong", i.e.
    /// the server sent some bullshit message order
    #[error("Protocol error: {}", _0)]
    Protocol(Box<str>),
    /// The server sent us an error message
    #[error("Received error message from server: {}", _0)]
    Server(Box<str>),
    #[error(
        "Server wants one of {:?} for permissions, but we don't suppport any of these",
        _0
    )]
    Login(Vec<String>),
    #[cfg(not(target_family = "wasm"))]
    #[error("Websocket IO error")]
    IO(
        #[from]
        #[source]
        ws2::Error,
    ),
    #[cfg(target_family = "wasm")]
    #[error("Websocket IO error")]
    IO(
        #[from]
        #[source]
        ws_stream_wasm::WsErr,
    ),
}

impl RendezvousError {
    pub(self) fn protocol(error: impl Into<Box<str>>) -> Self {
        Self::Protocol(error.into())
    }

    pub(self) fn invalid_message(expected: &str, got: impl std::fmt::Debug) -> Self {
        Self::protocol(format!(
            "Unexpected message (protocol error): Expected '{}', but got: {:?}",
            expected, got
        ))
    }

    pub(self) fn server(error: impl Into<Box<str>>) -> Self {
        Self::Server(error.into())
    }
}

type MessageQueue = VecDeque<EncryptedMessage>;

#[derive(Clone, Debug, derive_more::Display)]
#[display(fmt = "{:?}", _0)]
struct NameplateList(Vec<Nameplate>);

#[cfg(not(target_family = "wasm"))]
struct WsConnection {
    connection: async_tungstenite::WebSocketStream<async_tungstenite::async_std::ConnectStream>,
}

#[cfg(target_family = "wasm")]
struct WsConnection {
    connection: ws_stream_wasm::WsStream,
    meta: ws_stream_wasm::WsMeta,
}

impl WsConnection {
    #[cfg(not(target_family = "wasm"))]
    async fn send_message(
        &mut self,
        message: &OutboundMessage,
        queue: Option<&mut MessageQueue>,
    ) -> Result<(), RendezvousError> {
        log::debug!("Sending {}", message);
        self.connection
            .send(ws2::Message::Text(serde_json::to_string(message).unwrap()))
            .await?;
        self.receive_ack(queue).await?;
        Ok(())
    }

    #[cfg(target_family = "wasm")]
    async fn send_message(
        &mut self,
        message: &OutboundMessage,
        queue: Option<&mut MessageQueue>,
    ) -> Result<(), RendezvousError> {
        log::debug!("Sending {:?}", message);
        self.connection
            .send(ws_stream_wasm::WsMessage::Text(
                serde_json::to_string(message).unwrap(),
            ))
            .await?;
        self.receive_ack(queue).await?;
        Ok(())
    }

    async fn receive_ack(
        &mut self,
        mut queue: Option<&mut MessageQueue>,
    ) -> Result<(), RendezvousError> {
        loop {
            let message = self.receive_message().await?;
            match message {
                Some(InboundMessage::Ack) => break,
                Some(InboundMessage::Message(message)) => match &mut queue {
                    Some(queue) => {
                        queue.push_back(message);
                    },
                    None => {
                        return Err(RendezvousError::protocol(
                            "Received peer message, but haven't opened the mailbox yet",
                        ));
                    },
                },
                Some(other) => {
                    return Err(RendezvousError::protocol(format!(
                        "Got unexpected message type from server '{}'",
                        other
                    )))
                },
                None => continue,
            }
        }
        Ok(())
    }

    async fn receive_reply(
        &mut self,
        mut queue: Option<&mut MessageQueue>,
    ) -> Result<RendezvousReply, RendezvousError> {
        loop {
            let message = self.receive_message().await?;
            match message {
                Some(InboundMessage::Allocated { nameplate }) => {
                    break Ok(RendezvousReply::Allocated(nameplate))
                },
                Some(InboundMessage::Released) => break Ok(RendezvousReply::Released),
                Some(InboundMessage::Claimed { mailbox }) => {
                    break Ok(RendezvousReply::Claimed(mailbox))
                },
                Some(InboundMessage::Closed) => break Ok(RendezvousReply::Closed),
                Some(InboundMessage::Message(message)) => match &mut queue {
                    Some(queue) => {
                        queue.push_back(message);
                    },
                    None => {
                        break Err(RendezvousError::protocol(
                            "Received peer message, but haven't opened the mailbox yet",
                        ))
                    },
                },
                Some(InboundMessage::Error { error, orig: _ }) => {
                    break Err(RendezvousError::Server(error.into()));
                },
                Some(InboundMessage::Nameplates { nameplates }) => {
                    break Ok(RendezvousReply::Nameplates(NameplateList(nameplates)))
                },
                Some(other) => {
                    break Err(RendezvousError::protocol(format!(
                        "Got unexpected message type from server '{}'",
                        other
                    )))
                },
                None => (/*continue*/),
            }
        }
    }

    async fn receive_message_some(&mut self) -> Result<InboundMessage, RendezvousError> {
        loop {
            if let Some(message) = self.receive_message().await? {
                break Ok(message);
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    async fn receive_message(&mut self) -> Result<Option<InboundMessage>, RendezvousError> {
        let message = self
            .connection
            .next()
            .await
            .expect("TODO this should always be Some")?;
        match message {
            ws2::Message::Text(message_plain) => {
                let message = serde_json::from_str(&message_plain)?;
                log::debug!("Received {}", message);
                match message {
                    InboundMessage::Unknown => {
                        log::warn!("Got unknown message, ignoring: '{}'", message_plain);
                        Ok(None)
                    },
                    InboundMessage::Error { error, orig: _ } => Err(RendezvousError::server(error)),
                    message => Ok(Some(message)),
                }
            },
            ws2::Message::Binary(_) => Err(RendezvousError::protocol(
                "WebSocket messages must be UTF-8 encoded text",
            )),
            /* Ignore ping pong for now */
            ws2::Message::Ping(_) => Ok(None),
            ws2::Message::Pong(_) => Ok(None),
            ws2::Message::Close(_) => {
                log::debug!("Received connection close");
                Err(ws2::Error::ConnectionClosed.into())
            },
            ws2::Message::Frame(_) => {
                log::warn!("Received a WebSocket 'Frame' message and don't know what to do with it, please open a bug report");
                Ok(None)
            },
        }
    }

    #[cfg(target_family = "wasm")]
    async fn receive_message(&mut self) -> Result<Option<InboundMessage>, RendezvousError> {
        let message = self
            .connection
            .next()
            .await
            .expect("TODO this should always be Some");
        match message {
            ws_stream_wasm::WsMessage::Text(message_plain) => {
                let message = serde_json::from_str(&message_plain)?;
                log::debug!("Received {:?}", message);
                match message {
                    InboundMessage::Unknown => {
                        log::warn!("Got unknown message, ignoring: '{}'", message_plain);
                        Ok(None)
                    },
                    InboundMessage::Error { error, orig: _ } => Err(RendezvousError::server(error)),
                    message => Ok(Some(message)),
                }
            },
            ws_stream_wasm::WsMessage::Binary(_) => Err(RendezvousError::protocol(
                "WebSocket messages must be UTF-8 encoded text",
            )),
        }
    }

    #[cfg(not(target_family = "wasm"))]
    async fn close(&mut self) -> Result<(), ws2::Error> {
        self.connection.close(None).await
    }

    #[cfg(target_family = "wasm")]
    async fn close(&mut self) -> Result<ws_stream_wasm::CloseEvent, ws_stream_wasm::WsErr> {
        self.meta.close().await
    }
}

#[derive(Clone, Debug, derive_more::Display)]
enum RendezvousReply {
    Allocated(Nameplate),
    Released,
    Claimed(Mailbox),
    Closed,
    Nameplates(NameplateList),
}

#[derive(Clone, Debug, derive_more::Display)]
#[display(
    fmt = "MailboxMachine {{ mailbox: {}, processed: [{}] }}",
    mailbox,
    "processed.iter().map(|p| format!(\"{}\", p)).collect::<Vec<String>>().join(\", \")"
)]
struct MailboxMachine {
    nameplate: Option<Nameplate>,
    mailbox: Mailbox,
    queue: MessageQueue,
    processed: std::collections::HashSet<Phase>,
}

impl MailboxMachine {
    fn receive_message(&mut self, message: &EncryptedMessage, side: &MySide) -> bool {
        if *message.side != **side {
            // Got a message from them. Check if duplicate
            if !self.processed.contains(&message.phase) {
                self.processed.insert(message.phase.clone());
                true
            } else {
                false
            }
        } else {
            // Echo of ours. Ignore
            false
        }
    }
}

#[deprecated(
    since = "0.7.0",
    note = "This will be a private type in the future. Open an issue if you require access to protocol intrinsics in the future"
)]
pub struct RendezvousServer {
    connection: WsConnection,
    state: Option<MailboxMachine>,
    side: MySide,
}

#[allow(deprecated)]
impl std::fmt::Debug for RendezvousServer {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("RendezvousServer")
            .field("state", &self.state)
            .field("side", &self.side)
            .finish()
    }
}

#[allow(deprecated)]
impl RendezvousServer {
    /**
     * Connect to the rendezvous server
     *
     * This does the permission negotiation part if required and binds the
     * connection to the given `appid`.
     */
    pub async fn connect(
        appid: &AppID,
        relay_url: &str,
    ) -> Result<(Self, Option<String>), RendezvousError> {
        let side = MySide::generate();
        let mut connection;

        #[cfg(not(target_arch = "wasm32"))]
        {
            let (stream, _) = async_tungstenite::async_std::connect_async(relay_url).await?;
            connection = WsConnection { connection: stream };
        }

        #[cfg(target_arch = "wasm32")]
        {
            let (meta, stream) = ws_stream_wasm::WsMeta::connect(relay_url, None).await?;
            connection = WsConnection {
                meta,
                connection: stream,
            };
        }

        let welcome = match connection.receive_message_some().await? {
            InboundMessage::Welcome { welcome } => welcome,
            other => {
                return Err(RendezvousError::protocol(format!(
                    "First message server sends must be 'welcome', but was '{}'",
                    other
                )))
            },
        };

        match welcome.permission_required {
            Some(PermissionRequired {
                hashcash: Some(hashcash),
                ..
            }) => {
                let token = crate::util::hashcash(hashcash.resource, hashcash.bits);
                connection
                    .send_message(
                        &OutboundMessage::SubmitPermission(SubmitPermission::Hashcash {
                            stamp: token.to_string(),
                        }),
                        None,
                    )
                    .await?;
            },
            Some(PermissionRequired { none: true, .. }) => (),
            Some(PermissionRequired { other, .. }) => {
                /* We can't actually log in :/ */
                return Err(RendezvousError::Login(
                    // TODO use `into_keys` once stable and remove the `cloned`
                    other.keys().cloned().collect(),
                ));
            },
            None => (),
        }

        connection
            .send_message(&OutboundMessage::bind(appid.clone(), side.clone()), None)
            .await?;

        log::info!("Connected to rendezvous server.");

        Ok((
            Self {
                connection,
                state: None,
                side,
            },
            welcome.motd,
        ))
    }

    /** A random unique string for this session */
    pub(crate) fn side(&self) -> &MySide {
        &self.side
    }

    async fn send_message(&mut self, message: &OutboundMessage) -> Result<(), RendezvousError> {
        self.connection
            .send_message(message, self.state.as_mut().map(|state| &mut state.queue))
            .await
    }

    async fn receive_reply(&mut self) -> Result<RendezvousReply, RendezvousError> {
        self.connection
            .receive_reply(self.state.as_mut().map(|state| &mut state.queue))
            .await
    }

    pub(crate) async fn send_peer_message(
        &mut self,
        phase: Phase,
        body: Vec<u8>,
    ) -> Result<(), RendezvousError> {
        self.send_message(&OutboundMessage::Add { body, phase })
            .await
    }

    pub(crate) async fn next_peer_message_some(
        &mut self,
    ) -> Result<EncryptedMessage, RendezvousError> {
        loop {
            if let Some(message) = self.next_peer_message().await? {
                return Ok(message);
            }
        }
    }

    pub(crate) async fn next_peer_message(
        &mut self,
    ) -> Result<Option<EncryptedMessage>, RendezvousError> {
        let machine = &mut self
            .state
            .as_mut()
            .expect("Can only receive messages when having a claimed+open mailbox");
        if let Some(message) = machine.queue.pop_front() {
            if machine.receive_message(&message, &self.side) {
                return Ok(Some(message));
            } else {
                return Ok(None);
            }
        }
        match self.connection.receive_message().await? {
            Some(InboundMessage::Message(message)) => {
                if machine.receive_message(&message, &self.side) {
                    Ok(Some(message))
                } else {
                    Ok(None)
                }
            },
            Some(other) => Err(RendezvousError::protocol(format!(
                "Expected message from peer, got '{}' instead",
                other
            ))),
            None => Ok(None),
        }
    }

    /** Allocate a nameplate, claim the mailbox and open it */
    pub async fn allocate_claim_open(&mut self) -> Result<(Nameplate, Mailbox), RendezvousError> {
        assert!(
            self.state.is_none(),
            "Can only call in initial state, and only once"
        );

        self.send_message(&OutboundMessage::Allocate).await?;
        let nameplate = match self.receive_reply().await? {
            RendezvousReply::Allocated(nameplate) => nameplate,
            other => return Err(RendezvousError::invalid_message("allocated", other)),
        };

        self.send_message(&OutboundMessage::claim(nameplate.clone()))
            .await?;
        let mailbox = match self.receive_reply().await? {
            RendezvousReply::Claimed(mailbox) => mailbox,
            other => return Err(RendezvousError::invalid_message("claimed", other)),
        };

        self.send_message(&OutboundMessage::open(mailbox.clone()))
            .await?;

        self.state = Some(MailboxMachine {
            nameplate: Some(nameplate.clone()),
            mailbox: mailbox.clone(),
            queue: Default::default(),
            processed: Default::default(),
        });
        Ok((nameplate, mailbox))
    }

    /** Claim a nameplate+mailbox and open it */
    pub async fn claim_open(&mut self, nameplate: Nameplate) -> Result<Mailbox, RendezvousError> {
        assert!(
            self.state.is_none(),
            "Can only call in initial state, and only once"
        );

        self.send_message(&OutboundMessage::claim(nameplate.clone()))
            .await?;
        let mailbox = match self.receive_reply().await? {
            RendezvousReply::Claimed(mailbox) => mailbox,
            other => return Err(RendezvousError::invalid_message("claimed", other)),
        };

        self.send_message(&OutboundMessage::open(mailbox.clone()))
            .await?;

        self.state = Some(MailboxMachine {
            nameplate: Some(nameplate.clone()),
            mailbox: mailbox.clone(),
            queue: Default::default(),
            processed: Default::default(),
        });
        Ok(mailbox)
    }

    pub fn needs_nameplate_release(&self) -> bool {
        self.state
            .as_ref()
            .and_then(|state| state.nameplate.as_ref())
            .is_some()
    }

    /**
     * Gets the list of currently claimed nameplates.
     * This can be called at any time.
     */
    pub async fn list_nameplates(&mut self) -> Result<Vec<Nameplate>, RendezvousError> {
        self.send_message(&OutboundMessage::List).await?;
        let nameplate_reply = self.receive_reply().await?;
        match nameplate_reply {
            RendezvousReply::Nameplates(x) => Ok(x.0),
            other => Err(RendezvousError::invalid_message("nameplates", other)),
        }
    }

    pub async fn release_nameplate(&mut self) -> Result<(), RendezvousError> {
        let nameplate = &mut self
            .state
            .as_mut()
            .and_then(|state| state.nameplate.clone())
            .expect("Can only release an allocated nameplate, and only once");

        use std::ops::Deref;
        self.send_message(&OutboundMessage::release(nameplate.deref().deref()))
            .await?;
        match self.receive_reply().await? {
            RendezvousReply::Released => (),
            other => return Err(RendezvousError::invalid_message("released", other)),
        };
        self.state.as_mut().unwrap().nameplate = None;
        Ok(())
    }

    /**
     * Open a mailbox while skipping the nameplate part.
     *
     * This is the base functionality for seeds.
     */
    pub async fn open_directly(&mut self, mailbox: Mailbox) -> Result<(), RendezvousError> {
        assert!(
            self.state.is_none(),
            "Can only call in initial state, and only once"
        );
        self.send_message(&OutboundMessage::open(mailbox.clone()))
            .await?;
        self.state = Some(MailboxMachine {
            nameplate: None,
            mailbox,
            queue: Default::default(),
            processed: Default::default(),
        });
        Ok(())
    }

    pub async fn shutdown(mut self, mood: Mood) -> Result<(), RendezvousError> {
        if let Some(MailboxMachine {
            nameplate,
            mailbox,
            mut queue,
            ..
        }) = self.state
        {
            if let Some(nameplate) = nameplate {
                self.connection
                    .send_message(&OutboundMessage::release(nameplate), Some(&mut queue))
                    .await?;
                match self.connection.receive_reply(Some(&mut queue)).await? {
                    RendezvousReply::Released => (),
                    other => return Err(RendezvousError::invalid_message("released", other)),
                };
            }

            self.connection
                .send_message(&OutboundMessage::close(mailbox, mood), Some(&mut queue))
                .await?;
            match self.connection.receive_reply(Some(&mut queue)).await? {
                RendezvousReply::Closed => (),
                other => return Err(RendezvousError::invalid_message("closed", other)),
            };
        }

        self.connection.close().await?;
        Ok(())
    }
}
