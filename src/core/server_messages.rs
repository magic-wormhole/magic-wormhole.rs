use super::{AppID, ClientVersion, Mailbox, Mood, MySide, Nameplate, Phase, TheirSide};
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;

/// Special encoding for the `nameplates` message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Nameplate_ {
    pub id: String,
}

impl Nameplate_ {
    fn deserialize<'de, D>(de: D) -> Result<Vec<Nameplate>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: Vec<Nameplate_> = serde::Deserialize::deserialize(de)?;
        Ok(value.into_iter().map(|value| Nameplate(value.id)).collect())
    }

    #[allow(clippy::all, dead_code)]
    fn serialize<S>(value: &Vec<Nameplate>, ser: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ser.collect_seq(value.iter().map(|value| Self {
            id: value.to_string(),
        }))
    }
}

#[derive(Serialize, Debug, PartialEq, Eq, derive_more::Display)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "method")]
pub enum SubmitPermission {
    #[display(fmt = "Hashcash {{ stamp: '{}' }}", stamp)]
    Hashcash { stamp: String },
}

#[derive(Deserialize, Debug, PartialEq, Eq, Default)]
pub struct WelcomeMessage {
    #[deprecated(note = "This is for the Python client")]
    pub current_cli_version: Option<String>,
    pub motd: Option<String>,
    #[deprecated(note = "Servers should send a proper error message instead")]
    pub error: Option<String>,
    #[serde(rename = "permission-required")]
    pub permission_required: Option<PermissionRequired>,
}

impl std::fmt::Display for WelcomeMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "WelcomeMessage {{ ")?;
        if let Some(motd) = &self.motd {
            write!(f, "motd: '{}', ", motd)?;
        }
        if let Some(permission_required) = &self.permission_required {
            write!(f, "permission_required: '{}', ", permission_required)?;
        }
        write!(f, ".. }}")?;
        Ok(())
    }
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct PermissionRequired {
    #[serde(deserialize_with = "PermissionRequired::deserialize_none")]
    pub none: bool,
    pub hashcash: Option<HashcashPermission>,
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

impl PermissionRequired {
    fn deserialize_none<'de, D>(de: D) -> Result<bool, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: Option<serde_json::Map<String, serde_json::Value>> =
            serde::Deserialize::deserialize(de)?;
        Ok(value.is_some())
    }
}

impl std::fmt::Display for PermissionRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let none_iter = std::iter::once("none").filter(|_| self.none);
        let hashcash_iter = std::iter::once("hashcash").filter(|_| self.hashcash.is_some());
        let other_iter = self.other.keys().map(String::as_str);
        write!(
            f,
            "PermissionRequired {{ one of: {:?}}}",
            none_iter.chain(hashcash_iter).chain(other_iter)
        )
    }
}

#[derive(Deserialize, Debug, PartialEq, Eq, derive_more::Display)]
#[display(
    fmt = "HashcashPermission {{ bits: {}, resource: '{}' }}",
    bits,
    resource
)]
#[serde(deny_unknown_fields)]
pub struct HashcashPermission {
    pub bits: u32,
    pub resource: String,
}

#[derive(Debug, PartialEq, Clone, Deserialize, derive_more::Display)]
#[display(
    fmt = "EncryptedMessage {{ side: {}, phase: {}, body: {}",
    side,
    phase,
    "crate::util::DisplayBytes(body)"
)]
pub struct EncryptedMessage {
    pub side: TheirSide,
    pub phase: Phase,
    #[serde(deserialize_with = "hex::serde::deserialize")]
    pub body: Vec<u8>,
}

impl EncryptedMessage {
    pub fn decrypt(&self, key: &xsalsa20poly1305::Key) -> Option<Vec<u8>> {
        use super::key;
        let data_key = key::derive_phase_key(&self.side, key, &self.phase);
        key::decrypt_data(&data_key, &self.body)
    }
}

