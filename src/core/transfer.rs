use std::path::PathBuf;
use serde_derive::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PeerMessage {
    Offer(OfferType),
    Answer(AnswerType),
    Error(String),
    Transit(TransitType),
}

impl PeerMessage {
    pub fn new_offer_message(msg: impl Into<String>) -> Self {
        PeerMessage::Offer(OfferType::Message(msg.into()))
    }

    pub fn new_offer_file(name: impl Into<PathBuf>, size: u64) -> Self {
        PeerMessage::Offer(OfferType::File {
            filename: name.into(),
            filesize: size,
        })
    }

    pub fn new_message_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::MessageAck(msg.into()))
    }

    pub fn new_file_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::FileAck(msg.into()))
    }

    pub fn new_error_message(msg: impl Into<String>) -> Self {
        PeerMessage::Error(msg.into())
    }

    pub fn new_offer_directory(
        name: impl Into<PathBuf>,
        mode: impl Into<String>,
        compressed_size: u64,
        numbytes: u64,
        numfiles: u64,
    ) -> Self {
        PeerMessage::Offer(OfferType::Directory {
            dirname: name.into(),
            mode: mode.into(),
            zipsize: compressed_size,
            numbytes,
            numfiles,
        })
    }
    
    pub fn new_transit(abilities: Vec<Abilities>, hints: Vec<Hints>) -> Self {
        PeerMessage::Transit(TransitType {
            abilities_v1: abilities,
            hints_v1: hints,
        })
    }

    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    // TODO: This can error out so we should actually have error returning
    // capability here
    pub fn deserialize(msg: &str) -> Self {
        serde_json::from_str(msg).unwrap()
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OfferType {
    Message(String),
    File {
        filename: PathBuf,
        filesize: u64,
    },
    Directory {
        dirname: PathBuf,
        mode: String,
        zipsize: u64,
        numbytes: u64,
        numfiles: u64,
    },
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    MessageAck(String),
    FileAck(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitAck {
    pub ack: String,
    pub sha256: String,
}

impl TransitAck {
    pub fn new(msg: impl Into<String>, sha256: impl Into<String>) -> Self {
        TransitAck {
            ack: msg.into(),
            sha256: sha256.into(),
        }
    }

    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    // TODO: This can error out so we should actually have error returning
    // capability here
    pub fn deserialize(msg: &str) -> Self {
        serde_json::from_str(msg).unwrap()
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitType {
    pub abilities_v1: Vec<Abilities>,
    pub hints_v1: Vec<Hints>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Abilities {
    #[serde(rename = "type")]
    pub ttype: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum Hints {
    DirectTcpV1(DirectType),
    RelayV1(RelayType),
}

impl Hints {
    pub fn new_direct(priority: f32, hostname: &str, port: u16) -> Self {
        Hints::DirectTcpV1(
            DirectType {
                priority,
                hostname: hostname.to_string(),
                port,
            }
        )
    }

    pub fn new_relay(h: Vec<DirectType>) -> Self {
        Hints::RelayV1(
            RelayType { hints: h }
        )
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type", rename = "direct-tcp-v1")]
pub struct DirectType {
    pub priority: f32,
    pub hostname: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type", rename = "relay-v1")]
pub struct RelayType {
    pub hints: Vec<DirectType>,
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_message() {
        let m1 = PeerMessage::new_offer_message("hello from rust");
        assert_eq!(
            m1.serialize(),
            "{\"offer\":{\"message\":\"hello from rust\"}}"
        );
    }

    #[test]
    fn test_offer_file() {
        let f1 = PeerMessage::new_offer_file("somefile.txt", 34556);
        assert_eq!(
            f1.serialize(),
            "{\"offer\":{\"file\":{\"filename\":\"somefile.txt\",\"filesize\":34556}}}"
       );
    }

    #[test]
    fn test_offer_directory() {
        let d1 = PeerMessage::new_offer_directory("somedirectory", "zipped", 45, 1234, 10);
        assert_eq!(
            d1.serialize(),
            "{\"offer\":{\"directory\":{\"dirname\":\"somedirectory\",\"mode\":\"zipped\",\"numbytes\":1234,\"numfiles\":10,\"zipsize\":45}}}"
        );
    }

    #[test]
    fn test_message_ack() {
        let m1 = PeerMessage::new_message_ack("ok");
        assert_eq!(m1.serialize(), "{\"answer\":{\"message_ack\":\"ok\"}}");
    }

    #[test]
    fn test_file_ack() {
        let f1 = PeerMessage::new_file_ack("ok");
        assert_eq!(f1.serialize(), "{\"answer\":{\"file_ack\":\"ok\"}}");
    }

    #[test]
    fn test_transit_ack() {
        let f1 = TransitAck::new("ok", "deadbeaf");
        assert_eq!(f1.serialize(), "{\"ack\":\"ok\",\"sha256\":\"deadbeaf\"}");
    }

    #[test]
    fn test_transit() {
        let abilities = vec![
            Abilities {
                ttype: String::from("direct-tcp-v1"),
            },
            Abilities {
                ttype: String::from("relay-v1"),
            },
        ];
        let hints = vec![
            Hints::new_direct(0.0, "192.168.1.8", 46295),
            Hints::new_relay(vec![DirectType {
                priority: 2.0,
                hostname: "magic-wormhole-transit.debian.net".to_string(),
                port: 4001,
            }]),
        ];
        let t = PeerMessage::new_transit(abilities, hints);
        assert_eq!(t.serialize(), "{\"transit\":{\"abilities-v1\":[{\"type\":\"direct-tcp-v1\"},{\"type\":\"relay-v1\"}],\"hints-v1\":[{\"hostname\":\"192.168.1.8\",\"port\":46295,\"priority\":0.0,\"type\":\"direct-tcp-v1\"},{\"hints\":[{\"hostname\":\"magic-wormhole-transit.debian.net\",\"port\":4001,\"priority\":2.0,\"type\":\"direct-tcp-v1\"}],\"type\":\"relay-v1\"}]}}")
    }
}
