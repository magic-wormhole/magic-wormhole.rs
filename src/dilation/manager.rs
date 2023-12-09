use derive_more::Display;
#[cfg(test)]
use mockall::automock;

use crate::{
    core::{MySide, TheirSide},
    dilation::api::ManagerCommand,
    WormholeError,
};

use super::{api::ProtocolCommand, events::ManagerEvent};

#[derive(Debug, PartialEq, Display)]
pub enum Role {
    Leader,
    Follower,
}

#[derive(Debug, PartialEq, Clone, Copy, Display)]
#[allow(dead_code)]
pub enum State {
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

pub struct ManagerMachine {
    pub side: MySide,
    pub role: Role,
    pub state: Option<State>,
}

#[cfg_attr(test, automock)]
impl ManagerMachine {
    pub fn new(side: MySide) -> Self {
        ManagerMachine {
            side,
            role: Role::Follower,
            state: Some(State::Wanting),
        }
    }

    pub fn current_state(&self) -> Option<State> {
        self.state
    }

    fn choose_role(&self, theirside: &TheirSide) -> Role {
        let myside: TheirSide = self.side.clone().into();
        if myside > *theirside {
            Role::Leader
        } else {
            Role::Follower
        }
    }

    pub fn process(
        &mut self,
        event: ManagerEvent,
        side: &MySide,
        command_handler: &mut dyn FnMut(ManagerCommand) -> Result<(), WormholeError>,
    ) {
        log::debug!(
            "processing event: state={}, event={}",
            self.state.unwrap(),
            &event
        );
        // given the event and the current state, generate output
        // event and move to the new state
        use State::*;
        let mut command = None;
        let current_state = self.state.unwrap();
        let new_state = match current_state {
            Waiting => match event {
                ManagerEvent::Start => {
                    command = Some(ManagerCommand::from(ProtocolCommand::SendPlease {
                        side: side.clone(),
                    }));
                    Wanting
                },
                ManagerEvent::Stop => {
                    // actions.addAction(NotifyStopped)
                    Stopped
                },
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Wanting => match event {
                ManagerEvent::RxPlease { side: their_side } => {
                    command = Some(ManagerCommand::from(ProtocolCommand::SendPlease {
                        side: side.clone(),
                    }));
                    let role = self.choose_role(&their_side.clone());
                    log::debug!(
                        "role: {}",
                        if role == Role::Leader {
                            "leader"
                        } else {
                            "follower"
                        }
                    );
                    self.role = role;
                    Connecting
                },
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints { hints: _ } => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connecting => match event {
                ManagerEvent::RxHints { hints } => {
                    log::debug!("received connection hints: {:?}", hints);
                    // TODO store the other side's hints
                    current_state
                },
                ManagerEvent::Stop => Stopped,
                ManagerEvent::ConnectionMade => Connected,
                ManagerEvent::RxReconnect => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connected => match event {
                ManagerEvent::RxReconnect => Abandoning,
                ManagerEvent::RxHints { hints: _ } => current_state,
                ManagerEvent::ConnectionLostFollower => Lonely,
                ManagerEvent::ConnectionLostLeader => Flushing,
                ManagerEvent::Stop => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Abandoning => match event {
                ManagerEvent::RxHints { hints: _ } => current_state,
                ManagerEvent::ConnectionLostFollower => Connecting,
                ManagerEvent::Stop => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Flushing => match event {
                ManagerEvent::RxReconnecting => Connecting,
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints { hints: _ } => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Lonely => match event {
                ManagerEvent::RxReconnect => Connecting,
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints { hints: _ } => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Stopping => match event {
                ManagerEvent::RxHints { hints: _ } => current_state,
                ManagerEvent::ConnectionLostFollower => Stopped,
                ManagerEvent::ConnectionLostLeader => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Stopped => current_state,
        };

        let command_result = match command.clone() {
            Some(command) => command_handler(command),
            None => Ok(()),
        };

        match command_result {
            Ok(_result) => {
                self.state = Some(new_state);
                log::debug!(
                    "processing event finished: state={}, command={}",
                    self.state.unwrap(),
                    command
                        .clone()
                        .map(|cmd| cmd.to_string())
                        .unwrap_or("n/a".to_string())
                );
            },
            Err(wormhole_error) => {
                panic!("processing event errored: {}", wormhole_error);
            },
        };
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state == Option::from(State::Stopped)
    }
}

#[cfg(test)]
mod test {
    use crate::core::{MySide, TheirSide};

    use super::*;

    struct TestHandler {
        command: Option<ManagerCommand>,
    }

    impl TestHandler {
        fn new() -> Self {
            TestHandler { command: None }
        }

        fn handle_command(&mut self, command: ManagerCommand) -> Result<(), WormholeError> {
            self.command = Some(command);
            Ok(())
        }
    }

    #[test]
    fn test_manager_machine() {
        // Sends Start event during construction:
        let mut manager_fsm =
            ManagerMachine::new(MySide::unchecked_from_string("test123".to_string()));
        let side = MySide::generate(8);

        assert_eq!(manager_fsm.current_state(), Some(State::Wanting));
        assert_eq!(manager_fsm.is_done(), false);

        let mut handler = TestHandler::new();

        // generate an input Event and see if we get the desired state and output Actions
        manager_fsm.process(
            ManagerEvent::RxPlease {
                side: TheirSide::from("test"),
            },
            &side,
            &mut |cmd| handler.handle_command(cmd),
        );

        assert_eq!(manager_fsm.current_state(), Some(State::Connecting));
        assert_eq!(
            handler.command,
            Some(ManagerCommand::Protocol(ProtocolCommand::SendPlease {
                side: side,
            }))
        )
    }

    #[test]
    #[should_panic(expected = "Protocol error: foo")]
    fn test_manager_machine_handle_error() {
        let side = MySide::generate(8);
        let mut manager_fsm = ManagerMachine {
            side: side.clone(),
            role: Role::Follower,
            state: Some(State::Waiting),
        };

        assert_eq!(manager_fsm.current_state(), Some(State::Waiting));

        manager_fsm.process(ManagerEvent::Start, &side, &mut |_cmd| {
            Err(WormholeError::Protocol("foo".into()))
        });
    }
}
