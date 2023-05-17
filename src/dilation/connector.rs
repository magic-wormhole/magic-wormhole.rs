use derive_more::Display;

use crate::core::MySide;

use super::{manager, events::ConnectorEvent};

#[derive(Copy, Clone, Debug, Display, PartialEq)]
pub enum State {
    Connecting,
    Connected,
    Stopped,
}

pub struct ConnectorMachine {
    pub role: manager::Role,
    pub side: MySide,
    pub state: Option<State>,
}

impl ConnectorMachine {
    pub fn new(role: manager::Role, side: MySide) -> Self {
        ConnectorMachine {
            role,
            side,
            state: None,
        }
    }

    // called by manager
    pub fn start() {}
    pub fn stop() {}

    pub fn process(
        &mut self,
        event: ConnectorEvent,
    ) {
        use State::*;
        let current_state = self.state.unwrap();
        let new_state = match current_state {
            Connecting => match event {
                ConnectorEvent::GotHints { hints } => {
                    current_state
                },
                ConnectorEvent::AddCandidate => {
                    current_state
                },
                ConnectorEvent::ListenerReady { hints } => {
                    current_state
                },
                ConnectorEvent::Accept => {
                    Connected
                },
                ConnectorEvent::AddRelay { hints } => {
                    current_state
                },
                ConnectorEvent::Stop => {
                    Stopped
                }
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connected => match event {
                ConnectorEvent::Accept => {
                    current_state
                },
                ConnectorEvent::ListenerReady { hints } => {
                    current_state
                },
                ConnectorEvent::GotHints { hints } => {
                    current_state
                },
                ConnectorEvent::AddCandidate => {
                    current_state
                },
                ConnectorEvent::AddRelay { hints } => {
                    current_state
                },
                ConnectorEvent::Stop => {
                    Stopped
                },
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Stopped => current_state,
        };
    }
}
