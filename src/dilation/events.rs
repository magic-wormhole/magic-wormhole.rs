use derive_more::Display;
use serde_derive::Deserialize;

use crate::{
    core::TheirSide,
    dilation::api::{IOEvent, ManagerCommand},
    transit,
};

use super::api::ProtocolCommand;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub enum Event {
    //IO(IOAction),
    // All state machine events
    Manager(ManagerEvent),
    Connection(IOEvent),
}

impl From<ProtocolCommand> for ManagerCommand {
    fn from(r: ProtocolCommand) -> ManagerCommand {
        ManagerCommand::Protocol(r)
    }
}

impl From<ManagerEvent> for Event {
    fn from(r: ManagerEvent) -> Event {
        Event::Manager(r)
    }
}

// individual fsm events
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ManagerEvent {
    Start,
    #[serde(rename = "please")]
    RxPlease {
        side: TheirSide,
    },
    #[serde(rename = "connection-hints")]
    RxHints {
        hints: transit::Hints,
    },
    RxReconnect,
    RxReconnecting,
    ConnectionMade,
    ConnectionLostLeader,
    ConnectionLostFollower,
    Stop,
}

impl std::fmt::Display for ManagerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            ManagerEvent::Start => write!(f, "start"),
            ManagerEvent::RxPlease { side } => write!(f, "please"),
            ManagerEvent::RxHints { hints } => write!(f, "connection-hints"),
            ManagerEvent::RxReconnect => write!(f, "reconnect"),
            ManagerEvent::RxReconnecting => write!(f, "reconnecting"),
            ManagerEvent::ConnectionMade => write!(f, "connection-made"),
            ManagerEvent::ConnectionLostLeader => write!(f, "connection-lost-leader"),
            ManagerEvent::ConnectionLostFollower => write!(f, "connection-lost-follower"),
            ManagerEvent::Stop => write!(f, "stop"),
        }
    }
}

#[test]
fn test_manager_event_deserialisation() {
    let result: ManagerEvent =
        serde_json::from_str(r#"{"type": "please", "side": "f91dcdaccc7cc336"}"#)
            .expect("parse error");
    assert_eq!(
        result,
        ManagerEvent::RxPlease {
            side: TheirSide::from("f91dcdaccc7cc336")
        }
    );
}

// XXX: for Connector fsm events
// ...
// XXX
