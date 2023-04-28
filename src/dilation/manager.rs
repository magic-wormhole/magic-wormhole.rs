use async_trait::async_trait;
use derive_more::Display;
#[cfg(test)]
use mockall::automock;

use crate::{core::TheirSide, dilation::api::ManagerCommand, WormholeError};

use super::{
    api::{IOEvent, ProtocolCommand},
    events::ManagerEvent,
};

pub struct ManagerMachine {
    pub state: Option<State>,
}

#[derive(Debug, PartialEq, Clone, Copy, Display)]
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

#[cfg_attr(test, automock)]
impl ManagerMachine {
    pub fn new() -> Self {
        let mut machine = ManagerMachine {
            state: Some(State::Wanting),
        };
        machine
    }

    pub fn is_waiting(&self) -> bool {
        self.state == Some(State::Waiting)
    }

    pub fn process_io(&mut self, event: IOEvent) -> Vec<ManagerCommand> {
        // XXX: which Manager states process IO events?
        let mut actions = Vec::<ManagerCommand>::new();

        // XXX: big match expression here

        actions
    }

    pub fn get_current_state(&self) -> Option<State> {
        self.state
    }

    pub fn process(
        &mut self,
        event: ManagerEvent,
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
                    command = Some(ManagerCommand::from(ProtocolCommand::SendPlease));
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
                ManagerEvent::RxPlease { side: theirSide } => {
                    command = Some(ManagerCommand::from(ProtocolCommand::SendPlease));
                    Connecting
                },
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connecting => match event {
                ManagerEvent::RxHints => current_state,
                ManagerEvent::Stop => Stopped,
                ManagerEvent::ConnectionMade => Connected,
                ManagerEvent::RxReconnect => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connected => match event {
                ManagerEvent::RxReconnect => Abandoning,
                ManagerEvent::RxHints => current_state,
                ManagerEvent::ConnectionLostFollower => Lonely,
                ManagerEvent::ConnectionLostLeader => Flushing,
                ManagerEvent::Stop => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Abandoning => match event {
                ManagerEvent::RxHints => current_state,
                ManagerEvent::ConnectionLostFollower => Connecting,
                ManagerEvent::Stop => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Flushing => match event {
                ManagerEvent::RxReconnecting => Connecting,
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Lonely => match event {
                ManagerEvent::RxReconnect => Connecting,
                ManagerEvent::Stop => Stopped,
                ManagerEvent::RxHints => current_state,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Stopping => match event {
                ManagerEvent::RxHints => current_state,
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
            Ok(result) => {
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
                log::warn!("processing event errored: {}", wormhole_error);
            },
        };
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state == Option::from(State::Stopped)
    }
}

#[cfg(test)]
mod test {
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
        let mut manager_fsm = ManagerMachine::new();

        // generate an input Event and see if we get the desired state and output Actions
        assert_eq!(manager_fsm.get_current_state(), Some(State::Wanting));

        let mut handler = TestHandler::new();

        manager_fsm.process(
            ManagerEvent::RxPlease {
                side: TheirSide::from("test"),
            },
            &mut |cmd| handler.handle_command(cmd),
        );

        assert_eq!(manager_fsm.get_current_state(), Some(State::Connecting));
        assert_eq!(
            handler.command,
            Some(ManagerCommand::Protocol(ProtocolCommand::SendPlease))
        )
    }
}
