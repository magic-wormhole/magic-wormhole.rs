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

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct DirectType {
    priority: f32,
    hostname: String,
    port: u16,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct RelayType {
    hints: Vec<DirectType>,
}

impl PeerMessage {
    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    // TODO: This can error out so we should actually have error returning
    // capability here
    pub fn deserialize(msg: &str) -> Self {
        serde_json::from_str(msg).unwrap()
    }
}

pub fn message(msg: &str) -> PeerMessage {
    PeerMessage::Offer(OfferType::Message(msg.to_string()))
}

pub fn offer_file(name: &str, size: u32) -> PeerMessage {
    PeerMessage::Offer(OfferType::File {
        filename: name.to_string(),
        filesize: size,
    })
}

pub fn message_ack(msg: &str) -> PeerMessage {
    PeerMessage::Answer(AnswerType::MessageAck(msg.to_string()))
}

pub fn file_ack(msg: &str) -> PeerMessage {
    PeerMessage::Answer(AnswerType::FileAck(msg.to_string()))
}

pub fn error_message(msg: &str) -> PeerMessage {
    PeerMessage::Error(msg.to_string())
}

pub fn offer_directory(
    name: &str,
    mode: &str,
    compressed_size: u32,
    numbytes: u32,
    numfiles: u32,
) -> PeerMessage {
    PeerMessage::Offer(OfferType::Directory {
        dirname: name.to_string(),
        mode: mode.to_string(),
        zipsize: compressed_size,
        numbytes,
        numfiles,
    })
}

pub fn transit(abilities: Vec<Abilities>, hints: Vec<Hints>) -> PeerMessage {
    PeerMessage::Transit(TransitType {
        abilities_v1: abilities,
        hints_v1: hints,
    })
}

pub fn direct_type(priority: f32, hostname: &str, port: u16) -> DirectType {
    DirectType {
        priority,
        hostname: hostname.to_string(),
        port,
    }
}

pub fn relay_type(h: Vec<DirectType>) -> RelayType {
    RelayType { hints: h }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_message() {
        let m1 = message("hello from rust");
        assert_eq!(
            m1.serialize(),
            "{\"offer\":{\"message\":\"hello from rust\"}}"
        );
    }

    #[test]
    fn test_offer_file() {
        let f1 = offer_file("somefile.txt", 34556);
        assert_eq!(
            f1.serialize(),
            "{\"offer\":{\"file\":{\"filename\":\"somefile.txt\",\"filesize\":34556}}}"
       );
    }

    #[test]
    fn test_offer_directory() {
        let d1 = offer_directory("somedirectory", "zipped", 45, 1234, 10);
        assert_eq!(
            d1.serialize(),
            "{\"offer\":{\"directory\":{\"dirname\":\"somedirectory\",\"mode\":\"zipped\",\"numbytes\":1234,\"numfiles\":10,\"zipsize\":45}}}"
        );
    }

    #[test]
    fn test_message_ack() {
        let m1 = message_ack("ok");
        assert_eq!(m1.serialize(), "{\"answer\":{\"message_ack\":\"ok\"}}");
    }

    #[test]
    fn test_file_ack() {
        let f1 = file_ack("ok");
        assert_eq!(f1.serialize(), "{\"answer\":{\"file_ack\":\"ok\"}}");
    }

    #[test]
    fn test_transit() {
        let abilities = vec![
            Abilities {
                ttype: "direct-tcp-v1".to_string(),
            },
            Abilities {
                ttype: "relay-v1".to_string(),
            },
        ];
        let hints = vec![
            Hints::DirectTcpV1(direct_type(0.0, "192.168.1.8", 46295)),
            Hints::RelayV1(relay_type(vec![direct_type(
                2.0,
                "magic-wormhole-transit.debian.net",
                4001,
            )])),
        ];
        let t = transit(abilities, hints);
        assert_eq!(t.serialize(), "{\"transit\":{\"abilities-v1\":[{\"type\":\"direct-tcp-v1\"},{\"type\":\"relay-v1\"}],\"hints-v1\":[{\"hostname\":\"192.168.1.8\",\"port\":46295,\"priority\":0.0,\"type\":\"direct-tcp-v1\"},{\"hints\":[{\"hostname\":\"magic-wormhole-transit.debian.net\",\"port\":4001,\"priority\":2.0}],\"type\":\"relay-v1\"}]}}")
    }
}
