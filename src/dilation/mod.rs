use std::{cell::RefCell, rc::Rc};

use async_channel::{Receiver, RecvError, SendError};
#[cfg(test)]
use mockall::{automock, predicate::*};
use mockall_double::double;
use serde_derive::{Deserialize, Serialize};

#[cfg(test)]
use crate::core::protocol::MockWormholeProtocol;
#[mockall_double::double]
use crate::dilation::manager::ManagerMachine;
use crate::{
    core::{MySide, Phase, APPID_RAW},
    dilation::{
        api::{ManagerCommand, ProtocolCommand},
        connector::ConnectorMachine,
        events::ManagerEvent,
    },
    Wormhole, WormholeError,
};

mod api;
mod connection;
mod connector;
mod events;
mod manager;

#[mockall_double::double]
pub(crate) type WormholeConnection = WormholeConnectionDefault;

pub struct WormholeConnectionDefault {
    wormhole: Rc<RefCell<Wormhole>>,
}

#[cfg_attr(test, mockall::automock)]
impl WormholeConnectionDefault {
    pub(crate) fn new(wormhole: Wormhole) -> Self {
        Self {
            wormhole: Rc::new(RefCell::new(wormhole)),
        }
    }

    async fn receive_json<T>(&self) -> Result<T, WormholeError>
    where
        T: for<'a> serde::Deserialize<'a> + 'static,
    {
        let message = self.wormhole.borrow_mut().receive_json().await;
        match message {
            Ok(Ok(result)) => {
                log::debug!("received JSON");
                Ok(result)
            },
            Ok(Err(error)) => Err(WormholeError::ProtocolJson(error)),
            Err(error) => Err(error),
        }
    }

    async fn send_json(&self, command: &ProtocolCommand) -> Result<(), WormholeError> {
        log::debug!("send JSON {:?} in dilation phase", command);
        self.wormhole
            .borrow_mut()
            .send_json_with_phase(command, Phase::dilation)
            .await
    }
}

pub struct CommEndpoint<S, R> {
    sender: async_channel::Sender<S>,
    receiver: async_channel::Receiver<R>,
}

impl<S, R> CommEndpoint<S, R> {
    pub fn create_pair() -> (Self, CommEndpoint<R, S>) {
        // TODO finally we might need to use a queue size of 1 to achieve a backpressure mechanism. But we need to take care of avoiding deadlocks here.
        let (forth_sender, forth_receiver) = async_channel::bounded(10);
        let (back_sender, back_receiver) = async_channel::bounded(10);

        (
            CommEndpoint {
                sender: forth_sender,
                receiver: back_receiver,
            },
            CommEndpoint {
                sender: back_sender,
                receiver: forth_receiver,
            },
        )
    }

    pub async fn send(&self, msg: S) -> Result<(), SendError<S>> {
        self.sender.send(msg).await
    }

    pub async fn receive(&self) -> Result<R, RecvError> {
        self.receiver.recv().await
    }
}

pub(crate) struct MailboxCommunication {
    incoming_sender: async_channel::Sender<ManagerEvent>,
    outgoing_receiver: async_channel::Receiver<ProtocolCommand>,
}

pub struct MailboxClient {
    incoming_receiver: async_channel::Receiver<ManagerEvent>,
    outgoing_sender: async_channel::Sender<ProtocolCommand>,
}

enum MailboxCommunicationAction {
    Continue,
    Stop,
}

pub struct DilatedWormhole {
    wormhole: WormholeConnection,
    event_receiver: async_channel::Receiver<ManagerEvent>,
    side: MySide,
    manager: ManagerMachine,
}

impl DilatedWormhole {
    pub fn new(wormhole: Wormhole, side: MySide) -> Self {
        let (event_sender, event_receiver) = async_channel::bounded(10);

        DilatedWormhole {
            wormhole: WormholeConnection::new(wormhole),
            event_receiver,
            side: side.clone(),
            manager: ManagerMachine::new(side.clone(), event_sender),
        }
    }

