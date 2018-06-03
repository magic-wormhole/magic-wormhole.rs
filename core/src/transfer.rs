use serde_json;

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
        assert_eq!(
            m1.serialize(),
            "{\"answer\":{\"message_ack\":\"ok\"}}"
        );
    }

    #[test]
    fn test_file_ack() {
        let f1 = file_ack("ok");
        assert_eq!(
            f1.serialize(),
            "{\"answer\":{\"file_ack\":\"ok\"}}"
        );
    }
}