// Client sends only these
#[derive(Serialize, Debug, PartialEq, derive_more::Display)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum OutboundMessage<'a> {
    #[display(fmt = "SubmitPermission({})", _0)]
    SubmitPermission(SubmitPermission),
    #[display(
        fmt = "Bind {{ appid: {}, side: {}, client_version: {} }}",
        appid,
        side,
        client_version
    )]
    Bind {
        appid: AppID,
        side: MySide,
        client_version: ClientVersion<'a>,
    },
    List,
    Allocate,
    #[display(fmt = "Claim({})", nameplate)]
    Claim {
        nameplate: String,
    },
    #[display(fmt = "Release({})", nameplate)]
    Release {
        nameplate: String,
    }, // TODO: nominally optional
    #[display(fmt = "Open({})", mailbox)]
    Open {
        mailbox: Mailbox,
    },
    #[display(
        fmt = "Add {{ phase: {}, body: {} }}",
        phase,
        "crate::util::DisplayBytes(body)"
    )]
    Add {
        phase: Phase,
        #[serde(serialize_with = "hex::serde::serialize")]
        body: Vec<u8>,
    },
    #[display(fmt = "Close {{ mailbox: {}, mood: {} }}", mailbox, mood)]
    Close {
        mailbox: Mailbox,
        mood: Mood,
    },
    #[display(fmt = "Ping({})", ping)]
    Ping {
        ping: u64,
    },
}

const CLIENT_NAME: &str = "rust";
impl<'a> OutboundMessage<'a> {
    pub fn bind(appid: AppID, side: MySide) -> Self {
        let client_version_string: &str = env!("CARGO_PKG_VERSION");
        let client_version = ClientVersion::new(CLIENT_NAME, client_version_string);
        OutboundMessage::Bind {
            appid,
            side,
            client_version,
        }
    }

    pub fn claim(nameplate: impl Into<String>) -> Self {
        OutboundMessage::Claim {
            nameplate: nameplate.into(),
        }
    }

    pub fn release(nameplate: impl Into<String>) -> Self {
        OutboundMessage::Release {
            nameplate: nameplate.into(),
        }
    }

    pub fn open(mailbox: Mailbox) -> Self {
        OutboundMessage::Open { mailbox }
    }

    pub fn add(phase: Phase, body: Vec<u8>) -> Self {
        OutboundMessage::Add { body, phase }
    }

    pub fn close(mailbox: Mailbox, mood: Mood) -> Self {
        OutboundMessage::Close { mailbox, mood }
    }
}

