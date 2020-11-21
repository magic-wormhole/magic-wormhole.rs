use crate::core::server_messages::OutboundMessage;
use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use std::collections::VecDeque;
use std::sync::Arc;

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
use super::CodeProvider;
use log::*;

use serde_derive::{Deserialize, Serialize};

/// Send an API event to the outside
// TODO manually implement Debug again to display some Vec<u8> as string and others as hex
#[derive(Debug)]
pub enum APIEvent {
    ConnectedToServer {
        /// A little welcome message from the server (message of the day and such)
        // TODO we can actually provide more structure than a "value", see the protocol
        welcome: serde_json::Value,
        /// Share this with your peer so they can connect
        code: Code,
    },

    /// The wormhole is now up and running
    ConnectedToClient {
        key: Key,
        verifier: Vec<u8>,
        versions: serde_json::Value,
    },

    GotMessage(Vec<u8>),
    /// If this message is sent, it always is the last before the channel closes
    GotError(anyhow::Error),
}

// the serialized forms of these variants are part of the wire protocol, so
// they must be spelled exactly as shown
#[derive(Debug, PartialEq, Copy, Clone, Deserialize, Serialize)]
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

enum State {
    AllocatingNameplate {
        wordlist: Arc<Wordlist>,

        /* Propagate to later states */
        side: MySide,
        versions: serde_json::Value,
    },
    ClaimingNameplate {
        nameplate: Nameplate,
        code: Code,

        /* Propagate to later states */
        side: MySide,
        versions: serde_json::Value,
    },
    Keying(Box<key::KeyMachine>),
    Running(running::RunningMachine),
    Closing {
        await_nameplate_release: bool,
        await_mailbox_close: bool,
        result: anyhow::Result<()>,
    },
}

// TODO update docs
/** The core implementation of the protocol(s)
 *
 * This is a big composite state machine that implements the Client-Server and Client-Client protocols
 * in a rather weird way. All state machines communicate with each other by sending events and actions around
 * like crazy. The wormhole is driven by processing APIActions that generate APIEvents.
 *
 * Due to the inherent asynchronous nature of IO together with these synchronous blocking state machines, generated IOEvents
 * are sent to a channel. The holder of the struct must then take care of letting the core process these by calling `do_io`.
 * */
/// Set up a WormholeCore and run it
///
/// This will create a new WormholeCore, connect its IO and API interfaces together
/// and spawn a new task that runs the event loop. A channel pair to make API calls is returned.

pub async fn run(
    appid: &AppID,
    versions: serde_json::Value,
    relay_url: &str,
    code_provider: CodeProvider,
    to_api: UnboundedSender<APIEvent>,
    mut to_core: UnboundedReceiver<Vec<u8>>,
) {
    let side = MySide::generate();
    let mut io = io::WormholeIO::new(relay_url).await;

    use futures::stream::StreamExt;

    let mut actions: VecDeque<Event> = VecDeque::new();

    /* Bootstrapping code */
    let mut state;
    actions.push_back(OutboundMessage::bind(appid.clone(), side.clone()).into());
    /* A mini state machine to track that messaage. It's okay for now, but modularize if it starts growing. */
    let mut welcome_message = None;

    match code_provider {
        CodeProvider::AllocateCode(num_words) => {
            // TODO: provide choice of wordlists
            let wordlist = Arc::new(wordlist::default_wordlist(num_words));
            actions.push_back(OutboundMessage::Allocate.into());

            state = State::AllocatingNameplate {
                wordlist,
                side,
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
                side,
                versions,
            };
        },
    }

    loop {
        let e = match actions.pop_front() {
            Some(event) => Ok(event),
            None => futures::select_biased! {
                event = io.ws_rx.select_next_some() => {
                    event.map_err(anyhow::Error::from)
                        .and_then(|event| io.process_io(event))
                },
                event = to_core.next() => {
                    /* If to_core closes ends, we shut down */
                    Ok(event.map(Event::FromAPI).unwrap_or(Event::ShutDown(Ok(()))))
                },
                complete => Err(anyhow::format_err!("IO channel closed prematurely")),
            },
        };
        /* If there's an error here the connection got down the hill so we don't do full "close". */
        let e = match e {
            Ok(e) => e,
            Err(error) => {
                to_api
                    .unbounded_send(APIEvent::GotError(error))
                    .expect("Don't close the receiver before shutting down the wormhole!");
                break;
            },
        };

        trace!("Processing: {:?}", e);
        use self::events::Event::*;
        use self::server_messages::InboundMessage;
        match e {
            FromIO(InboundMessage::Welcome { welcome }) => {
                welcome_message = Some(welcome);
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

                        actions.push_back(
                            APIEvent::ConnectedToServer {
                                /* TODO Is the welcome message mandatory or optional? */
                                welcome: welcome_message
                                    .take()
                                    .ok_or_else(|| {
                                        anyhow::format_err!("Didn't get a welcome message")
                                    })
                                    .unwrap(),
                                code,
                            }
                            .into(),
                        );
                    },
                    State::Closing { .. } => { /* This may happen. Ignore it. */ },
                    _ => {
                        actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                            "Protocol error: received message without requesting it"
                        ))));
                    },
                }
            },
            FromIO(InboundMessage::Released { .. }) => match &mut state {
                State::Keying(machine) => {
                    // TODO make more elegant with boxed patterns (once stable)
                    machine.nameplate = None;
                },
                State::Running(running::RunningMachine {
                    await_nameplate_release,
                    ..
                }) => {
                    *await_nameplate_release = false;
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
                    actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                        "Protocol error: received message without requesting it"
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
                    actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                        "Protocol error: received message in invalid state"
                    ))));
                },
            },
            FromIO(InboundMessage::Message { side, phase, body }) => {
                let message = EncryptedMessage {
                    side,
                    phase: Phase(phase),
                    body: hex::decode(body).unwrap(),
                };
                actions.push_back(Event::BounceMessage(message));
            },
            FromIO(InboundMessage::Allocated { nameplate }) => {
                if let State::AllocatingNameplate {
                    wordlist,
                    side,
                    versions,
                } = state
                {
                    let nameplate = Nameplate(nameplate);
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
                    actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                        "Protocol error: received message without requesting it"
                    ))));
                }
            },
            FromIO(InboundMessage::Nameplates { nameplates: _ }) => {
                /* We do not implement the "list" command at the moment. */
                // TODO protocol error
                actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                    "Protocol error: received message without requesting it"
                ))));
            },
            FromIO(InboundMessage::Error {
                error: message,
                orig: _,
            }) => {
                // TODO maybe hanlde orig field for better messages
                actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                    "Received error message from server: {}",
                    message
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
                    actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                        "Cannot call Send outside of State::Running"
                    ))));
                }
            },
            ShutDown(result) => match state {
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
                _ => {
                    actions.push_back(Event::ShutDown(Err(anyhow::format_err!(
                        "Protocol error: received message in invalid state"
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
