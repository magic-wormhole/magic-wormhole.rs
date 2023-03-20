use super::events::{Events, Event};
use super::events::ManagerEvent;
use super::api::{IOAction, IOEvent};

pub struct ManagerMachine {
    state: Option<State>
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum State {
    Waiting,
    Wanting,
    Connecting,
    Connected,
    Abandoning,
    Flushing,
    Lonely,
    Stopping,
    Stopped,
}

impl ManagerMachine {
    pub fn new() -> Self {
        ManagerMachine {
            state: Some(State::Waiting),
        }
    }

    pub fn process_io(&mut self, event: IOEvent) -> Events {
        // XXX: which Manager states process IO events?
        let mut actions = Events::new();

        // XXX: big match expression here

        actions
    }

    pub fn process(&mut self, event: ManagerEvent) -> Events {
        // given the event and the current state, generate output
        // event and move to the new state
        use State::*;
        let mut actions = Events::new();
        let current_state = self.state.unwrap();
        let new_state = match current_state {
            Waiting => match event {
                ManagerEvent::Start => {
                    actions.addEvent(Event::from(IOAction::SendPlease));
                    Wanting
                }
                ManagerEvent::Stop => {
                    // actions.addAction(NotifyStopped)
                    Stopped
                }
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Wanting => match event {
                ManagerEvent::RxPlease { side: theirSide }  => {
                    Connecting
                }
                ManagerEvent::Stop => {
                    Stopped
                }
                ManagerEvent::RxHints => {
                    current_state
                }
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Connecting => match event {
                ManagerEvent::RxHints => {
                    current_state
                },
                ManagerEvent::Stop => {
                    Stopped
                },
                ManagerEvent::ConnectionMade => {
                    Connected
                }
                ManagerEvent::RxReconnect => {
                    current_state
                }
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Connected => match event {
                ManagerEvent::RxReconnect => {
                    Abandoning
                },
                ManagerEvent::RxHints => {
                    current_state
                },
                ManagerEvent::ConnectionLostFollower => {
                    Lonely
                },
                ManagerEvent::ConnectionLostLeader => {
                    Flushing
                },
                ManagerEvent::Stop => {
                    Stopped
                },
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Abandoning => match event {
                ManagerEvent::RxHints => {
                    current_state
                },
                ManagerEvent::ConnectionLostFollower => {
                    Connecting
                },
                ManagerEvent::Stop => {
                    Stopped
                },
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Flushing => match event {
                ManagerEvent::RxReconnecting => {
                    Connecting
                },
                ManagerEvent::Stop => {
                    Stopped
                },
                ManagerEvent::RxHints => {
                    current_state
                },
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Lonely => match event {
                ManagerEvent::RxReconnect => {
                    Connecting
                },
                ManagerEvent::Stop => {
                    Stopped
                },
                ManagerEvent::RxHints => {
                    current_state
                },
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Stopping => match event {
                ManagerEvent::RxHints => {
                    current_state
                },
                ManagerEvent::ConnectionLostFollower => {
                    Stopped
                },
                ManagerEvent::ConnectionLostLeader => {
                    Stopped
                },
                _ => {
                    panic!{"unexpected event {:?} for state {:?}", current_state, event }
                }
            },
            Stopped => current_state
        };
        self.state = Some(new_state);

        actions
    }

    fn get_current_state(&self) -> Option<State> {
        self.state
    }
}

#[test]
fn test_manager_machine() {
    let mut manager_fsm = ManagerMachine::new();

    // generate an input Event and see if we get the desired state and output Actions
    assert_eq!(manager_fsm.get_current_state(), Some(State::Waiting));

    let actions = manager_fsm.process(ManagerEvent::Start);
    assert_eq!(manager_fsm.get_current_state(), Some(State::Wanting));
}
