use crate::core::server_messages::OutboundMessage;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use std::{collections::VecDeque, sync::Arc};

#[macro_use]
mod events;
mod io;
pub mod key;
mod mailbox;
mod running;
mod server_messages;
#[cfg(test)]
mod test;
mod util;
mod wordlist;

use self::events::*;
pub use self::events::{AppID, Code};
pub use self::server_messages::EncryptedMessage;
use super::CodeProvider;
use log::*;

use serde_derive::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WormholeCoreError {
    /// Some deserialization went wrong, we probably got some garbage
    #[error("Corrupt message received")]
    ProtocolJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
    #[error("Corrupt hex string encountered within a message")]
    ProtocolHex(
        #[from]
        #[source]
        hex::FromHexError,
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
    #[error(
        "Key confirmation failed. If you didn't mistype the code, \
        this is a sign of an attacker guessing passwords. Please try \
        again some time later."
    )]
    PakeFailed,
    #[error("Cannot decrypt a received message")]
    Crypto,
    #[error("Websocket IO error")]
    IO(
        #[from]
        #[source]
        async_tungstenite::tungstenite::Error,
    ),
}

impl WormholeCoreError {
    pub(self) fn protocol(error: impl Into<Box<str>>) -> Self {
        Self::Protocol(error.into())
    }

    /** Should we tell the server that we are "errory" or "scared"? */
    pub fn is_scared(&self) -> bool {
        matches!(self, Self::PakeFailed)
    }
}

impl From<std::convert::Infallible> for WormholeCoreError {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

/// Send an API event to the outside
// TODO manually implement Debug again to display some Vec<u8> as string and others as hex
#[derive(Debug, derive_more::Display)]
pub enum APIEvent {
    #[display(
        fmt = "ConnectedToServer {{ motd: {} }}",
        r#"motd.as_deref().unwrap_or("<none>")"#
    )]
    ConnectedToServer {
        /// A little welcome message from the server (message of the day and such)
        motd: Option<String>,
    },
    #[display(fmt = "GotCode {{ code: {} }}", code)]
    GotCode {
        /// Share this with your peer so they can connect
        code: Code,
    },

    /// The wormhole is now up and running
    #[display(
        fmt = "ConnectedToClient {{ key: <censored>, verifier: {:x?}, versions: {} }}",
        verifier,
        versions
    )]
    ConnectedToClient {
        key: Box<xsalsa20poly1305::Key>,
        verifier: Box<xsalsa20poly1305::Key>,
        versions: serde_json::Value,
    },

    #[display(fmt = "GotMessage({})", "crate::util::DisplayBytes(_0)")]
    GotMessage(Vec<u8>),
    /// If this message is sent, it always is the last before the channel closes
    GotError(WormholeCoreError),
}

// the serialized forms of these variants are part of the wire protocol, so
// they must be spelled exactly as shown
#[derive(Debug, PartialEq, Copy, Clone, Deserialize, Serialize, derive_more::Display)]
pub enum Mood {
    #[serde(rename = "happy")]
    Happy,
    #[serde(rename = "lonely")]
    Lonely,
    #[serde(rename = "errory")]
    Errory,
    #[serde(rename = "scary")]
    Scared,
    #[serde(rename = "unwelcome")]
    Unwelcome,
}

#[derive(Debug, derive_more::Display)]
enum State {
    #[display(fmt = "")] // TODO
    WaitForWelcome {
        versions: serde_json::Value,
        code_provider: CodeProvider,
    },

    #[display(
        fmt = "AllocatingNameplate {{ wordlist: <{} words>, side: {}, versions: {} }}",
        "wordlist.num_words",
        side,
        versions
    )]
    AllocatingNameplate {
        wordlist: Arc<Wordlist>,

        /* Propagate to later states */
        side: MySide,
        versions: serde_json::Value,
    },
    #[display(
        fmt = "ClaimingNameplate {{ nameplate: {}, code: {}, side: {}, versions: {} }}",
        nameplate,
        code,
        side,
        versions
    )]
    ClaimingNameplate {
        nameplate: Nameplate,
        code: Code,

        /* Propagate to later states */
        side: MySide,
        versions: serde_json::Value,
    },
    #[display(fmt = "Keying({})", _0)]
    Keying(Box<key::KeyMachine>),
    #[display(fmt = "Running({})", _0)]
    Running(Box<running::RunningMachine>),
    #[display(
        fmt = "Closing {{ await_nameplate_release: {}, await_mailbox_close: {}, result: {:?} }}",
        await_nameplate_release,
        await_mailbox_close,
        result
    )]
    Closing {
        await_nameplate_release: bool,
        await_mailbox_close: bool,
        result: Result<(), WormholeCoreError>,
    },
}

/** The core implementation of the protocol(s)
 *
 * This runs the main event loop that will do all the work. The code is implemented as a state machine, sometimes with
 * nested state (a state machine in a state machine).
 */
