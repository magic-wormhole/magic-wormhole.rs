//! Over-the-wire messages for the file transfer (including transit)
//!
//! The transit protocol does not specify how to deliver the information to
//! the other side, so it is up to the file transfer to do that. hfoo

use crate::transit::{self, Ability, DirectHint};
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

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

    pub fn transit(abilities: Vec<transit::Ability>, hints: Vec<Hint>) -> Self {
        PeerMessage::Transit(TransitV1 {
            abilities_v1: abilities,
            hints_v1: hints,
        })
    }

    #[allow(dead_code)]
    pub fn transit_v2(hints: Vec<Hint>) -> Self {
        PeerMessage::TransitV2(TransitV2 { hints_v2: hints })
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
            .with_string_variants();
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
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV1 {
    pub abilities_v1: Vec<Ability>,
    pub hints_v1: Vec<Hint>,
}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitV2 {
    pub hints_v2: Vec<Hint>,
}

impl From<transit::Hints> for Vec<Hint> {
    fn from(hints: transit::Hints) -> Self {
        hints
            .direct_tcp
            .into_iter()
            .map(Hint::DirectTcpV1)
            .chain(hints.relay.into_iter().map(|hint| Hint::relay_v1(hint.tcp)))
            .collect()
    }
}

impl Into<transit::Hints> for Vec<Hint> {
    fn into(self) -> transit::Hints {
        let mut direct_tcp = HashSet::new();
        let mut relay = Vec::<transit::RelayHint>::new();
        let mut relay_v2 = Vec::<transit::RelayHint>::new();

        for hint in self {
            match hint {
                Hint::DirectTcpV1(hint) => {
                    direct_tcp.insert(hint);
                },
                Hint::RelayV1 { hints } => {
                    relay.push(transit::RelayHint {
                        tcp: hints,
                        ..transit::RelayHint::default()
                    });
                },
                Hint::RelayV2 { urls } => {
                    let hint = transit::RelayHint::new(urls);
                    hint.merge_into(&mut relay_v2);
                },
                /* Ignore unknown hints */
                _ => {},
            }
        }

        if !relay_v2.is_empty() {
            relay.clear();
        }
        relay.extend(relay_v2.into_iter().map(Into::into));

        transit::Hints { direct_tcp, relay }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
pub enum Hint {
    DirectTcpV1(DirectHint),
    /* Weirdness alarm: a "relay hint" contains multiple "direct hints". This means
     * that there may be multiple direct hints, but if there are multiple relay hints
     * it's still only one item because it internally has a list.
     */
    RelayV1 {
        hints: HashSet<DirectHint>,
    },
    RelayV2 {
        urls: HashSet<url::Url>,
    },
    #[serde(other)]
    Unknown,
}

impl Hint {
    pub fn direct_tcp(_priority: f32, hostname: &str, port: u16) -> Self {
        Hint::DirectTcpV1(DirectHint {
            hostname: hostname.to_string(),
            port,
        })
    }

    pub fn relay_v1(h: impl IntoIterator<Item = DirectHint>) -> Self {
        Hint::RelayV1 {
            hints: h.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_transit() {
        let abilities = vec![Ability::DirectTcpV1, Ability::RelayV1];
        let hints = vec![
            Hint::direct_tcp(0.0, "192.168.1.8", 46295),
            Hint::relay_v1(vec![DirectHint {
                hostname: "magic-wormhole-transit.debian.net".to_string(),
                port: 4001,
            }]),
        ];
        let t =
            serde_json::json!(crate::transfer::PeerMessage::transit(abilities, hints)).to_string();
        assert_eq!(t, "{\"transit\":{\"abilities-v1\":[{\"type\":\"direct-tcp-v1\"},{\"type\":\"relay-v1\"}],\"hints-v1\":[{\"hostname\":\"192.168.1.8\",\"port\":46295,\"type\":\"direct-tcp-v1\"},{\"hints\":[{\"hostname\":\"magic-wormhole-transit.debian.net\",\"port\":4001}],\"type\":\"relay-v1\"}]}}")
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
