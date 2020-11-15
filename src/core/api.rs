use super::events::{Code, Key};
use super::util::maybe_utf8;
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
// use std::error::Error;
use std::fmt;

#[derive(PartialEq)]
pub enum APIEvent {
    AllocateCode(usize), // num_words
    SetCode(Code),
    Close,
    Send(Vec<u8>),
}

impl fmt::Debug for APIEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::APIEvent::*;
        let t = match *self {
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

#[derive(Debug, PartialEq)]
pub enum IOEvent {
    WebSocketMessageReceived(String),
    WebSocketConnectionLost,
}

#[derive(Debug, PartialEq)]
pub enum IOAction {
    WebSocketSendMessage(String),
    WebSocketClose,
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
