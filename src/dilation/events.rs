use std::sync::mpsc::{channel, Receiver, Sender};
use derive_more::Display;
use serde_derive::{Deserialize};

use crate::core::TheirSide;
use crate::dilation::api::Action;

use super::api::{APIAction, IOAction};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    IO(IOAction),
    // All state machine events
    Manager(ManagerEvent),
}

impl From<IOAction> for Event {
    fn from(r: IOAction) -> Event {
        Event::IO(r)
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
    RxPlease { side: TheirSide },
    RxHints,
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


#[derive(Debug)]
pub struct Events {
    pub events: Vec<Event>,
}

impl Events {
    pub fn new() -> Self {
        Events {
            events: Vec::new(),
        }
    }

    pub fn addEvent(&mut self, event: Event) {
        self.events.push(event);
    }
}

pub struct ManagerChannels {
    pub inbound: Receiver<Event>,
    pub outbound: Sender<Action>,
}

impl ManagerChannels {
    pub fn new(inbound: Receiver<Event>, outbound: Sender<Action>) -> Self {
        ManagerChannels { inbound, outbound }
    }
}

pub struct ConnectionChannels {
    pub inbound: Receiver<Action>,
    pub outbound: Sender<Event>,
}

impl ConnectionChannels {
    pub fn new(inbound: Receiver<Action>, outbound: Sender<Event>) -> Self {
        ConnectionChannels { inbound, outbound }
    }
}

pub fn create_channels() -> (ConnectionChannels, ManagerChannels) {
    let (event_sender, event_receiver) = channel();
    let (action_sender, action_receiver) = channel();

    return (
        ConnectionChannels::new(action_receiver, event_sender),
        ManagerChannels::new(event_receiver, action_sender)
    );
}

#[test]
fn test_create_channels() {
    let (connection_channels, manager_channels) = create_channels();

    let event = Event::Manager(ManagerEvent::Start);
    connection_channels.outbound.send(event.clone()).expect("send failed");

    let result = manager_channels.inbound.recv().expect("receive failed");

    assert_eq!(&event, &result);
}
