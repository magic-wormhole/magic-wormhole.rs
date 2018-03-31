use std::collections::HashMap;

#[derive(Debug, Copy, Clone)]
pub struct WSHandle {
    id: u32,
}
impl WSHandle {
    pub fn new(id: u32) -> WSHandle {
        WSHandle { id: id }
    }
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

pub trait Core {
    fn allocate_code(&mut self) -> ();
    fn set_code(&mut self, code: &str) -> ();
    fn derive_key(&mut self, purpose: &str, length: u8) -> Vec<u8>;
    fn close(&mut self) -> ();

    fn get_action(&mut self) -> Option<Action>;

    fn timer_expired(&mut self, handle: TimerHandle) -> ();

    fn websocket_connection_made(&mut self, handle: WSHandle) -> ();
    fn websocket_message_received(&mut self, handle: WSHandle, message: &str) -> ();
    fn websocket_connection_lost(&mut self, handle: WSHandle) -> ();
}

#[derive(Debug)]
pub enum Action {
    GotWelcome(HashMap<String, String>), // actually anything JSON-able
    GotCode(String),                     // must be easy to canonically encode into UTF-8 bytes
    GotUnverifiedKey(Vec<u8>),
    GotVerifier(Vec<u8>),
    GotVersions(HashMap<String, String>), // actually anything JSON-able
    GotMessage(Vec<u8>),
    GotClosed(Result),

    StartTimer(TimerHandle, f32),
    CancelTimer(TimerHandle),

    WebSocketOpen(WSHandle, String), // url
    WebSocketSendMessage(WSHandle, String),
    WebSocketClose(WSHandle),
}

#[derive(Debug)]
pub enum Result {
    Happy,
    Error,
}
