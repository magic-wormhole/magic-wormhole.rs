use std::{
    borrow::Cow,
    cell::{RefCell, RefMut},
    rc::Rc,
};

use async_trait::async_trait;
use futures::executor;
#[cfg(test)]
use mockall::{automock, mock, predicate::*};
use mockall_double::double;
use serde_derive::{Deserialize, Serialize};

use crate::{
    core::{MySide, Phase},
    dilation::api::{ManagerCommand, ProtocolCommand},
    Wormhole, WormholeError,
};

#[double]
use crate::dilation::manager::ManagerMachine;

use super::AppID;

mod api;
mod events;
mod manager;

const APPID_RAW: &str = "lothar.com/wormhole/text-or-file-xfer";

// XXX define an dilation::APP_CONFIG
pub const APP_CONFIG_SEND: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(Cow::Borrowed(APPID_RAW)),
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion::new(FileTransferV2Mode::Send),
    with_dilation: false,
};

pub const APP_CONFIG_RECEIVE: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(Cow::Borrowed(APPID_RAW)),
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion::new(FileTransferV2Mode::Receive),
    with_dilation: false,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[serde(rename = "transfer")]
enum FileTransferV2Mode {
    Send,
    Receive,
    Connect,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct DilatedTransfer {
    mode: FileTransferV2Mode,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AppVersion {
    // #[serde(default)]
    // abilities: Cow<'static, [Cow<'static, str>]>,
    // #[serde(default)]
    // transfer_v2: Option<AppVersionTransferV2Hint>,

    // XXX: we don't want to send "can-dilate" key for non-dilated
    // wormhole, would making this an Option help? i.e. when the value
    // is a None, we don't serialize that into the json and do it only
    // when it is a "Some" value?
    // overall versions payload is of the form:
    // b'{"can-dilate": ["1"], "dilation-abilities": [{"type": "direct-tcp-v1"}, {"type": "relay-v1"}], "app_versions": {"transfer": {"mode": "send", "features": {}}}}'

    //can_dilate: Option<[Cow<'static, str>; 1]>,
    //dilation_abilities: Cow<'static, [Ability; 2]>,
    #[serde(rename = "transfer")]
    app_versions: DilatedTransfer,
}

impl AppVersion {
    const fn new(mode: FileTransferV2Mode) -> Self {
        // let can_dilate: Option<[Cow<'static, str>; 1]> = if enable_dilation {
        //     Some([std::borrow::Cow::Borrowed("1")])
        // } else {
        //     None
        // };

        Self {
            // abilities: Cow::Borrowed([Cow::Borrowed("transfer-v1"), Cow::Borrowed("transfer-v2")]),
            // transfer_v2: Some(AppVersionTransferV2Hint::new())
            // can_dilate: can_dilate,
            // dilation_abilities: std::borrow::Cow::Borrowed(&[
            //     Ability{ ty: std::borrow::Cow::Borrowed("direct-tcp-v1") },
            //     Ability{ ty: std::borrow::Cow::Borrowed("relay-v1") },
            // ]),
            app_versions: DilatedTransfer { mode },
        }
    }
}

impl Default for AppVersion {
    fn default() -> Self {
        Self::new(FileTransferV2Mode::Send)
    }
}

#[double]
type WormholeConnection = WormholeConnectionDefault;

pub struct WormholeConnectionDefault {
    wormhole: Rc<RefCell<Wormhole>>,
}

#[cfg_attr(test, automock)]
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
        message
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
        rendezvous::RendezvousError,
    };
    use std::sync::{Arc, Mutex};

    use super::*;

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

        let mut verify_events = Arc::new(Mutex::new(vec![ManagerEvent::Stop, ManagerEvent::Start]));
        let verify_my_side = my_side.clone();
        manager
            .expect_process()
            .withf(move |event, side, handler| {
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
