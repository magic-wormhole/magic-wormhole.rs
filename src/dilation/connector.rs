use std::collections::HashMap;

use async_std::task::JoinHandle;
use derive_more::Display;

use crate::{
    core::{MySide, TheirSide},
    dilation::{
        connection::{spawn_connection, ConnectionMachine, ConnectionRef},
        events::{ManagerEvent, ManagerEvent::Stop},
        manager::Role,
    },
    transit::Hints,
};

use super::{events::ConnectorEvent, manager};

#[derive(Copy, Clone, Debug, Display, PartialEq)]
pub enum State {
    Waiting,
    Connecting,
    Connected,
    Stopped,
}

pub struct ConnectorMachine {
    side: MySide,
    state: State,
    manager_event_sender: async_channel::Sender<ManagerEvent>,
    connector_event_sender: async_channel::Sender<ConnectorEvent>,
    role: Role,
    connections: HashMap<ConnectionId, ConnectionRef>,
    connection_counter: std::sync::atomic::AtomicUsize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Deserialize, derive_more::Display)]
#[display(fmt = "Connection_{}", "&*_0")]
pub struct ConnectionId(usize);

impl ConnectorMachine {
    pub fn new(
        side: MySide,
        manager_event_sender: async_channel::Sender<ManagerEvent>,
        connector_event_sender: async_channel::Sender<ConnectorEvent>,
    ) -> Self {
        ConnectorMachine {
            side,
            state: State::Waiting,
            manager_event_sender,
            connector_event_sender,
            role: Role::Follower,
            connections: HashMap::new(),
            connection_counter: std::sync::atomic::AtomicUsize::new(1),
        }
    }

    async fn process(&mut self, event: ConnectorEvent) {
        log::debug!("process {:?}", event);
        use State::*;
        let current_state = self.state.clone();
        self.state = match current_state {
            Waiting => match event {
                ConnectorEvent::GotTheirSide { their_side } => {
                    self.role = self.choose_role(&their_side);
                    log::debug!(
                        "set role: {}",
                        if self.role == Role::Leader {
                            "leader"
                        } else {
                            "follower"
                        }
                    );

                    // Both sides (leader and follower) now open listener sockets (like in transit::init()) and exchange hints.

                    // This seems to use the ListenerReady event when set up successfully

                    Connecting
                },
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connecting => match event {
                ConnectorEvent::GotHints { hints } => {
                    // try to connect via the hint which has been received
                    let connection_id = ConnectionId(
                        self.connection_counter
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    );
                    let result = ConnectionMachine::connect(
                        connection_id,
                        hints,
                        self.connector_event_sender.clone(),
                        self.role.clone(),
                    )
                    .await;

                    if let Ok((connection_ref, connection, connection_machine)) = result {
                        spawn_connection(connection, connection_machine);
                        self.connections.insert(connection_id, connection_ref);
                    }

                    current_state
                },
                ConnectorEvent::AddCandidate => {
                    // The leader then decides which connection to select

                    current_state
                },

                ConnectorEvent::ListenerReady { hints } => current_state,
                ConnectorEvent::Accept => Connected,
                ConnectorEvent::Stop => Stopped,
                ConnectorEvent::Stopped { connection_id } => {
                    if self.connections.contains_key(&connection_id) {
                        self.connections.remove(&connection_id);
                    }
                    if self.connections.len() == 0 {
                        log::debug!("last connection closed -> stopping");
                        Stopped
                    } else {
                        current_state
                    }
                },

                // Should'n there be an event which will be sent when a connection has been set up successfully or the attempt has failed?
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Connected => match event {
                // If a connection attempt has been successful the Leader will select if that connection should be used.

                // All other connection attempts can be stopped.
                ConnectorEvent::Accept => current_state,
                ConnectorEvent::ListenerReady { hints } => current_state,
                ConnectorEvent::GotHints { hints } => current_state,
                ConnectorEvent::AddCandidate => current_state,
                ConnectorEvent::Stop => Stopped,
                _ => {
                    panic! {"unexpected event {:?} for state {:?}", current_state, event}
                },
            },
            Stopped => current_state,
        };
    }

    pub fn send(
        &self,
        manager_event: ManagerEvent,
    ) -> Result<(), async_channel::SendError<ManagerEvent>> {
        futures::executor::block_on(self.manager_event_sender.send(manager_event))
    }

    pub fn is_stopped(&self) -> bool {
        return self.state == State::Stopped;
    }

    fn choose_role(&self, their_side: &TheirSide) -> Role {
        let myside: TheirSide = self.side.clone().into();
        if myside > *their_side {
            Role::Leader
        } else {
            Role::Follower
        }
    }
}

struct ConnectorContainer {
    connector: ConnectorMachine,
    event_receiver: async_channel::Receiver<ConnectorEvent>,
    task: Option<async_std::task::JoinHandle<()>>,
}

pub fn spawn_connector(
    mut connector: ConnectorMachine,
    event_receiver: async_channel::Receiver<ConnectorEvent>,
) -> JoinHandle<()> {
    log::debug!("starting connector task");
    let worker = async_std::task::spawn(async move {
        loop {
            log::debug!("wait for connector event");
            let result = event_receiver.recv().await;

            match result {
                Ok(connection_event) => connector.process(connection_event).await,
                Err(error) => {
                    log::warn!("receiver error: {:?}", error);
                    async_std::task::sleep(instant::Duration::from_secs(1)).await;
                },
            }

            if connector.is_stopped() {
                log::debug!("exiting connector");
                break;
            }
        }
        log::debug!("exiting connector event loop");
    });
    worker
}
