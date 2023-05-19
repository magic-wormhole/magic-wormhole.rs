use std::net::SocketAddr;

use async_channel::RecvError;
use async_std::task::JoinHandle;
use futures::{AsyncReadExt, AsyncWriteExt};

use crate::{
    dilation::{
        connection::ConnectionError::ConnectFailed, connector::ConnectionId,
        events::ConnectorEvent, manager::Role,
    },
    transit,
    transit::DirectHint,
};

pub struct Connection {
    connection_id: ConnectionId,
    tcp_reader: futures::io::ReadHalf<async_std::net::TcpStream>,
    connection_event_receiver: async_channel::Receiver<ConnectionEvent>,
}

#[derive(Debug, PartialEq, Clone, Copy, derive_more::Display)]
enum ConnectionState {
    Waiting,
    Connected,
    Stopped,
}

pub struct ConnectionMachine {
    connection_id: ConnectionId,
    connector_event_sender: async_channel::Sender<ConnectorEvent>,
    tcp_writer: futures::io::WriteHalf<async_std::net::TcpStream>,
    hints: transit::Hints,
    state: ConnectionState,
    role: Role,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("Could not initiate connection")]
    NotFound,
    #[error("IO error")]
    ConnectFailed { err: std::io::Error },
}

impl ConnectionMachine {
    pub(crate) async fn connect(
        connection_id: ConnectionId,
        hints: transit::Hints,
        connector_event_sender: async_channel::Sender<ConnectorEvent>,
        role: Role,
    ) -> Result<(ConnectionRef, Connection, Self), ConnectionError> {
        let (connection_event_sender, connection_event_receiver) = async_channel::unbounded();

        let direct_hints = hints.clone().direct_tcp;
        let mut tcp_stream: Option<async_std::net::TcpStream> = None;

        if direct_hints.len() > 0 {
            log::debug!("direct hints #{}", direct_hints.len());
            let first_direct_hint = direct_hints.iter().next();
            if let Some(hint) = first_direct_hint {
                tcp_stream = Some(Self::connect_direct(hint).await?)
            }
        }

        let relay_hints = hints.clone().relay;
        if relay_hints.len() > 0 {
            log::debug!("relaty hints #{}", relay_hints.len());
        }

        if let Some(stream) = tcp_stream {
            let (tcp_reader, tcp_writer) = stream.split();

            Ok((
                ConnectionRef {
                    connection_id,
                    connection_event_sender,
                },
                Connection {
                    connection_id,
                    tcp_reader,
                    connection_event_receiver,
                },
                ConnectionMachine {
                    connection_id,
                    connector_event_sender,
                    tcp_writer,
                    hints,
                    state: ConnectionState::Waiting,
                    role,
                },
            ))
        } else {
            log::debug!("no connection found");
            Err(ConnectionError::NotFound)
        }
    }

    async fn connect_direct(
        hint: &DirectHint,
    ) -> Result<async_std::net::TcpStream, ConnectionError> {
        let dest_addr = SocketAddr::try_from(hint).expect("error converting destination address");
        async_std::net::TcpStream::connect(dest_addr)
            .await
            .map_err(|err| ConnectionError::ConnectFailed { err })
    }

    pub(crate) async fn process(&mut self, connection_event: ConnectionEvent) -> () {
        let current_state = self.state.clone();
        self.state = match current_state {
            ConnectionState::Waiting => match connection_event {
                ConnectionEvent::Closed => {
                    self.notify_connector_about_closed().await;
                    ConnectionState::Stopped
                },
                ConnectionEvent::DataReceived { data } => {
                    log::debug!(
                        "received data ({} bytes): {:?}",
                        data.len(),
                        std::str::from_utf8(&data).unwrap()
                    );

                    let response = if self.role == Role::Leader {
                        "Magic-Wormhole Dilation Handshake v1 Leader\n\n"
                    } else {
                        "Magic-Wormhole Dilation Handshake v1 Follower\n\n"
                    };
                    self.tcp_writer
                        .write(response.as_bytes())
                        .await
                        .expect("could not write to tcp");
                    log::debug!("sent response: {}", response);
                    ConnectionState::Connected
                },
            },
            ConnectionState::Connected => match connection_event {
                ConnectionEvent::Closed => {
                    self.notify_connector_about_closed();
                    ConnectionState::Stopped
                },
                ConnectionEvent::DataReceived { data } => {
                    log::debug!("received data ({} bytes): {:02X?}", data.len(), data);
                    current_state
                },
            },
            _ => current_state,
        };
    }

    async fn notify_connector_about_closed(&mut self) {
        let _ = self
            .connector_event_sender
            .send(ConnectorEvent::Stopped {
                connection_id: self.connection_id,
            })
            .await;
    }

    pub(crate) fn is_stopped(&self) -> bool {
        return self.state == ConnectionState::Stopped;
    }
}

pub enum ConnectionEvent {
    Closed,
    DataReceived { data: Vec<u8> },
}

pub(crate) struct ConnectionRef {
    connection_id: ConnectionId,
    connection_event_sender: async_channel::Sender<ConnectionEvent>,
}

pub fn spawn_connection(
    mut connection: Connection,
    mut connection_machine: ConnectionMachine,
) -> JoinHandle<()> {
    log::debug!(
        "start connection task #{:?} {:?}",
        connection.connection_id,
        connection_machine.hints
    );
    let mut connection_event_receiver = connection.connection_event_receiver.clone();
    let worker = async_std::task::spawn(async move {
        let mut buf = vec![0u8; 1024];

        loop {
            log::debug!("Wait for connection event #{:?}", connection.connection_id);
            use futures::{FutureExt, StreamExt};

            let t1 = connection_event_receiver.next().fuse();
            let t2 = connection.tcp_reader.read(&mut buf).fuse();

            futures::pin_mut!(t1, t2);

            let result = futures::select! {
                value = t1 => match value {
                    Some(event) => Ok(event),
                    None => Err(ConnectionError::NotFound)
                },
                value = t2 => match value {
                    Ok(n) => {
                        if n == 0 {
                            log::debug!("received empty result -> closing");
                            Ok(ConnectionEvent::Closed)
                        } else {
                            let mut data = vec![0u8; n];
                            data.clone_from_slice(&buf[0..n]);
                            Ok(ConnectionEvent::DataReceived { data})
                        }
                    },
                    Err(err) => Err(ConnectionError::ConnectFailed {err}),
                }
            };

            match result {
                Ok(connection_event) => connection_machine.process(connection_event).await,
                Err(error) => {
                    log::warn!("receiver error: {:?}", error);
                    async_std::task::sleep(instant::Duration::from_secs(1)).await;
                },
            }

            if connection_machine.is_stopped() {
                log::debug!(
                    "exiting connection #{:?} ({:?})",
                    connection_machine.connection_id,
                    connection_machine.hints
                );
                break;
            }
        }
    });
    worker
}
