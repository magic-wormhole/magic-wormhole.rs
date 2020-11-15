use super::events::{Code, Key};
use super::util::maybe_utf8;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
// use std::error::Error;
use std::fmt;

#[derive(PartialEq)]
pub enum APIEvent {
    // from application to IO glue to WormholeCore
    Start,
    AllocateCode(usize), // num_words
    SetCode(Code),
    Close,
    Send(Vec<u8>),
}

impl fmt::Debug for APIEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::APIEvent::*;
        let t = match *self {
            Start => String::from("Start"),
            AllocateCode(ref num_words) => format!("AllocateCode({})", num_words),
            SetCode(ref code) => format!("SetCode({:?})", code),
            Close => String::from("Close"),
            Send(ref msg) => format!("Send({})", maybe_utf8(msg)),
        };
        write!(f, "APIEvent::{}", t)
    }
}

// the serialized forms of these variants are part of the wire protocol, so
// they must be spelled exactly as shown
#[derive(Debug, PartialEq, Copy, Clone, Deserialize, Serialize)]
pub enum Mood {
    #[serde(rename = "happy")]
    Happy,
    #[serde(rename = "lonely")]
    Lonely,
    #[serde(rename = "errory")]
    Errory,
    #[serde(rename = "scary")]
    Scared,
    #[serde(rename = "unwelcome")]
    Unwelcome,
}

#[derive(PartialEq)]
pub enum APIAction {
    // from WormholeCore out through IO glue to application
    GotWelcome(Value),
    GotCode(Code), // must be easy to canonically encode into UTF-8 bytes
    GotUnverifiedKey(Key),
    GotVerifier(Vec<u8>),
    GotVersions(Value),
    GotMessage(Vec<u8>),
    GotClosed(Mood),
}

impl fmt::Debug for APIAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::APIAction::*;
        let t = match *self {
            GotWelcome(ref welcome) => format!("GotWelcome({:?})", welcome),
            GotCode(ref code) => format!("GotCode({:?})", code),
            GotUnverifiedKey(ref _key) => String::from("GotUnverifiedKey(REDACTED)"),
            GotVerifier(ref v) => format!("GotVerifier({})", hex::encode(v)),
            GotVersions(ref versions) => format!("GotVersions({:?})", versions),
            GotMessage(ref msg) => format!("GotMessage({})", maybe_utf8(msg)),
            GotClosed(ref mood) => format!("GotClosed({:?})", mood),
        };
        write!(f, "APIAction::{}", t)
    }
}

// Handles should be unforgeable: the struct is pub so they can be referenced
// by application code, but the 'id' field is private, so they cannot be
// constructed externally, nor can existing ones be modified.

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct WSHandle(u64);
impl WSHandle {
    pub(crate) fn new(id: u64) -> WSHandle {
        WSHandle(id)
    }
}

#[derive(Debug, PartialEq)]
pub enum IOEvent {
    // from IO glue layer into WormholeCore
    WebSocketConnectionMade(WSHandle),
    WebSocketMessageReceived(WSHandle, String),
    WebSocketConnectionLost(WSHandle),
}

#[derive(Debug, PartialEq)]
pub enum IOAction {
    WebSocketOpen(WSHandle, String), // url
    WebSocketSendMessage(WSHandle, String),
    WebSocketClose(WSHandle),
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn test_display() {
        // verify that APIActions have their key redacted
        let w: Value = json!("howdy");
        assert_eq!(
            format!("{:?}", APIAction::GotWelcome(w)),
            r#"APIAction::GotWelcome(String("howdy"))"#
        );
        assert_eq!(
            format!("{:?}", APIAction::GotCode(Code("4-code".into()))),
            r#"APIAction::GotCode(Code("4-code"))"#
        );
        assert_eq!(
            format!(
                "{:?}",
                APIAction::GotUnverifiedKey(Key("secret_key".into()))
            ),
            r#"APIAction::GotUnverifiedKey(REDACTED)"#
        );
        assert_eq!(
            format!("{:?}", APIAction::GotVerifier("verf".into())),
            r#"APIAction::GotVerifier(76657266)"#
        );
        let v: Value = json!("v1");
        assert_eq!(
            format!("{:?}", APIAction::GotVersions(v)),
            r#"APIAction::GotVersions(String("v1"))"#
        );
        assert_eq!(
            format!("{:?}", APIAction::GotMessage("howdy".into())),
            r#"APIAction::GotMessage((s=howdy))"#
        );
        assert_eq!(
            format!("{:?}", APIAction::GotClosed(Mood::Happy)),
            r#"APIAction::GotClosed(Happy)"#
        );
    }
}