// Server sends only these
#[derive(Deserialize, Debug, PartialEq, derive_more::Display)]
#[serde(rename_all = "kebab-case")]
#[serde(tag = "type")]
pub enum InboundMessage {
    #[display(fmt = "Welcome({})", welcome)]
    Welcome {
        welcome: WelcomeMessage,
    },
    #[display(fmt = "Nameplates({:?})", nameplates)]
    Nameplates {
        #[serde(with = "Nameplate_")]
        nameplates: Vec<Nameplate>,
    },
    #[display(fmt = "Allocated({})", nameplate)]
    Allocated {
        nameplate: Nameplate,
    },
    #[display(fmt = "Claimed({})", mailbox)]
    Claimed {
        mailbox: Mailbox,
    },
    Released,
    #[display(
        fmt = "Message {{ side: {}, phase: {:?}, body: {} }}",
        _0.side,
        _0.phase,
        "crate::util::DisplayBytes(_0.body.as_bytes())"
    )]
    Message(EncryptedMessage),
    Closed,
    Ack,
    #[display(fmt = "Pong({})", pong)]
    Pong {
        pong: u64,
    },
    #[display(fmt = "Error {{ error: {:?}, .. }}", error)]
    Error {
        error: String,
        orig: Box<InboundMessage>,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::{from_str, json, Value};
    use std::ops::Deref;

    #[test]
    fn test_bind() {
        let client_version_string: String = String::from(env!("CARGO_PKG_VERSION"));
        let m1 = OutboundMessage::bind(
            AppID::new("appid"),
            MySide::unchecked_from_string(String::from("side1")),
        );
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "bind", "appid": "appid",
                   "side": "side1", "client_version": ["rust", client_version_string]})
        );
    }

    #[test]
    fn test_client_version_string_rep() {
        let client_version = ClientVersion::new("foo", "1.0.2");

        assert_eq!(client_version.to_string(), "foo-1.0.2")
    }

    #[test]
    fn test_client_version_deref() {
        let client_version = ClientVersion::new("bar", "0.8.9");

        assert_eq!(client_version.deref(), &["bar", "0.8.9"])
    }

    #[test]
    fn test_list() {
        let m1 = OutboundMessage::List;
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "list"}));
    }

    #[test]
    fn test_allocate() {
        let m1 = OutboundMessage::Allocate;
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "allocate"}));
    }

    #[test]
    fn test_claim() {
        let m1 = OutboundMessage::claim("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "claim", "nameplate": "nameplate1"}));
    }

    #[test]
    fn test_release() {
        let m1 = OutboundMessage::release("nameplate1");
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "release", "nameplate": "nameplate1"}));
    }

    #[test]
    fn test_open() {
        let m1 = OutboundMessage::open(Mailbox(String::from("mailbox1")));
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(m2, json!({"type": "open", "mailbox": "mailbox1"}));
    }

    #[test]
    fn test_add() {
        let m1 = OutboundMessage::add(Phase("phase1".into()), b"body".to_vec());
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
        let m1 = OutboundMessage::close(Mailbox(String::from("mailbox1")), Mood::Happy);
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
        let m1 = OutboundMessage::close(Mailbox(String::from("mailbox1")), Mood::Errory);
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
        let m1 = OutboundMessage::close(Mailbox(String::from("mailbox1")), Mood::Scared);
        let s = serde_json::to_string(&m1).unwrap();
        let m2: Value = from_str(&s).unwrap();
        assert_eq!(
            m2,
            json!({"type": "close", "mailbox": "mailbox1",
                   "mood": "scary"})
        );
    }

    #[test]
    #[allow(deprecated)]
    fn test_welcome3() {
        let s = r#"{"type": "welcome", "welcome": {}, "server_tx": 1234.56}"#;
        let m = serde_json::from_str(s).unwrap();
        assert!(matches!(
            m,
            InboundMessage::Welcome {
                welcome: WelcomeMessage {
                    current_cli_version: None,
                    motd: None,
                    error: None,
                    permission_required: None
                }
            }
        ));
    }

    #[test]
    #[allow(deprecated)]
    fn test_welcome4() {
        let s = r#"{"type": "welcome", "welcome": {} }"#;
        let m = serde_json::from_str(s).unwrap();
        assert!(matches!(
            m,
            InboundMessage::Welcome {
                welcome: WelcomeMessage {
                    current_cli_version: None,
                    motd: None,
                    error: None,
                    permission_required: None
                }
            }
        ));
    }

    // TODO: when "error_on_line_overflow=false" lands on rustfmt(stable),
    // let's replace this cfg_attr with a change to our .rustfmt.toml
    #[test]
    #[rustfmt::skip]
    #[allow(deprecated)]
    fn test_welcome5() {
        let s = r#"{"type": "welcome", "welcome": { "motd": "hello world" }, "server_tx": 1234.56 }"#;
        let m = serde_json::from_str(s).unwrap();
        assert!(matches!(m, InboundMessage::Welcome { welcome: WelcomeMessage { current_cli_version: None, motd: Some(_), error: None, permission_required: None }  }));
    }

    /// Test permission_required field deserialization
    #[test]
    #[allow(deprecated)]
    fn test_welcome6() {
        let s = r#"{"type": "welcome", "welcome": { "motd": "hello world", "permission-required": { "none": {}, "hashcash": { "bits": 6, "resource": "resource-string" }, "dark-ritual": { "hocrux": true } } } }"#;
        let m: InboundMessage = serde_json::from_str(s).unwrap();
        assert_eq!(
            m,
            InboundMessage::Welcome {
                welcome: WelcomeMessage {
                    motd: Some("hello world".into()),
                    permission_required: Some(PermissionRequired {
                        none: true,
                        hashcash: Some(HashcashPermission {
                            bits: 6,
                            resource: "resource-string".into(),
                        }),
                        other: [("dark-ritual".to_string(), json!({ "hocrux": true }))]
                            .into_iter()
                            .collect()
                    }),
                    current_cli_version: None,
                    error: None,
                }
            }
        )
    }

    #[test]
    fn test_submit_permissions() {
        let m = OutboundMessage::SubmitPermission(SubmitPermission::Hashcash {
            stamp: "stamp".into(),
        });
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(
            s,
            r#"{"type":"submit-permission","method":"hashcash","stamp":"stamp"}"#
        );
    }

    #[test]
    fn test_ack() {
        let s = r#"{"type": "ack", "id": null, "server_tx": 1234.56}"#;
        let m = serde_json::from_str(s).unwrap();
        match m {
            InboundMessage::Ack {} => (),
            _ => panic!(),
        }
    }

    #[test]
    fn test_message() {
        let s = r#"{"body": "7b2270616b655f7631223a22353361346566366234363434303364376534633439343832663964373236646538396462366631336632613832313537613335646562393562366237633536353533227d", "server_rx": 1523468188.293486, "id": null, "phase": "pake", "server_tx": 1523498654.753594, "type": "message", "side": "side1"}"#;
        let m = serde_json::from_str(s).unwrap();
        match m {
            InboundMessage::Message(EncryptedMessage {
                side: _s,
                phase: _p,
                body: _b,
                //id: i
            }) => (),
            _ => panic!(),
        }
    }
}
