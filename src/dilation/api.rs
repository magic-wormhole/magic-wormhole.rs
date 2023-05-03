use crate::core::MySide;
use derive_more::Display;
use serde_derive::{Deserialize, Serialize};

// from IO to DilationCore
#[derive(Debug, Clone, PartialEq, Display, Deserialize)]
pub enum IOEvent {
    WormholeMessageReceived(String),
    TCPConnectionLost,
    TCPConnectionMade,
}

/// Commands to be executed
#[derive(Debug, Clone, PartialEq, Display)]
pub enum ManagerCommand {
    // XXX: include API calls to IO layer
    Protocol(ProtocolCommand),
    IO(IOCommand),
}

/// Protocol level commands
#[derive(Debug, Clone, PartialEq, Display, Serialize)]
#[serde(tag = "type")]
pub enum ProtocolCommand {
    #[serde(rename = "please")]
    SendPlease { side: MySide },
}

/// Protocol level commands
#[derive(Debug, Clone, PartialEq, Display)]
pub enum IOCommand {
    CloseConnection,
}
