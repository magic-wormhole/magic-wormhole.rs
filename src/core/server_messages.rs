use super::api::Mood;
use super::events::{AppID, Mailbox, MySide};
use super::util;
use serde_derive::{Deserialize, Serialize};
use serde_json::{self, Value};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Nameplate {
    pub id: String,
}

// Client sends only these
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum OutboundMessage {
    Bind { appid: AppID, side: String },
    List {},
    Allocate {},
    Claim { nameplate: String },
    Release { nameplate: String }, // TODO: nominally optional
    Open { mailbox: String },
    Add { phase: String, body: String },
    Close { mailbox: String, mood: Mood },
    Ping { ping: u64 },
}

pub fn bind(appid: AppID, side: &MySide) -> OutboundMessage {
    OutboundMessage::Bind {
        side: side.to_string(),
        appid,
    }
}
pub fn list() -> OutboundMessage {
    OutboundMessage::List {}
}
pub fn allocate() -> OutboundMessage {
    OutboundMessage::Allocate {}
}
pub fn claim(nameplate: &str) -> OutboundMessage {
    OutboundMessage::Claim {
        nameplate: nameplate.to_string(),
    }
}

pub fn release(nameplate: &str) -> OutboundMessage {
    OutboundMessage::Release {
        nameplate: nameplate.to_string(),
    }
}
pub fn open(mailbox: &Mailbox) -> OutboundMessage {
    OutboundMessage::Open {
        mailbox: mailbox.0.to_string(),
    }
}

pub fn add(phase: &str, body: &[u8]) -> OutboundMessage {
    // TODO: make this take Vec<u8>, do the hex-encoding internally
    let hexstr = util::bytes_to_hexstr(body);

    OutboundMessage::Add {
        phase: phase.to_string(),
        body: hexstr,
    }
}

pub fn close(mailbox: &Mailbox, mood: Mood) -> OutboundMessage {
    OutboundMessage::Close {
        mailbox: mailbox.0.to_string(),
        mood,
    }
}

// we only parse our own outbound messages in unit tests
#[cfg(test)]
pub fn deserialize_outbound(s: &str) -> OutboundMessage {
    serde_json::from_str(&s).unwrap()
}

#[allow(dead_code)]
pub fn ping(ping: u64) -> OutboundMessage {
    OutboundMessage::Ping { ping }
}

// Server sends only these
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum InboundMessage {
    Welcome {
        welcome: Value, // left mostly-intact for application
    },
    Nameplates {
        nameplates: Vec<Nameplate>,
    },
    Allocated {
        nameplate: String,
    },
    Claimed {
        mailbox: String,
    },
    Released {},
    Message {
        side: String,
        phase: String,
        body: String,
        //id: String,
    },
    Closed {},
    Ack {},
    Pong {
        pong: u64,
    },
    //Error { error: String, orig: Message },
}

pub fn deserialize(s: &str) -> InboundMessage {
    serde_json::from_str(&s).unwrap()
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::api::Mood;
    use serde_json::from_str;
    use serde_json::json;

    #[test]
    fn test_bind() {
        let m1 = bind(
            &AppID(String::from("appid")),
            &MySide(String::from("side1")),
        );
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "bind", "appid": "appid",
                              "side": "side1"})
        );
    }

    #[test]
    fn test_list() {
        let m1 = list();
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "list"}));
    }

    #[test]
    fn test_allocate() {
        let m1 = allocate();
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "allocate"}));
    }

    #[test]
    fn test_claim() {
        let m1 = claim("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "claim", "nameplate": "nameplate1"}));
    }

    #[test]
    fn test_release() {
        let m1 = release("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "release", "nameplate": "nameplate1"}));
    }

    #[test]
    fn test_open() {
        let m1 = open(&Mailbox(String::from("mailbox1")));
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "open", "mailbox": "mailbox1"}));
    }

    #[test]
    fn test_add() {
        let m1 = add("phase1", b"body");
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "add", "phase": "phase1",
                              "body": "626f6479"})
        ); // body is hex-encoded
    }

    #[test]
    fn test_close() {
        let m1 = close(&Mailbox(String::from("mailbox1")), Mood::Happy);
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "close", "mailbox": "mailbox1",
                              "mood": "happy"})
        );
    }

    #[test]
    fn test_close_errory() {
        let m1 = close(&Mailbox(String::from("mailbox1")), Mood::Errory);
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "close", "mailbox": "mailbox1",
                              "mood": "errory"})
        );
    }

    #[test]
    fn test_close_scared() {
        let m1 = close(&Mailbox(String::from("mailbox1")), Mood::Scared);
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "close", "mailbox": "mailbox1",
                              "mood": "scary"})
        );
    }

    #[test]
    fn test_ping() {
        let m1 = ping(123);
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "ping", "ping": 123}));
    }

    #[test]
    fn test_welcome3() {
        let s = r#"{"type": "welcome", "welcome": {}, "server_tx": 1234.56}"#;
        let m = deserialize(&s);
        match m {
            InboundMessage::Welcome { welcome: msg } => {
                assert_eq!(msg, json!({}))
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_welcome4() {
        let s = r#"{"type": "welcome", "welcome": {} }"#;
        let m = deserialize(&s);
        match m {
            InboundMessage::Welcome { welcome: msg } => {
                assert_eq!(msg, json!({}))
            }
            _ => panic!(),
        }
    }

    // TODO: when "error_on_line_overflow=false" lands on rustfmt(stable),
    // let's replace this cfg_attr with a change to our .rustfmt.toml
    #[test]
    #[rustfmt::skip]
    fn test_welcome5() {
        let s = r#"{"type": "welcome", "welcome": { "motd": "hello world" }, "server_tx": 1234.56 }"#;
        let m = deserialize(&s);
        match m {
            InboundMessage::Welcome { welcome: msg } =>
                assert_eq!(msg, json!({"motd": "hello world"})),
            _ => panic!(),
        }
    }

    #[test]
    fn test_ack() {
        let s = r#"{"type": "ack", "id": null, "server_tx": 1234.56}"#;
        let m = deserialize(&s);
        match m {
            InboundMessage::Ack {} => (),
            _ => panic!(),
        }
    }

    #[test]
    fn test_message() {
        let s = r#"{"body": "7b2270616b655f7631223a22353361346566366234363434303364376534633439343832663964373236646538396462366631336632613832313537613335646562393562366237633536353533227d", "server_rx": 1523468188.293486, "id": null, "phase": "pake", "server_tx": 1523498654.753594, "type": "message", "side": "side1"}"#;
        let m = deserialize(&s);
        match m {
            InboundMessage::Message {
                side: _s,
                phase: _p,
                body: _b,
                //id: i
            } => (),
            _ => panic!(),
        }
    }
}
