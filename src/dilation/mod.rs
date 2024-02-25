use std::{cell::RefCell, rc::Rc};

use futures::executor;

use crate::{
    core::{MySide, Phase},
    dilation::api::{ManagerCommand, ProtocolCommand},
    Wormhole, WormholeError,
};

#[cfg(test)]
use crate::core::protocol::MockWormholeProtocol;

#[mockall_double::double]
use crate::dilation::manager::ManagerMachine;

mod api;
mod events;
mod manager;

#[mockall_double::double]
type WormholeConnection = WormholeConnectionDefault;

pub struct WormholeConnectionDefault {
    wormhole: Rc<RefCell<Wormhole>>,
}

#[cfg_attr(test, mockall::automock)]
impl WormholeConnectionDefault {
    fn new(wormhole: Wormhole) -> Self {
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
            Ok(result) => match result {
                Ok(result) => Ok(result),
                Err(error) => Err(WormholeError::ProtocolJson(error)),
            },
            Err(error) => Err(error),
        }
    }

    async fn send_json(&self, command: &ProtocolCommand) -> Result<(), WormholeError> {
        self.wormhole
            .borrow_mut()
            .send_json_with_phase(command, Phase::dilation)
            .await
    }
}

pub struct DilatedWormhole {
    wormhole: WormholeConnection,
    side: MySide,
    manager: ManagerMachine,
}

impl DilatedWormhole {
    pub fn new(wormhole: Wormhole, side: MySide) -> Self {
        DilatedWormhole {
            wormhole: WormholeConnection::new(wormhole),
            side: side.clone(),
            manager: ManagerMachine::new(side.clone()),
        }
    }

    pub async fn run(&mut self) {
        log::info!(
            "start state machine: state={}",
            &self.manager.current_state().unwrap()
        );

        let mut command_handler = |cmd| Self::execute_command(&self.wormhole, cmd);

        loop {
            log::debug!("wait for next event");
            let event_result = self.wormhole.receive_json().await;

            match event_result {
                Ok(manager_event) => {
                    log::debug!("received event");
                    self.manager
                        .process(manager_event, &self.side, &mut command_handler)
                },
                Err(error) => {
                    log::warn!("received error {}", error);
                    continue;
                },
            };

            if self.manager.is_done() {
                log::debug!("exiting");
                break;
            }
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
                executor::block_on(wormhole.send_json(&protocol_command))
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
    use crate::{
        core::test::init_logger,
        dilation::{
            api::{IOCommand, ProtocolCommand},
            events::ManagerEvent,
            manager::{MockManagerMachine, State},
        },
    };
    use std::sync::{Arc, Mutex};

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
        let wc_ctx = MockWormholeConnectionDefault::new_context();
        wc_ctx
            .expect()
            .with(always())
            .return_once(move |_| WormholeConnection::default());

        let mm_ctx = MockManagerMachine::new_context();
        mm_ctx
            .expect()
            .with(always())
            .return_once(move |_| ManagerMachine::default());
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

        let mut dilated_wormhole = DilatedWormhole {
            manager,
            side: my_side,
            wormhole,
        };

        dilated_wormhole.run().await;
    }

    #[async_std::test]
    async fn test_dilated_wormhole_receving_error() {
        init_logger();

        let mut manager = ManagerMachine::default();
        let mut wormhole = WormholeConnection::default();

        let my_side = MySide::generate(23);

        manager
            .expect_current_state()
            .return_once(|| Some(State::Wanting));

        let mut events = vec![Ok(ManagerEvent::Start), Err(WormholeError::DilationVersion)];
        wormhole
            .expect_receive_json()
            .returning(move || events.pop().unwrap());

        manager
            .expect_process()
            .with(eq(ManagerEvent::Start), eq(my_side.clone()), always())
            .times(1)
            .return_once(|_, _, _| ());

        manager.expect_is_done().return_once(|| true);

        let mut dilated_wormhole = DilatedWormhole {
            manager,
            side: my_side,
            wormhole,
        };

        dilated_wormhole.run().await;
    }

    #[async_std::test]
    async fn test_dilated_wormhole_two_iterations() {
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

        let mut dilated_wormhole = DilatedWormhole {
            manager,
            side: my_side.clone(),
            wormhole,
        };

        dilated_wormhole.run().await;
    }

    #[test]
    fn test_dilated_wormhole_execute_protocol_command() {
        init_logger();

        let mut wormhole = WormholeConnection::default();

        let protocol_command = ProtocolCommand::SendPlease {
            side: MySide::generate(2),
        };

        wormhole
            .expect_send_json()
            .with(eq(protocol_command.clone()))
            .return_once(|_| Ok(()))
            .times(1);

        let result = DilatedWormhole::execute_command(
            &mut wormhole,
            ManagerCommand::Protocol(protocol_command),
        );

        assert!(result.is_ok())
    }

    #[test]
    fn test_dilated_wormhole_execute_protocol_command_failure() {
        init_logger();

        let mut wormhole = WormholeConnection::default();

        let protocol_command = ProtocolCommand::SendPlease {
            side: MySide::generate(2),
        };

        let protocol_command_ref = protocol_command.clone();
        wormhole
            .expect_send_json()
            .with(eq(protocol_command_ref))
            .return_once(|_| Err(WormholeError::Crypto))
            .times(1);

        let result = DilatedWormhole::execute_command(
            &mut wormhole,
            ManagerCommand::Protocol(protocol_command.clone()),
        );

        assert!(result.is_err())
    }

    #[test]
    fn test_dilated_wormhole_execute_io_command() {
        init_logger();

        let mut wormhole = WormholeConnection::default();

        wormhole.expect_send_json().times(0);

        let result = DilatedWormhole::execute_command(
            &mut wormhole,
            ManagerCommand::IO(IOCommand::CloseConnection),
        );

        assert!(result.is_ok())
    }
}
