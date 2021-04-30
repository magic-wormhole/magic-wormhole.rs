use crate::{
    core::{
        server_messages::{InboundMessage, OutboundMessage},
        util::random_bytes,
    },
    APIEvent,
};
use serde_derive::{Deserialize, Serialize};
use std::{fmt, ops::Deref};

pub use super::wordlist::Wordlist;

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
pub struct AppID(pub String);

impl std::ops::Deref for AppID {
    type Target = String;

    /// Dereferences the value.
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AppID {
    pub fn new(id: impl Into<String>) -> Self {
        AppID(id.into())
    }
}

impl Into<String> for AppID {
    fn into(self) -> String {
        self.0
    }
}

impl From<String> for AppID {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

fn generate_side() -> String {
    let mut bytes: [u8; 5] = [0; 5];
    random_bytes(&mut bytes);
    hex::encode(bytes)
}

// MySide is used for the String that we send in all our outbound messages
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct MySide(EitherSide);

impl MySide {
    pub fn generate() -> MySide {
        MySide(EitherSide(generate_side()))
    }
    // It's a minor type system feature that converting an arbitrary string into MySide is hard.
    // This prevents it from getting swapped around with TheirSide.
    #[cfg(test)]
    pub fn unchecked_from_string(s: String) -> MySide {
        MySide(EitherSide(s))
    }
}

impl Deref for MySide {
    type Target = EitherSide;
    fn deref(&self) -> &EitherSide {
        &self.0
    }
}

// TheirSide is used for the string that arrives inside inbound messages
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct TheirSide(EitherSide);

impl<S: Into<String>> From<S> for TheirSide {
    fn from(s: S) -> TheirSide {
        TheirSide(EitherSide(s.into()))
    }
}

impl Deref for TheirSide {
    type Target = EitherSide;
    fn deref(&self) -> &EitherSide {
        &self.0
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct EitherSide(pub String);

impl<S: Into<String>> From<S> for EitherSide {
    fn from(s: S) -> EitherSide {
        EitherSide(s.into())
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Hash, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Phase(pub String);

impl Phase {
    pub fn is_version(&self) -> bool {
        &self.0[..] == "version"
    }
    pub fn is_pake(&self) -> bool {
        &self.0[..] == "pake"
    }
    pub fn to_num(&self) -> Option<u64> {
        self.0.parse().ok()
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Mailbox(pub String);

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Nameplate(pub String);
impl Nameplate {
    pub fn new(n: &str) -> Self {
        Nameplate(String::from(n))
    }
}
impl Deref for Nameplate {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl Into<String> for Nameplate {
    fn into(self) -> String {
        self.0
    }
}

impl fmt::Display for Nameplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Code(pub String);
impl Deref for Code {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct EncryptedMessage {
    pub side: TheirSide,
    pub phase: Phase,
    pub body: Vec<u8>,
}

impl EncryptedMessage {
    pub fn decrypt(&self, key: &xsalsa20poly1305::Key) -> anyhow::Result<Vec<u8>> {
        use super::key;
        let data_key = key::derive_phase_key(&self.side, key, &self.phase);
        key::decrypt_data(&data_key, &self.body)
            .ok_or_else(|| anyhow::format_err!("Got bad message that could not be decrypted"))
    }
}

#[derive(Debug)]
pub enum Event {
    /** Got a message from the server */
    FromIO(InboundMessage),
    ToIO(OutboundMessage),
    /** This is second to the last command issued by the core */
    CloseWebsocket,
    /** This is the last event received by the core. After this the event loop will exit. */
    WebsocketClosed,
    /** Sometimes we queue up messages and then release them */
    BounceMessage(EncryptedMessage),
    FromAPI(Vec<u8>),
    ToAPI(APIEvent),
    /** Close the connection to the server
     *
     * This might trigger a series of events to release all resources and end up with [`Event::WebsocketClosed`]
     */
    ShutDown(anyhow::Result<()>),
}

// conversion from specific event types to the generic Event

impl From<APIEvent> for Event {
    fn from(r: APIEvent) -> Self {
        Event::ToAPI(r)
    }
}

impl From<OutboundMessage> for Event {
    fn from(r: OutboundMessage) -> Self {
        Event::ToIO(r)
    }
}
