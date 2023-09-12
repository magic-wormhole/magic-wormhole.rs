use async_channel::Sender;
use derive_more::Display;
#[cfg(test)]
use mockall::automock;

use crate::{
    core::{MySide, TheirSide},
    dilation::{
        api::{IOEvent, ManagerCommand},
        connector::{spawn_connector, ConnectorMachine},
        events::ConnectorEvent,
    },
    transit::Hints,
    WormholeError,
};

use super::{api::ProtocolCommand, events::ManagerEvent};

#[derive(Debug, PartialEq, Display, Copy, Clone)]
pub enum Role {
    Leader,
    Follower,
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

pub struct ManagerMachine {
    pub side: MySide,
    pub state: Option<State>,
    pub event_sender: async_channel::Sender<ManagerEvent>,
    connection_event_sender: async_channel::Sender<ConnectorEvent>,
}

#[cfg_attr(test, automock)]
impl ManagerMachine {
    pub fn new(side: MySide, event_sender: Sender<ManagerEvent>) -> Self {
        let (connection_event_sender, connection_event_receiver) = async_channel::unbounded();

        let connector = ConnectorMachine::new(
            side.clone(),
            event_sender.clone(),
            connection_event_sender.clone(),
        );

        spawn_connector(connector, connection_event_receiver);

        ManagerMachine {
            side,
            state: Some(State::Wanting),
            event_sender,
            connection_event_sender,
        }
    }

    pub fn current_state(&self) -> Option<State> {
        self.state
    }

    pub fn is_waiting(&self) -> bool {
        self.state == Some(State::Waiting)
    }

    pub fn process_io(&mut self, event: IOEvent) -> Vec<ManagerCommand> {
        // XXX: which Manager states process IO events?
        let actions = Vec::<ManagerCommand>::new();

        // XXX: big match expression here

        actions
    }

    pub fn get_current_state(&self) -> Option<State> {
        self.state
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
        let mut command: Option<ManagerCommand> = None;
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
                    let _ = self.send_connector_event(ConnectorEvent::GotTheirSide { their_side });

                    command = Some(ManagerCommand::from(ProtocolCommand::SendPlease {
                        side: side.clone(),
                    }));

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
                    self.send_connector_event(ConnectorEvent::GotHints { hints });

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
            Some(manager_command) => command_handler(manager_command),
            None => Ok(()),
        };

        match command_result {
            Ok(result) => {
                log::debug!("update state {:?} -> {:?}", self.state.unwrap(), new_state);
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

    fn send_connector_event(&self, event: ConnectorEvent) {
        let _ = futures::executor::block_on(self.connection_event_sender.send(event));
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state == Option::from(State::Stopped)
    }
}

#[cfg(test)]
mod test {
    use crate::{
        core::{MySide, TheirSide},
        transit::{DirectHint, RelayHint},
    };

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

        fn reset(&mut self) {
            self.command = None;
        }
    }

    #[test]
    fn test_manager_machine() {
        let (event_sender, event_receiver) = async_channel::bounded(10);
        // Sends Start event during construction:
        let mut manager_fsm = ManagerMachine::new(
            MySide::unchecked_from_string("test123".to_string()),
            event_sender,
        );
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
                side: side.clone(),
            }))
        );

        let direct_hint = DirectHint::new("host", 123);
        let relay_hint: RelayHint;
        let hints = Hints::new(vec![direct_hint], vec![]);

        handler.reset();
        // generate an input Event and see if we get the desired state and output Actions
        manager_fsm.process(ManagerEvent::RxHints { hints }, &side, &mut |cmd| {
            handler.handle_command(cmd)
        });

        assert_eq!(manager_fsm.get_current_state(), Some(State::Connecting));
        assert_eq!(handler.command, None);
        //assert_eq!(manager_fsm.their_hints.direct_tcp.len(), 1);
        //assert_eq!(manager_fsm.their_hints.relay.len(), 0);
    }
}
