use std::collections::HashMap;
use std::error::Error;
use std::fmt;

#[derive(Debug, PartialEq)]
pub enum APIEvent {
    // from application to IO glue to WormholeCore
    Start,
    AllocateCode(usize), // num_words
    InputCode,
    InputHelperRefreshNameplates,
    InputHelperChooseNameplate(String),
    InputHelperChooseWords(String),
    SetCode(String),
    Close,
    Send(Vec<u8>),
}

#[derive(Debug, PartialEq)]
pub enum InputHelperError {
    Inactive,
    MustChooseNameplateFirst,
    AlreadyChoseNameplate,
    AlreadyChoseWords,
}

impl fmt::Display for InputHelperError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InputHelperError::Inactive => write!(f, "Inactive"),
            InputHelperError::MustChooseNameplateFirst => {
                write!(f, "Should Choose Nameplate first")
            }
            InputHelperError::AlreadyChoseNameplate => {
                write!(f, "nameplate already chosen, can't go back")
            }
            InputHelperError::AlreadyChoseWords => {
                write!(f, "Words are already chosen")
            }
        }
    }
}

impl Error for InputHelperError {
    fn description(&self) -> &str {
        match *self {
            InputHelperError::Inactive => "Input is not yet started",
            InputHelperError::MustChooseNameplateFirst => {
                "You should input name plate first!"
            }
            InputHelperError::AlreadyChoseNameplate => {
                "Nameplate is already chosen, you can't go back!"
            }
            InputHelperError::AlreadyChoseWords => {
                "Words are already chosen you can't go back!"
            }
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Mood {
    Happy,
    Lonely,
    Error,
    Scared,
}

impl fmt::Display for Mood {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Mood::Happy => write!(f, "happy"),
            Mood::Lonely => write!(f, "lonely"),
            Mood::Error => write!(f, "error"),
            Mood::Scared => write!(f, "scared"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum APIAction {
    // from WormholeCore out through IO glue to application
    GotWelcome(HashMap<String, String>), // actually anything JSON-able: Value
    GotCode(String), // must be easy to canonically encode into UTF-8 bytes
    GotUnverifiedKey(Vec<u8>),
    GotVerifier(Vec<u8>),
    GotVersions(HashMap<String, String>), // actually anything JSON-able
    GotMessage(Vec<u8>),
    GotClosed(Mood),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct TimerHandle {
    id: u32,
}
impl TimerHandle {
    pub fn new(id: u32) -> TimerHandle {
        TimerHandle { id: id }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct WSHandle {
    id: u32,
}
impl WSHandle {
    pub fn new(id: u32) -> WSHandle {
        WSHandle { id: id }
    }
}

#[derive(Debug, PartialEq)]
pub enum IOEvent {
    // from IO glue layer into WormholeCore
    TimerExpired(TimerHandle),
    WebSocketConnectionMade(WSHandle),
    WebSocketMessageReceived(WSHandle, String),
    WebSocketConnectionLost(WSHandle),
}

#[derive(Debug, PartialEq)]
pub enum IOAction {
    // commands from WormholeCore out to IO glue layer
    StartTimer(TimerHandle, f32),
    CancelTimer(TimerHandle),

    WebSocketOpen(WSHandle, String), // url
    WebSocketSendMessage(WSHandle, String),
    WebSocketClose(WSHandle),
}

// disabled: for now, the glue should call separate do_api/do_io methods
// with an APIEvent or IOEvent respectively
//pub enum InboundEvent { // from IO glue layer
//    IO(IOEvent),
//    API(APIEvent),
//}

#[derive(Debug, PartialEq)]
pub enum Action {
    // to IO glue layer
    // outbound
    IO(IOAction),
    API(APIAction),
}
