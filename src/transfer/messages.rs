//! Over-the-wire messages for the file transfer (including transit)
//!
//! The transit protocol does not specify how to deliver the information to
//! the other side, so it is up to the file transfer to do that. hfoo

use crate::transit::{self, Abilities as TransitAbilities};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

/**
 * The type of message exchanged over the wormhole for this protocol
 */
#[derive(Deserialize, Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PeerMessage {
    Offer(Offer),
    OfferV2(OfferV2),
    Answer(Answer),
    AnswerV2(AnswerV2),
    /** Tell the other side you got an error */
    Error(String),
    /** Used to set up a transit channel */
    Transit(TransitV1),
    TransitV2(TransitV2),
    #[serde(other)]
    Unknown,
}

impl PeerMessage {
    pub fn offer_message(msg: impl Into<String>) -> Self {
        PeerMessage::Offer(Offer::Message(msg.into()))
    }

    pub fn offer_file(name: impl Into<PathBuf>, size: u64) -> Self {
        PeerMessage::Offer(Offer::File {
            filename: name.into(),
            filesize: size,
        })
    }

    #[allow(dead_code)]
    pub fn offer_directory(
        name: impl Into<PathBuf>,
        mode: impl Into<String>,
        compressed_size: u64,
        numbytes: u64,
        numfiles: u64,
    ) -> Self {
        PeerMessage::Offer(Offer::Directory {
            dirname: name.into(),
            mode: mode.into(),
            zipsize: compressed_size,
            numbytes,
            numfiles,
        })
    }

    #[allow(dead_code)]
    pub fn message_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(Answer::MessageAck(msg.into()))
    }

    pub fn file_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(Answer::FileAck(msg.into()))
    }

    pub fn error_message(msg: impl Into<String>) -> Self {
        PeerMessage::Error(msg.into())
    }

    pub fn transit(abilities: TransitAbilities, hints: transit::Hints) -> Self {
        PeerMessage::Transit(TransitV1 {
            abilities_v1: abilities,
            hints_v1: hints,
        })
    }

    #[allow(dead_code)]
    pub fn transit_v2(hints: transit::Hints) -> Self {
        PeerMessage::TransitV2(TransitV2 { hints })
    }

    #[allow(dead_code)]
    pub fn ser_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }

    #[allow(dead_code)]
    pub fn ser_msgpack(&self) -> Vec<u8> {
        let mut writer = Vec::with_capacity(128);
        let mut ser = rmp_serde::encode::Serializer::new(&mut writer)
            .with_struct_map()
            .with_human_readable();
        serde::Serialize::serialize(self, &mut ser).unwrap();
        writer
    }

    #[allow(dead_code)]
    pub fn de_msgpack(data: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_read(&mut &*data)
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Offer {
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
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct OfferV2 {
    transfer_name: Option<String>,
    files: Vec<OfferV2Entry>,
    format: String, // TODO use custom enum?
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct OfferV2Entry {
    path: String,
    size: u64,
    mtime: u64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    MessageAck(String),
    FileAck(String),
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct AnswerV2 {
    files: HashMap<u64, u64>,
}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV1 {
    pub abilities_v1: TransitAbilities,
    pub hints_v1: transit::Hints,
}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV2 {
    pub hints: transit::Hints,
}

#[cfg(test)]
mod test {
    use super::*;
    use transit::{Abilities, DirectHint, RelayHint};

    #[test]
    fn test_transit() {
        let abilities = Abilities::ALL_ABILITIES;
        let hints = transit::Hints::new(
            [DirectHint::new("192.168.1.8", 46295)],
            [RelayHint::new(
                None,
                [DirectHint::new("magic-wormhole-transit.debian.net", 4001)],
                [],
            )],
        );
        assert_eq!(
            serde_json::json!(crate::transfer::PeerMessage::transit(abilities, hints)),
            serde_json::json!({
                "transit": {
                    "abilities-v1": [{"type":"direct-tcp-v1"},{"type":"relay-v1"},{"type":"relay-v2"}],
                    "hints-v1": [
                        {"hostname":"192.168.1.8","port":46295,"type":"direct-tcp-v1"},
                        {
                            "type": "relay-v1",
                            "hints": [
                                {"hostname": "magic-wormhole-transit.debian.net", "port": 4001 }
                            ]
                        },
                        {
                            "type": "relay-v2",
                            "hints": [
                                {"type": "tcp", "hostname": "magic-wormhole-transit.debian.net", "port": 4001}
                            ],
                            "name": null
                        }
                    ],
                }
            })
        );
    }

    #[test]
    fn test_message() {
        let m1 = PeerMessage::offer_message("hello from rust");
        assert_eq!(
            serde_json::json!(m1).to_string(),
            "{\"offer\":{\"message\":\"hello from rust\"}}"
        );
    }

    #[test]
    fn test_offer_file() {
        let f1 = PeerMessage::offer_file("somefile.txt", 34556);
        assert_eq!(
            serde_json::json!(f1).to_string(),
            "{\"offer\":{\"file\":{\"filename\":\"somefile.txt\",\"filesize\":34556}}}"
        );
    }

    #[test]
    fn test_offer_directory() {
        let d1 = PeerMessage::offer_directory("somedirectory", "zipped", 45, 1234, 10);
        assert_eq!(
            serde_json::json!(d1).to_string(),
            "{\"offer\":{\"directory\":{\"dirname\":\"somedirectory\",\"mode\":\"zipped\",\"numbytes\":1234,\"numfiles\":10,\"zipsize\":45}}}"
        );
    }

    #[test]
    fn test_message_ack() {
        let m1 = PeerMessage::message_ack("ok");
        assert_eq!(
            serde_json::json!(m1).to_string(),
            "{\"answer\":{\"message_ack\":\"ok\"}}"
        );
    }

    #[test]
    fn test_file_ack() {
        let f1 = PeerMessage::file_ack("ok");
        assert_eq!(
            serde_json::json!(f1).to_string(),
            "{\"answer\":{\"file_ack\":\"ok\"}}"
        );
    }
}