    pub async fn run(&mut self) {
        log::info!(
            "start state machine: state={}",
            &self.manager.current_state().unwrap()
        );

        use futures::{FutureExt, StreamExt};

        let mut command_handler = |cmd| Self::execute_command(&self.wormhole, cmd);

        loop {
            log::debug!("wait for next event");

            let t1 = self.wormhole.receive_json().fuse();
            let t2 = self.event_receiver.next().fuse();

            futures::pin_mut!(t1, t2);

            let manager_event_result = futures::select! {
                value = t1 => value,
                value = t2 => Ok(value.unwrap()),
            };

            match manager_event_result {
                Ok(manager_event) => {
                    log::debug!("received ManagerEvent: {:?}", manager_event);
                    self.manager
                        .process(manager_event, &self.side, &mut command_handler);

                    if self.manager.is_done() {
                        log::debug!("exiting");
                        break;
                    }
                },

                Err(error) => {
                    log::warn!("queue read error: {:?}", error)
                },
            }
        }
    }

    fn handle_error(value: Result<ManagerEvent, WormholeError>) -> ManagerEvent {
        match value {
            Ok(event) => {
                return event;
            },
            Err(error) => {
                panic!("handle error: {:?}", error);
            },
        }
    }

    fn execute_command(
        wormhole: &WormholeConnection,
        command: ManagerCommand,
    ) -> Result<(), WormholeError> {
        log::debug!("execute_command");
        match command {
            ManagerCommand::Protocol(protocol_command) => {
                log::debug!("       command: {}", protocol_command);
                futures::executor::block_on(wormhole.send_json(&protocol_command))
            },
            ManagerCommand::IO(io_command) => {
                println!("io command: {}", io_command);
                Ok(())
            },
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::{Arc, Mutex};

    use crate::{
        core::test::init_logger,
        dilation::{api::ProtocolCommand, events::ManagerEvent, manager::State},
    };

    use super::*;

    use mockall::predicate::{always, eq};

    #[async_std::test]
    async fn test_wormhole_connection_send() {
        let mut protocol = MockWormholeProtocol::default();
        let command = ProtocolCommand::SendPlease {
            side: MySide::generate(2),
        };

        let serialized_bytes = serde_json::to_vec(&command).unwrap();

        protocol
            .expect_send_with_phase()
            .withf(move |bytes, provider| {
                bytes == &serialized_bytes && provider(0) == Phase::dilation(0)
            })
            .return_once(|_, _| Ok(()));

        let connection = WormholeConnectionDefault::new(Wormhole::new(Box::new(protocol)));

        let result = connection.send_json(&command).await;

        assert!(result.is_ok())
    }

    #[async_std::test]
    async fn test_wormhole_connection_send_error() {
        let mut protocol = MockWormholeProtocol::default();
        let command = ProtocolCommand::SendPlease {
            side: MySide::generate(2),
        };

        protocol
            .expect_send_with_phase()
            .return_once(|_, _| Err(WormholeError::Protocol(Box::from("foo"))));

        let connection = WormholeConnectionDefault::new(Wormhole::new(Box::new(protocol)));

        let result = connection.send_json(&command).await;

        assert!(result.is_err())
    }

    #[async_std::test]
    async fn test_wormhole_connection_receive() {
        let mut protocol = MockWormholeProtocol::default();

        let serialized_bytes = r#"{"type": "start"}"#.as_bytes().to_vec();

        protocol
            .expect_receive()
            .return_once(|| Ok(serialized_bytes));

        let connection = WormholeConnectionDefault::new(Wormhole::new(Box::new(protocol)));

        let result = connection.receive_json::<ManagerEvent>().await;

        assert!(result.is_ok())
    }

    #[async_std::test]
    async fn test_wormhole_connection_receive_error() {
        let mut protocol = MockWormholeProtocol::default();

        protocol
            .expect_receive()
            .return_once(|| Err(WormholeError::Protocol(Box::from("foo"))));

        let connection = WormholeConnectionDefault::new(Wormhole::new(Box::new(protocol)));

        let result = connection.receive_json::<ManagerEvent>().await;

        assert!(result.is_err())
    }

    #[async_std::test]
    async fn test_wormhole_connection_receive_deserialization_error() {
        let mut protocol = MockWormholeProtocol::default();

        let serialized_bytes = r#"{"type": "foo"}"#.as_bytes().to_vec();

        protocol
            .expect_receive()
            .return_once(|| Ok(serialized_bytes));

        let connection = WormholeConnectionDefault::new(Wormhole::new(Box::new(protocol)));

        let result = connection.receive_json::<ManagerEvent>().await;

        assert!(result.is_err())
    }

    #[async_std::test]
    async fn test_dilated_wormhole_new() {
        use crate::dilation::manager::MockManagerMachine;

        let mut protocol = MockWormholeProtocol::default();
        let mut wormhole = Wormhole::new(Box::new(protocol));

        let wc_ctx = MockWormholeConnectionDefault::new_context();
        wc_ctx
            .expect()
            .with(always())
            .return_once(move |_| WormholeConnection::default());

        let mm_ctx = MockManagerMachine::new_context();
        mm_ctx
            .expect()
            .with(always(), always())
            .return_once(move |_, _| ManagerMachine::default());
    }

    #[async_std::test]
    async fn test_dilated_wormhole() {
        init_logger();

        let mut manager = ManagerMachine::default();
        let mut wormhole = WormholeConnection::default();

        let my_side = MySide::generate(23);

        manager
            .expect_current_state()
            .return_once(|| Some(State::Wanting));

        wormhole
            .expect_receive_json()
            .return_once(|| Ok(ManagerEvent::Start));

        manager
            .expect_process()
            .with(eq(ManagerEvent::Start), eq(my_side.clone()), always())
            .times(1)
            .return_once(|_, _, _| ());

        manager.expect_is_done().return_once(|| true);

        let (event_sender, event_receiver) = async_channel::unbounded();
        let (mailbox_communication, mailbox_client) =
            CommEndpoint::<ManagerEvent, ProtocolCommand>::create_pair();

        let mut dilated_wormhole = DilatedWormhole {
            wormhole,
            event_receiver,
            side: my_side,
            manager,
        };

        mailbox_communication
            .send(ManagerEvent::Start)
            .await
            .unwrap();

        dilated_wormhole.run().await;
    }

    #[async_std::test]
    async fn test_dilated_wormhole_two_iterations(
    ) -> eyre::Result<(), async_channel::SendError<ManagerEvent>> {
        init_logger();

        let mut manager = ManagerMachine::default();
        let mut wormhole = WormholeConnection::default();

        let my_side = MySide::generate(23);

        manager
            .expect_current_state()
            .return_once(|| Some(State::Wanting));

        let mut events = vec![Ok(ManagerEvent::Stop), Ok(ManagerEvent::Start)];
        wormhole
            .expect_receive_json()
            .times(2)
            .returning(move || events.pop().unwrap());

        let verify_events = Arc::new(Mutex::new(vec![ManagerEvent::Stop, ManagerEvent::Start]));
        let verify_my_side = my_side.clone();
        manager
            .expect_process()
            .withf(move |event, side, _| {
                *event == verify_events.lock().unwrap().pop().unwrap() && side == &verify_my_side
            })
            .times(2)
            .returning(|_, _, _| ());

        let mut returns = vec![true, false];
        manager
            .expect_is_done()
            .returning(move || returns.pop().unwrap());

        let (event_sender, event_receiver) = async_channel::unbounded();

        let mut dilated_wormhole = DilatedWormhole {
            wormhole,
            event_receiver,
            side: my_side.clone(),
            manager,
        };

        dilated_wormhole.run().await;
        Ok(())
    }
}