pub async fn run(
    appid: &AppID,
    versions: serde_json::Value,
    relay_url: &str,
    code_provider: CodeProvider,
    to_api: UnboundedSender<APIEvent>,
    mut to_core: UnboundedReceiver<Vec<u8>>,
) {
    let side = MySide::generate();
    // TODO somehow move this into the generic error handling loop, because of code duplication
    let mut io = match io::WormholeIO::new(relay_url).await {
        Ok(io) => io,
        Err(error) => {
            to_api
                .unbounded_send(APIEvent::GotError(WormholeCoreError::IO(error)))
                .expect("Don't close the receiver before shutting down the wormhole!");
            return;
        },
    };

    use futures::stream::StreamExt;

    let mut actions: VecDeque<Event> = VecDeque::new();

    let mut state = State::WaitForWelcome {
        versions,
        code_provider,
    };

    /* The usual main loop */
    loop {
        let e = match actions.pop_front() {
            Some(event) => Ok(event),
            None => futures::select_biased! {
                event = io.ws_rx.select_next_some() => {
                    event.and_then(|event| io.process_io(event))
                },
                event = to_core.next() => {
                    /* If to_core closes ends, we shut down */
                    Ok(event.map(Event::FromAPI).unwrap_or(Event::ShutDown(Ok(()))))
                },
                complete => Err(WormholeCoreError::protocol(
                    "IO channel closed prematurely"
                )),
            },
        };
        /* If there's an error here the connection got down the hill so we don't do full "close". */
        let e = match e {
            Ok(e) => e,
            Err(error) => {
                debug!(
                    "Stopping wormhole event loop because of IO Error: {:?}",
                    error
                );
                to_api
                    .unbounded_send(APIEvent::GotError(error))
                    .expect("Don't close the receiver before shutting down the wormhole!");
                break;
            },
        };

        trace!("[{}] State: {}", &**side, &state);
        debug!("[{}] Processing: {}", &**side, e);
        use self::{events::Event::*, server_messages::InboundMessage};
        match e {
            FromIO(InboundMessage::Welcome { welcome }) => {
                match state {
                    State::WaitForWelcome {
                        versions,
                        code_provider,
                    } => {
                        use server_messages::{PermissionRequired, SubmitPermission};

                        actions
                            .push_back(APIEvent::ConnectedToServer { motd: welcome.motd }.into());

                        match welcome.permission_required {
                            Some(PermissionRequired {
                                hashcash: Some(hashcash),
                                ..
                            }) => {
                                let token = hashcash::Token::new(hashcash.resource, hashcash.bits);
                                actions.push_back(
                                    OutboundMessage::SubmitPermission(SubmitPermission::Hashcash {
                                        stamp: token.to_string(),
                                    })
                                    .into(),
                                )
                            },
                            Some(PermissionRequired { none: true, .. }) => (),
                            Some(PermissionRequired { other, .. }) => {
                                /* We can't actually log in :/ */
                                actions.push_back(Event::ShutDown(Err(WormholeCoreError::Login(
                                    // TODO use `into_keys` once stable and remove the `cloned`
                                    other.keys().cloned().collect(),
                                ))));
                            },
                            None => (),
                        }

                        actions
                            .push_back(OutboundMessage::bind(appid.clone(), side.clone()).into());

                        match code_provider {
                            CodeProvider::AllocateCode(num_words) => {
                                // TODO: provide choice of wordlists
                                let wordlist = Arc::new(wordlist::default_wordlist(num_words));
                                actions.push_back(OutboundMessage::Allocate.into());

                                state = State::AllocatingNameplate {
                                    wordlist,
                                    side: side.clone(),
                                    versions,
                                };
                            },
                            CodeProvider::SetCode(code) => {
                                let code_string = code.to_string();
                                let nc: Vec<&str> = code_string.splitn(2, '-').collect();
                                let nameplate = Nameplate::new(nc[0]);
                                actions.push_back(OutboundMessage::claim(nameplate.clone()).into());

                                state = State::ClaimingNameplate {
                                    nameplate,
                                    code: Code(code),
                                    side: side.clone(),
                                    versions,
                                };
                            },
                        }
                    },
                    _ => unreachable!(),
                }
            },
            FromIO(InboundMessage::Claimed { mailbox }) => {
                match state {
                    State::ClaimingNameplate {
                        nameplate,
                        code,
                        side,
                        versions,
                    } => {
                        actions.push_back(OutboundMessage::open(mailbox.clone()).into());

                        state = State::Keying(Box::new(key::KeyMachine::start(
                            &mut actions,
                            &appid,
                            side,
                            versions,
                            nameplate,
                            mailbox,
                            &code,
                        )));

                        actions.push_back(APIEvent::GotCode { code }.into());
                    },
                    State::Closing { .. } => { /* This may happen. Ignore it. */ },
                    _ => {
                        actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                            "Received message without requesting it",
                        ))));
                    },
                }
            },
            FromIO(InboundMessage::Released { .. }) => match &mut state {
                State::Keying(machine) => {
                    // TODO make more elegant with boxed patterns (once stable)
                    machine.nameplate = None;
                },
                State::Running(machine) => {
                    // TODO make more elegant with boxed patterns (once stable)
                    machine.await_nameplate_release = false;
                },
                State::Closing {
                    await_nameplate_release,
                    await_mailbox_close,
                    ..
                } => {
                    *await_nameplate_release = false;
                    if !*await_mailbox_close && !*await_nameplate_release {
                        actions.push_back(Event::CloseWebsocket);
                    }
                },
                _ => {
                    actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                        "Received message without requesting it",
                    ))));
                },
            },
            FromIO(InboundMessage::Closed) => match &mut state {
                State::Closing {
                    await_mailbox_close,
                    await_nameplate_release,
                    ..
                } => {
                    *await_mailbox_close = false;
                    if !*await_mailbox_close && !*await_nameplate_release {
                        actions.push_back(Event::CloseWebsocket);
                    }
                },
                _ => {
                    actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                        "Received message in invalid state",
                    ))));
                },
            },
            FromIO(InboundMessage::Message(message)) => {
                actions.push_back(Event::BounceMessage(message));
            },
            FromIO(InboundMessage::Allocated { nameplate }) => {
                if let State::AllocatingNameplate {
                    wordlist,
                    side,
                    versions,
                } = state
                {
                    let words = wordlist.choose_words();
                    // TODO: assert code.startswith(nameplate+"-")
                    let code = Code(format!("{}-{}", &nameplate, &words));
                    actions.push_back(OutboundMessage::claim(nameplate.clone()).into());
                    state = State::ClaimingNameplate {
                        nameplate,
                        code,
                        side,
                        versions,
                    };
                } else {
                    // TODO protocol error
                    actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                        "Received message without requesting it",
                    ))));
                }
            },
            FromIO(InboundMessage::Nameplates { nameplates: _ }) => {
                /* We do not implement the "list" command at the moment. */
                // TODO protocol error
                actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                    "Received message without requesting it",
                ))));
            },
            FromIO(InboundMessage::Error {
                error: message,
                orig: _,
            }) => {
                // TODO maybe hanlde orig field for better messages
                actions.push_back(Event::ShutDown(Err(WormholeCoreError::Server(
                    message.into(),
                ))));
            },
            FromIO(InboundMessage::Pong { .. }) | FromIO(InboundMessage::Ack { .. }) => (), /* we ignore this, it's only for the timing log */
            FromIO(InboundMessage::Unknown) => {
                // TODO add more information once serde gets it's […] done
                log::warn!("Received unknown message type from server");
            },
            FromAPI(plaintext) => {
                if let State::Running(machine) = state {
                    state = machine.send_message(&mut actions, plaintext);
                } else {
                    // TODO print current state's name
                    actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                        "Cannot call Send outside of State::Running",
                    ))));
                }
            },
            ShutDown(result) => match state {
                State::WaitForWelcome { .. } => {
                    state = State::Closing {
                        await_nameplate_release: false,
                        await_mailbox_close: false,
                        result,
                    };
                },
                State::AllocatingNameplate { .. } => {
                    state = State::Closing {
                        await_nameplate_release: false,
                        await_mailbox_close: false,
                        result,
                    };
                },
                State::ClaimingNameplate { nameplate, .. } => {
                    // TODO do we need to send "close" to the server before having opened the mailbox?
                    actions.push_back(OutboundMessage::release(nameplate).into());
                    state = State::Closing {
                        await_nameplate_release: true,
                        await_mailbox_close: false,
                        result,
                    };
                },
                State::Keying(machine) => {
                    state = machine.shutdown(&mut actions, result);
                },
                State::Running(machine) => {
                    state = machine.shutdown(&mut actions, result);
                },
                State::Closing { .. } => {
                    // May somehow keep the state but somehow chain the error with another message?
                    // Or log it and do nothing?
                    todo!("I don't know how to handle this");
                },
            },
            BounceMessage(message) => match state {
                State::Keying(machine) => {
                    state = machine.receive_message(&mut actions, message);
                },
                State::Running(machine) => {
                    state = machine.receive_message(&mut actions, message);
                },
                State::Closing { .. } => {
                    /* If we're closing, simply ignore any incoming messages.
                     * We could decrypt them if we hadn't dropped the key at this point, but eeh
                     */
                },
                _ => {
                    actions.push_back(Event::ShutDown(Err(WormholeCoreError::protocol(
                        "Received message in invalid state",
                    ))));
                },
            },
            WebsocketClosed => match state {
                State::Closing { result, .. } => {
                    if let Err(error) = result {
                        /* We cannot use the usual way of queueing it up as event because we're about to quit */
                        to_api
                            .unbounded_send(APIEvent::GotError(error))
                            .expect("Don't close the receiver before shutting down the wormhole!");
                    }
                    // Don't assign a new state here
                    break;
                },
                _ => unreachable!(),
            },
            ToAPI(action) => {
                to_api
                    .unbounded_send(action)
                    .expect("Don't close the receiver before shutting down the wormhole!");
            },
            ToIO(message) => {
                io.send(message).await;
            },
            CloseWebsocket => {
                io.stop().await;
            },
        }
    }
    to_api.close_channel();
    to_core.close();
    debug!("Stopped wormhole event loop");
}
