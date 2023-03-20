use derive_more::Display;

pub enum APIAction {
}

// from IO to DilationCore
#[derive(Debug, Display)]
pub enum IOEvent {
    WormholeMessageReceived(String),
    TCPConnectionLost,
    TCPConnectionMade,
}

// from DilationCore to IO
#[derive(Debug, Clone, PartialEq, Display)]
pub enum IOAction {
    SendPlease,
}

pub enum Action {
    // outbound events to IO layer
    // XXX: include API calls to IO layer
    IO(IOAction),
}
