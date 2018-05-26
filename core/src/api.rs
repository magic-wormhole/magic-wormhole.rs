use hex;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use util::maybe_utf8;
use events::Key;

#[derive(PartialEq)]
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

impl fmt::Debug for APIEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::APIEvent::*;
        let t = match *self {
            Start => "Start".to_string(),
            AllocateCode(ref num_words) => {
                format!("AllocateCode({})", num_words)
            }
            InputCode => "InputCode".to_string(),
            InputHelperRefreshNameplates => {
                "InputHelperRefreshNameplates".to_string()
            }
            InputHelperChooseNameplate(ref nameplate) => {
                format!("InputHelperChooseNameplate({})", nameplate)
            }
            InputHelperChooseWords(ref words) => {
                format!("InputHelperChooseWords({})", words)
            }
            SetCode(ref code) => format!("SetCode({})", code),
            Close => "Close".to_string(),
            Send(ref msg) => format!("Send({})", maybe_utf8(msg)),
        };
        write!(f, "APIEvent::{}", t)
    }
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
    Unwelcome,
}

impl Mood {
    fn to_string(&self) -> String {
        // this is used for protocol messages as well as debug output
        match *self {
            Mood::Happy => "happy".to_string(),
            Mood::Lonely => "lonely".to_string(),
            Mood::Error => "error".to_string(),
            Mood::Scared => "scared".to_string(),
            Mood::Unwelcome => "unwelcome".to_string(),
        }
    }
}

impl fmt::Display for Mood {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

#[derive(PartialEq)]
pub enum APIAction {
    // from WormholeCore out through IO glue to application
    GotWelcome(HashMap<String, String>), // actually anything JSON-able: Value
    GotCode(String), // must be easy to canonically encode into UTF-8 bytes
    GotUnverifiedKey(Key),
    GotVerifier(Vec<u8>),
    GotVersions(HashMap<String, String>), // actually anything JSON-able
    GotMessage(Vec<u8>),
    GotClosed(Mood),
}

impl fmt::Debug for APIAction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::APIAction::*;
        let t = match *self {
            GotWelcome(ref welcome) => format!("GotWelcome({:?})", welcome),
            GotCode(ref code) => format!("GotCode({})", code),
            GotUnverifiedKey(ref _key) => {
                "GotUnverifiedKey(REDACTED)".to_string()
            }
            GotVerifier(ref v) => format!("GotVerifier({})", hex::encode(v)),
            GotVersions(ref versions) => format!("GotVersions({:?})", versions),
            GotMessage(ref msg) => format!("GotMessage({})", maybe_utf8(msg)),
            GotClosed(ref mood) => format!("GotClosed({:?})", mood),
        };
        write!(f, "APIAction::{}", t)
    }
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
