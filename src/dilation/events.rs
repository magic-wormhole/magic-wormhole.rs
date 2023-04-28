use derive_more::Display;
use serde_derive::Deserialize;

use crate::{
    core::TheirSide,
    dilation::api::{IOEvent, ManagerCommand},
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
#[derive(Debug, Clone, PartialEq, Display, Deserialize)]
#[serde(tag = "type")]
pub enum ManagerEvent {
    Start,
    #[serde(rename = "please")]
    RxPlease {
        side: TheirSide,
    },
    RxHints,
    RxReconnect,
    RxReconnecting,
    ConnectionMade,
    ConnectionLostLeader,
    ConnectionLostFollower,
    Stop,
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
