use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_derive::{Deserialize, Serialize};

use crate::dilation::events::{ConnectionChannels, Event, ManagerChannels, Events};

use super::AppID;

mod manager;
pub mod events;
mod api;


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
            app_versions: DilatedTransfer {
                mode,
            }
        }
    }
}

impl Default for AppVersion {
    fn default() -> Self {
        Self::new(FileTransferV2Mode::Send)
    }
}

type Queue<A> = Arc<Mutex<VecDeque<A>>>;

pub struct DilationCore {
    manager: manager::ManagerMachine,
    connection_channels: ConnectionChannels,
    manager_channels: ManagerChannels,
}

impl DilationCore {
    pub fn new(connection_channels: ConnectionChannels, manager_channels: ManagerChannels) -> DilationCore {
        // XXX: generate side
        DilationCore {
            manager: manager::ManagerMachine::new(),
            connection_channels,
            manager_channels,
        }
    }

    pub fn do_io(&mut self, event: api::IOEvent) -> Vec<api::Action> {
        let events: events::Events = self.manager.process_io(event);

        self.execute(events)
    }

    fn run(&mut self) {
        while let event_result = self.manager_channels.inbound.recv() {
            let actions = match event_result {
                Ok(Event::Manager(manager_event)) =>
                    self.manager.process(manager_event),
                Ok(Event::IO(io_action)) => {
                    println!("manager received IOAction {}", io_action);
                    // XXX to be filled in later
                    Events::new()
                },
                Err(error) => {
                    eprintln!("received error {}", error);
                    Events::new()
                }
            };
            // XXX do something with "actions", some of them could
            // be input to other state machines, some of them could
            // be IO actions.
        }
    }

    fn execute(&mut self, events: events::Events) -> Vec<api::Action> {
        // process until all the events are consumed and produce a
        // bunch of Actions.
        let mut action_queue: Vec<api::Action> = Vec::new();
        let mut event_queue: VecDeque<events::Event> = VecDeque::new();

        event_queue.append(&mut VecDeque::from(events.events));

        while let Some(event) = event_queue.pop_front() {
            let actions: events::Events = match event {
                events::Event::IO(_) => todo!(),
                events::Event::Manager(e) => self.manager.process(e),
            };
            for a in actions.events {
                event_queue.push_back(a)
            }
        }

        action_queue
    }
}
