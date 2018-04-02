use std::collections::HashMap;

pub enum APIEvent { // from application to IO glue to WormholeCore
    AllocateCode,
    SetCode(String),
    Close,
    Send,
}

#[derive(Debug, PartialEq)]
pub enum Result {
    Happy,
    Error,
}

#[derive(Debug, PartialEq)]
pub enum APIAction { // from WormholeCore out through IO glue to application
    GotWelcome(HashMap<String, String>), // actually anything JSON-able: Value
    GotCode(String),                     // must be easy to canonically encode into UTF-8 bytes
    GotUnverifiedKey(Vec<u8>),
    GotVerifier(Vec<u8>),
    GotVersions(HashMap<String, String>), // actually anything JSON-able
    GotMessage(Vec<u8>),
    GotClosed(Result),
}


#[derive(Debug, Copy, Clone)]
pub struct TimerHandle {
    id: u32,
}
impl TimerHandle {
    pub fn new(id: u32) -> TimerHandle {
        TimerHandle { id: id }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct WSHandle {
    id: u32,
}
impl WSHandle {
    pub fn new(id: u32) -> WSHandle {
        WSHandle { id: id }
    }
}

pub enum IOEvent { // from IO glue layer into WormholeCore
    TimerExpired(TimerHandle),
    WebSocketConnectionMade(WSHandle),
    WebSocketMessageReceived(WSHandle, String),
    WebSocketConnectionLost(WSHandle),
}

pub enum IOAction { // commands from WormholeCore out to IO glue layer
    StartTimer(TimerHandle, f32),
    CancelTimer(TimerHandle),

    WebSocketOpen(WSHandle, String), // url
    WebSocketSendMessage(WSHandle, String),
    WebSocketClose(WSHandle),
}
