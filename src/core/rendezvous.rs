// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use super::events::{Events, Nameplate, Phase};
use log::trace;
// we process these
use super::api::IOEvent;
use super::events::RendezvousEvent;
// we emit these
use super::api::IOAction;
use super::events::{BossEvent, MailboxEvent, NameplateEvent};

#[derive(Debug, PartialEq)]
enum State {
    Connected,
    Disconnecting, // -> Stopped
    Stopped,
}

#[derive(Debug)]
pub struct RendezvousMachine {
    state: Option<State>,
}

impl RendezvousMachine {
    pub fn new() -> RendezvousMachine {
        RendezvousMachine {
            state: Some(State::Connected),
        }
    }

    pub fn process_io(&mut self, event: IOEvent) -> anyhow::Result<Events> {
        use super::api::IOEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Connected => match event {
                WebSocketMessageReceived(message) => {
                    self.receive(&message, &mut actions)?;
                    old_state
                },
                WebSocketConnectionLost => {
                    anyhow::bail!("initial WebSocket connection lost");
                },
            },
            Disconnecting => match event {
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost => {
                    actions.push(BossEvent::Closed);
                    Stopped
                },
            },
            Stopped => panic!("I don't accept events after having stopped"),
        });
        Ok(actions)
    }

    pub fn process(&mut self, event: RendezvousEvent) -> Events {
        use super::events::RendezvousEvent::*;
        use State::*;
        trace!("rendezvous: {:?}", event);
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Connected => match event {
                TxBind(..) | TxOpen(..) | TxAdd(..) | TxClose(..) | TxClaim(..) | TxRelease(..)
                | TxAllocate | TxList => {
                    actions.push(self.send(event));
                    old_state
                },
                Stop => {
                    actions.push(IOAction::WebSocketClose);
                    Disconnecting
                },
            },
            Disconnecting => match event {
                Stop => old_state,
                e => panic!("Got invalid message: {:?}", e),
            },
            Stopped => match event {
                Stop => Stopped,
                _ => panic!(),
            },
        });
        actions
    }

    fn receive(&mut self, message: &str, actions: &mut Events) -> anyhow::Result<()> {
        trace!("msg is {:?}", message);
        use super::server_messages::deserialize;
        let m = deserialize(message);

        use super::server_messages::InboundMessage::*;
        match m {
            Welcome { ref welcome } => actions.push(BossEvent::RxWelcome(welcome.clone())),
            Released { .. } => actions.push(NameplateEvent::RxReleased),
            Closed { .. } => actions.push(MailboxEvent::RxClosed),
            Pong { .. } => (), // TODO
            Ack { .. } => (),  // we ignore this, it's only for the timing log
            Claimed { mailbox } => actions.push(NameplateEvent::RxClaimed(mailbox)),
            Message { side, phase, body } => actions.push(MailboxEvent::RxMessage(
                side,
                Phase(phase),
                hex::decode(body).unwrap(),
            )),
            Allocated { nameplate } => {
                actions.push(BossEvent::Allocated(Nameplate(nameplate)))
            },
            Nameplates { nameplates: _ } => {
                // TODO what does this event do?
                // I cut that code out and Wormhole seems to be still working fine?!

                // let nids: Vec<Nameplate> = nameplates
                //     .iter()
                //     .map(|n| Nameplate(n.id.to_owned()))
                //     .collect();
                // actions.push(ListerEvent::RxNameplates(nids));
            },
            Error {
                error: message,
                orig: _,
            } => {
                // TODO maybe hanlde orig field for better messages
                anyhow::bail!("Received error message from server: {}", message);
            },
            Unknown => {
                // TODO add more information once serde gets it's … done
                log::warn!("Received unknown message type from server");
            },
        };
        Ok(())
    }

    fn send(&mut self, e: RendezvousEvent) -> IOAction {
        use super::events::RendezvousEvent::*;
        use super::server_messages::OutboundMessage;
        let m = match e {
            TxBind(appid, side) => OutboundMessage::bind(appid, side),
            TxOpen(mailbox) => OutboundMessage::open(mailbox),
            TxAdd(phase, body) => OutboundMessage::add(phase, &body),
            TxClose(mailbox, mood) => OutboundMessage::close(mailbox, mood),
            TxClaim(nameplate) => OutboundMessage::claim(nameplate.0),
            TxRelease(nameplate) => OutboundMessage::release(nameplate.0),
            TxAllocate => OutboundMessage::Allocate,
            TxList => OutboundMessage::List,
            Stop => panic!(),
        };

        let ms = serde_json::to_string(&m).unwrap();
        IOAction::WebSocketSendMessage(ms)
    }
}
