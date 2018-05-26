use serde_json;

use api::Mood;
use serde::{Deserialize, Deserializer};
use util;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Nameplate {
    pub id: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct WelcomeMsg {
    pub motd: String,
}

// convert an optional field (which may result in deserialization error)
// into an Option::None.
pub fn invalid_option<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    Option<T>: Deserialize<'de>,
{
    Option::<T>::deserialize(de).or_else(|_| Ok(None))
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PeerMessage {
    Offer(OfferType),
    Answer(AnswerType),
    Error(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OfferType {
    Message(String),
    File {
        filename: String,
        filesize: u32,
    },
    Directory {
        dirname: String,
        mode: String,
        zipsize: u32,
        numbytes: u32,
        numfiles: u32,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    MessageAck(String),
    FileAck(String),
}

pub fn deserialize_peer_message(msg: &str) -> PeerMessage {
    serde_json::from_str(msg).unwrap()
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum Message {
    Bind {
        appid: String,
        side: String,
    },
    Welcome {
        #[serde(default)]
        server_tx: Option<f64>,
        #[serde(deserialize_with = "invalid_option")]
        welcome: Option<WelcomeMsg>,
    },
    List {},
    Nameplates {
        nameplates: Vec<Nameplate>,
    },
    Allocate {},
    Allocated {
        nameplate: String,
    },
    Claim {
        nameplate: String,
    },
    Claimed {
        mailbox: String,
    },
    Release {
        nameplate: String,
    }, // TODO: nominally optional
    Released {},
    Open {
        mailbox: String,
    },
    Add {
        phase: String,
        body: String,
    },
    Message {
        side: String,
        phase: String,
        body: String,
        //id: String,
    },
    Close {
        mailbox: String,
        mood: String,
    },
    Closed {},
    Ack {},
    Ping {
        ping: u32,
    },
    Pong {
        pong: u32,
    },
    //Error { error: String, orig: Message },
}

// Client only sends: bind, list, allocate, claim, release, open, add, close,
// ping

pub fn bind(appid: &str, side: &str) -> Message {
    Message::Bind {
        appid: appid.to_string(),
        side: side.to_string(),
    }
}
pub fn list() -> Message {
    Message::List {}
}
pub fn allocate() -> Message {
    Message::Allocate {}
}
pub fn claim(nameplate: &str) -> Message {
    Message::Claim {
        nameplate: nameplate.to_string(),
    }
}

pub fn release(nameplate: &str) -> Message {
    Message::Release {
        nameplate: nameplate.to_string(),
    }
}
pub fn open(mailbox: &str) -> Message {
    Message::Open {
        mailbox: mailbox.to_string(),
    }
}

pub fn add(phase: &str, body: &[u8]) -> Message {
    // TODO: make this take Vec<u8>, do the hex-encoding internally
    let hexstr = util::bytes_to_hexstr(body);

    Message::Add {
        phase: phase.to_string(),
        body: hexstr,
    }
}

pub fn close(mailbox: &str, mood: Mood) -> Message {
    Message::Close {
        mailbox: mailbox.to_string(),
        mood: mood.to_string(),
    }
}

#[allow(dead_code)]
pub fn ping(ping: u32) -> Message {
    Message::Ping { ping: ping }
}

#[allow(dead_code)]
pub fn welcome(motd: &str, timestamp: f64) -> Message {
    Message::Welcome {
        welcome: Some(WelcomeMsg {
            motd: motd.to_string(),
        }),
        server_tx: Some(timestamp),
    }
}

// Server sends: welcome, nameplates, allocated, claimed, released, message,
// closed, ack, pong, error

pub fn deserialize(s: &str) -> Message {
    serde_json::from_str(&s).unwrap()
}

#[cfg(test)]
mod test {
    use super::*;
    use api::Mood;

    #[test]
    fn test_bind() {
        let m1 = bind("appid", "side1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_list() {
        let m1 = list();
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_allocate() {
        let m1 = allocate();
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_claim() {
        let m1 = claim("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_release() {
        let m1 = release("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_open() {
        let m1 = open("mailbox1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_add() {
        let m1 = add("phase1", b"body");
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_close() {
        let m1 = close("mailbox1", Mood::Happy);
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_ping() {
        let m1 = ping(123);
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_welcome1() {
        let m1 = welcome("hi", 1234.56);
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_welcome2() {
        let m1 = welcome("", 1234.56);
        let s = serde_json::to_string(&m1).unwrap();
        let m2 = deserialize(&s);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_welcome3() {
        let s = r#"{"type": "welcome", "welcome": {}, "server_tx": 1234.56}"#;
        let m = deserialize(&s);
        match m {
            Message::Welcome {
                welcome: msg,
                server_tx: ts,
            } => {
                match msg {
                    None => (),
                    _ => panic!(),
                }
                assert_eq!(ts, Some(1234.56));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_welcome4() {
        let s = r#"{"type": "welcome", "welcome": {} }"#;
        let m = deserialize(&s);
        match m {
            Message::Welcome {
                welcome: msg,
                server_tx: ts,
            } => {
                match msg {
                    None => (),
                    _ => panic!(),
                }
                match ts {
                    None => (),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    // TODO: when "error_on_line_overflow=false" lands on rustfmt(stable),
    // let's replace this cfg_attr with a change to our .rustfmt.toml
    #[test]
    #[cfg_attr(rustfmt, rustfmt_skip)]
    fn test_welcome5() {
        let s = r#"{"type": "welcome", "welcome": { "motd": "hello world" }, "server_tx": 1234.56 }"#;
        let m = deserialize(&s);
        match m {
            Message::Welcome {
                welcome: msg,
                server_tx: ts,
            } => {
                match msg {
                    Some(wmsg) => match wmsg {
                        WelcomeMsg { motd: msg_of_day } => {
                            assert_eq!(msg_of_day, "hello world".to_string());
                        }
                        //_ => panic!(),
                    },
                    _ => panic!(),
                }
                match ts {
                    Some(t) => {
                        assert_eq!(t, 1234.56);
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_ack() {
        let s = r#"{"type": "ack", "id": null, "server_tx": 1234.56}"#;
        let m = deserialize(&s);
        match m {
            Message::Ack {} => (),
            _ => panic!(),
        }
    }

    #[test]
    fn test_message() {
        let s = r#"{"body": "7b2270616b655f7631223a22353361346566366234363434303364376534633439343832663964373236646538396462366631336632613832313537613335646562393562366237633536353533227d", "server_rx": 1523468188.293486, "id": null, "phase": "pake", "server_tx": 1523498654.753594, "type": "message", "side": "side1"}"#;
        let m = deserialize(&s);
        match m {
            Message::Message {
                side: _s,
                phase: _p,
                body: _b,
                //id: i
            } => (),
            _ => panic!(),
        }
    }
}
