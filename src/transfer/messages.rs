//! Over-the-wire messages for the file transfer (including transit)
//!
//! The transit protocol does not specify how to deliver the information to
//! the other side, so it is up to the file transfer to do that.

use crate::transit::{self, Ability, DirectHint};
use serde_derive::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use std::path::PathBuf;

/**
 * The type of message exchanged over the wormhole for this protocol
 */
#[derive(Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PeerMessage {
    Offer(OfferType),
    Answer(AnswerType),
    /** Tell the other side you got an error */
    Error(String),
    /** Used to set up a transit channel */
    Transit(TransitType),
    #[serde(other)]
    Unknown,
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

    pub fn new_message_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::MessageAck(msg.into()))
    }

    pub fn new_file_ack(msg: impl Into<String>) -> Self {
        PeerMessage::Answer(AnswerType::FileAck(msg.into()))
    }

    pub fn new_error_message(msg: impl Into<String>) -> Self {
        PeerMessage::Error(msg.into())
    }

    pub fn new_transit(abilities: Vec<transit::Ability>, hints: Vec<Hint>) -> Self {
        PeerMessage::Transit(TransitType {
            abilities_v1: abilities,
            hints_v1: hints,
        })
    }

    #[cfg(test)]
    pub fn serialize(&self) -> String {
        json!(self).to_string()
    }

    pub fn serialize_vec(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
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
    #[serde(other)]
    Unknown,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AnswerType {
    MessageAck(String),
    FileAck(String),
}

/**
 * A set of hints for both sides to find each other
 */
#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct TransitType {
    pub abilities_v1: Vec<Ability>,
    pub hints_v1: Vec<Hint>,
}

impl From<transit::Hints> for Vec<Hint> {
    fn from(hints: transit::Hints) -> Self {
        hints
            .direct_tcp
            .into_iter()
            .map(Hint::DirectTcpV1)
            .chain(std::iter::once(Hint::new_relay(hints.relay)))
            .collect()
    }
}

impl Into<transit::Hints> for Vec<Hint> {
    fn into(self) -> transit::Hints {
        let mut direct_tcp = Vec::new();
        let mut relay = Vec::new();

        /* There is only one "relay hint", though it may contain multiple
         * items. Yes, this is inconsistent and weird, watch your step.
         */
        for hint in self {
            match hint {
                Hint::DirectTcpV1(hint) => direct_tcp.push(hint),
                Hint::DirectUdtV1(_) => unimplemented!(),
                Hint::RelayV1(RelayHint { hints }) => relay.extend(hints),
            }
        }

        transit::Hints { direct_tcp, relay }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "kebab-case", tag = "type")]
#[non_exhaustive]
pub enum Hint {
    DirectTcpV1(DirectHint),
    DirectUdtV1(DirectHint),
    /* Weirdness alarm: a "relay hint" contains multiple "direct hints". This means
     * that there may be multiple direct hints, but if there are multiple relay hints
     * it's still only one item because it internally has a list.
     */
    RelayV1(RelayHint),
}

impl Hint {
    pub fn new_direct_tcp(priority: f32, hostname: &str, port: u16) -> Self {
        Hint::DirectTcpV1(DirectHint {
            hostname: hostname.to_string(),
            port,
        })
    }

    pub fn new_direct_udt(priority: f32, hostname: &str, port: u16) -> Self {
        Hint::DirectUdtV1(DirectHint {
            hostname: hostname.to_string(),
            port,
        })
    }

    pub fn new_relay(h: Vec<DirectHint>) -> Self {
        Hint::RelayV1(RelayHint { hints: h })
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct RelayHint {
    pub hints: Vec<DirectHint>,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_transit() {
        let abilities = vec![Ability::DirectTcpV1, Ability::RelayV1];
        let hints = vec![
            Hint::new_direct_tcp(0.0, "192.168.1.8", 46295),
            Hint::new_relay(vec![DirectHint {
                hostname: "magic-wormhole-transit.debian.net".to_string(),
                port: 4001,
            }]),
        ];
        let t = crate::transfer::PeerMessage::new_transit(abilities, hints);
        assert_eq!(t.serialize(), "{\"transit\":{\"abilities-v1\":[{\"type\":\"direct-tcp-v1\"},{\"type\":\"relay-v1\"}],\"hints-v1\":[{\"hostname\":\"192.168.1.8\",\"port\":46295,\"type\":\"direct-tcp-v1\"},{\"hints\":[{\"hostname\":\"magic-wormhole-transit.debian.net\",\"port\":4001}],\"type\":\"relay-v1\"}]}}")
    }
}
