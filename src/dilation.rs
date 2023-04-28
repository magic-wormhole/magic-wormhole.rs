use std::{
    borrow::Cow,
    cell::{RefCell, RefMut},
    ops::Deref,
    rc::Rc,
};

use async_trait::async_trait;
use futures::executor;
#[cfg(test)]
use mockall::{automock, mock, predicate::*};
use serde_derive::{Deserialize, Serialize};

use crate::{
    core::MySide,
    dilation::{api::ManagerCommand, manager::ManagerMachine},
    Wormhole, WormholeError,
};

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
};

pub const APP_CONFIG_RECEIVE: crate::AppConfig<AppVersion> = crate::AppConfig::<AppVersion> {
    id: AppID(Cow::Borrowed(APPID_RAW)),
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: AppVersion::new(FileTransferV2Mode::Receive),
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

pub struct DilatedWormhole {
    wormhole: Rc<RefCell<Wormhole>>,
    side: MySide,
    manager: ManagerMachine,
}

impl DilatedWormhole {
    pub fn new(wormhole: Wormhole, side: MySide) -> Self {
        let wormhole_ref = &wormhole;
        DilatedWormhole {
            wormhole: Rc::new(RefCell::new(wormhole)),
            side,
            manager: ManagerMachine::new(),
        }
    }

    pub async fn run(&mut self) {
        log::debug!(
            "start state machine: state={}",
            &self.manager.state.unwrap()
        );

        let mut command_handler = |cmd| Self::execute_command(self.wormhole.borrow_mut(), cmd);

        loop {
            let event_result = self.wormhole.borrow_mut().receive_json().await;

            match event_result {
                Ok(manager_event) => self.manager.process(manager_event, &mut command_handler),
                Err(error) => {
                    log::warn!("received error {}", error);
                },
            };

            if self.manager.is_done() {
                break;
            }
        }
    }

    fn execute_command(
        mut wormhole: RefMut<Wormhole>,
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
    use crate::dilation::{
        api::ProtocolCommand, events::ManagerEvent, manager::MockManagerMachine,
    };

    use super::*;

    #[test]
    fn test_dilated_wormhole() {
        let mut manager = MockManagerMachine::default();
        // let mut wormhole = MockWormhole::default();
        //
        // let dilated_wormhole = DilatedWormhole { manager, side: MySide::generate(23), wormhole };
    }
}
