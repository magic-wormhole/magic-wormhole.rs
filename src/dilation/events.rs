use derive_more::Display;
use serde_derive::Deserialize;

use crate::{
    core::TheirSide,
    dilation::api::{IOEvent, ManagerCommand},
    transit::Hints,
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
#[derive(Display, Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type")]
pub enum ManagerEvent {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "please")]
    RxPlease {
        side: TheirSide,
    },
    #[serde(rename = "connection-hints")]
    RxHints {
        hints: Hints,
    },
    RxReconnect,
    RxReconnecting,
    ConnectionMade,
    ConnectionLostLeader,
    ConnectionLostFollower,
    Stop,
}

// XXX: for Connector fsm events
// ...
// XXX

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_display_please_event() {
        let event = ManagerEvent::RxPlease {
            side: TheirSide::from("f91dcdaccc7cc336"),
        };
        assert_eq!(format!("{}", event), "TheirSide(f91dcdaccc7cc336)");
    }

    #[test]
    fn test_manager_event_deserialisation_start() {
        let result: ManagerEvent =
            serde_json::from_str(r#"{"type": "start"}"#).expect("parse error");
        assert_eq!(result, ManagerEvent::Start);
    }

    #[test]
    fn test_manager_event_deserialisation_rxplease() {
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
}
