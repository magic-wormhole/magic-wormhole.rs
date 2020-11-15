use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::iter::FromIterator;
use std::ops::Deref;
use std::sync::Arc;
// Events come into the core, Actions go out of it (to the IO glue layer)
use super::api::{APIAction, IOAction, Mood};
use super::util::maybe_utf8;
use crate::core::util::random_bytes;
use zeroize::Zeroize;

pub use super::wordlist::Wordlist;

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
#[serde(from = "String")]
#[serde(into = "String")]
pub struct AppID(pub Arc<String>);

impl std::ops::Deref for AppID {
    type Target = String;

    /// Dereferences the value.
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl AppID {
    pub fn new(id: impl Into<String>) -> Self {
        AppID(Arc::new(id.into()))
    }
}

impl Into<String> for &AppID {
    fn into(self) -> String {
        (*self.0).clone()
    }
}

impl Into<String> for AppID {
    fn into(self) -> String {
        (*self.0).clone()
    }
}

impl From<String> for AppID {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

#[derive(PartialEq, Eq, Clone, Zeroize)]
#[zeroize(drop)]
pub struct Key(pub Vec<u8>);
impl Deref for Key {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key(REDACTED)")
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

// TheirSide is used for the String that arrives inside inbound messages
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

// machines (or IO, or the API) emit these events, and each is routed to a
// specific machine (or IO or the API)
#[derive(Debug, PartialEq)]
pub enum AllocatorEvent {
    Allocate(Arc<Wordlist>),
    Connected,
    RxAllocated(Nameplate),
}

#[allow(dead_code)] // TODO: drop dead code directive once core is complete
#[derive(PartialEq)]
pub enum BossEvent {
    RxWelcome(Value),
    Closed,
    GotCode(Code),
    GotKey(Key), // TODO: fixed length?
    Happy,
    GotVerifier(Vec<u8>), // TODO: fixed length (sha256)
    GotMessage(Phase, Vec<u8>),
}

impl fmt::Debug for BossEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::BossEvent::*;
        let t = match *self {
            RxWelcome(ref v) => format!("RxWelcome({:?})", v),
            Closed => String::from("Closed"),
            GotCode(ref code) => format!("GotCode({:?})", code),
            GotKey(ref _key) => String::from("GotKey(REDACTED)"),
            Happy => String::from("Happy"),
            GotVerifier(ref v) => format!("GotVerifier({})", maybe_utf8(v)),
            GotMessage(ref phase, ref msg) => {
                format!("GotMessage({:?}, {})", phase, maybe_utf8(msg))
            },
        };
        write!(f, "BossEvent::{}", t)
    }
}

#[derive(Debug, PartialEq)]
pub enum CodeEvent {
    AllocateCode(Arc<Wordlist>),
    SetCode(Code),
    Allocated(Nameplate, Code),
    GotNameplate(Nameplate),
}

#[derive(PartialEq)]
pub enum KeyEvent {
    GotCode(Code),
    GotPake(Vec<u8>),
}

impl fmt::Debug for KeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::KeyEvent::*;
        let t = match *self {
            GotCode(ref code) => format!("GotCode({:?})", code),
            GotPake(ref pake) => format!("GotPake({})", hex::encode(pake)),
        };
        write!(f, "KeyEvent::{}", t)
    }
}

#[derive(PartialEq)]
pub enum MailboxEvent {
    Connected,
    RxMessage(TheirSide, Phase, Vec<u8>), // side, phase, body
    RxClosed,
    Close(Mood),
    GotMailbox(Mailbox),
    AddMessage(Phase, Vec<u8>), // PAKE+VERSION from Key, PHASE from Send
}

impl fmt::Debug for MailboxEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::MailboxEvent::*;
        let t = match *self {
            Connected => String::from("Connected"),
            // Lost => String::from("Lost"),
            RxMessage(ref side, ref phase, ref body) => format!(
                "RxMessage(side={:?}, phase={:?}, body={})",
                side,
                phase,
                maybe_utf8(body)
            ),
            RxClosed => String::from("RxClosed"),
            Close(ref mood) => format!("Close({:?})", mood),
            GotMailbox(ref mailbox) => format!("GotMailbox({:?})", mailbox),
            AddMessage(ref phase, ref body) => {
                format!("AddMessage({:?}, {})", phase, maybe_utf8(body))
            },
        };
        write!(f, "MailboxEvent::{}", t)
    }
}

#[derive(Debug, PartialEq)]
pub enum NameplateEvent {
    Connected,
    RxClaimed(Mailbox),
    RxReleased,
    SetNameplate(Nameplate),
    Release,
    Close,
}

#[derive(PartialEq)]
pub enum OrderEvent {
    GotMessage(TheirSide, Phase, Vec<u8>), // side, phase, body
}

impl fmt::Debug for OrderEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::OrderEvent::*;
        let t = match *self {
            GotMessage(ref side, ref phase, ref body) => format!(
                "GotMessage(side={:?}, phase={:?}, body={})",
                side,
                phase,
                maybe_utf8(body)
            ),
        };
        write!(f, "OrderEvent::{}", t)
    }
}

#[derive(PartialEq)]
pub enum ReceiveEvent {
    GotMessage(TheirSide, Phase, Vec<u8>), // side, phase, body
    GotKey(Key),
}

impl fmt::Debug for ReceiveEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::ReceiveEvent::*;
        let t = match *self {
            GotMessage(ref side, ref phase, ref body) => format!(
                "GotMessage(side={:?}, phase={:?}, body={})",
                side,
                phase,
                maybe_utf8(body)
            ),
            GotKey(ref _key) => String::from("GotKey(REDACTED)"),
        };
        write!(f, "ReceiveEvent::{}", t)
    }
}

#[derive(PartialEq)]
pub enum RendezvousEvent {
    Start,
    TxBind(AppID, MySide),
    TxOpen(Mailbox),
    TxAdd(Phase, Vec<u8>), // phase, body
    TxClose(Mailbox, Mood),
    Stop,
    TxClaim(Nameplate),   // nameplate
    TxRelease(Nameplate), // nameplate
    TxAllocate,
    TxList,
}

impl fmt::Debug for RendezvousEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use self::RendezvousEvent::*;
        let t = match *self {
            Start => String::from("Start"),
            TxBind(ref appid, ref side) => format!("TxBind(appid={:?}, side={:?})", appid, side),
            TxOpen(ref mailbox) => format!("TxOpen({:?})", mailbox),
            TxAdd(ref phase, ref body) => format!("TxAdd({:?}, {})", phase, maybe_utf8(body)),
            TxClose(ref mailbox, ref mood) => format!("TxClose({:?}, {:?})", mailbox, mood),
            Stop => String::from("Stop"),
            TxClaim(ref nameplate) => format!("TxClaim({:?})", nameplate),
            TxRelease(ref nameplate) => format!("TxRelease({:?})", nameplate),
            TxAllocate => String::from("TxAllocate"),
            TxList => String::from("TxList"),
        };
        write!(f, "RendezvousEvent::{}", t)
    }
}

#[derive(PartialEq)]
pub enum SendEvent {
    Send(Phase, Vec<u8>), // phase, plaintext
    GotVerifiedKey(Key),
}

impl fmt::Debug for SendEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendEvent::Send(ref phase, ref plaintext) => {
                write!(f, "Send({:?}, {})", phase, maybe_utf8(plaintext))
            },
            SendEvent::GotVerifiedKey(_) => write!(f, "Send(GotVerifiedKey)"),
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TerminatorEvent {
    Close(Mood),
    MailboxDone,
    NameplateDone,
    Stopped,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    API(APIAction),
    IO(IOAction),
    Allocator(AllocatorEvent),
    Boss(BossEvent),
    Code(CodeEvent),
    Key(KeyEvent),
    Mailbox(MailboxEvent),
    Nameplate(NameplateEvent),
    Order(OrderEvent),
    Receive(ReceiveEvent),
    Rendezvous(RendezvousEvent),
    Send(SendEvent),
    Terminator(TerminatorEvent),
}

// conversion from specific event types to the generic Event

impl From<APIAction> for Event {
    fn from(r: APIAction) -> Self {
        Event::API(r)
    }
}

impl From<IOAction> for Event {
    fn from(r: IOAction) -> Self {
        Event::IO(r)
    }
}

impl From<AllocatorEvent> for Event {
    fn from(r: AllocatorEvent) -> Self {
        Event::Allocator(r)
    }
}

impl From<BossEvent> for Event {
    fn from(r: BossEvent) -> Self {
        Event::Boss(r)
    }
}

impl From<CodeEvent> for Event {
    fn from(r: CodeEvent) -> Self {
        Event::Code(r)
    }
}

impl From<KeyEvent> for Event {
    fn from(r: KeyEvent) -> Self {
        Event::Key(r)
    }
}

impl From<MailboxEvent> for Event {
    fn from(r: MailboxEvent) -> Self {
        Event::Mailbox(r)
    }
}

impl From<NameplateEvent> for Event {
    fn from(r: NameplateEvent) -> Self {
        Event::Nameplate(r)
    }
}

impl From<OrderEvent> for Event {
    fn from(r: OrderEvent) -> Self {
        Event::Order(r)
    }
}

impl From<ReceiveEvent> for Event {
    fn from(r: ReceiveEvent) -> Self {
        Event::Receive(r)
    }
}

impl From<RendezvousEvent> for Event {
    fn from(r: RendezvousEvent) -> Self {
        Event::Rendezvous(r)
    }
}

impl From<SendEvent> for Event {
    fn from(r: SendEvent) -> Self {
        Event::Send(r)
    }
}

impl From<TerminatorEvent> for Event {
    fn from(r: TerminatorEvent) -> Self {
        Event::Terminator(r)
    }
}

// a Vec that can accept specific event types, used in each Machine to gather
// their results

#[derive(Debug, PartialEq)]
pub struct Events {
    pub events: Vec<Event>,
}
use std::convert::From;
impl Events {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Events {
        Events { events: vec![] }
    }

    pub fn push<T>(&mut self, item: T)
    where
        Event: From<T>,
    {
        self.events.push(Event::from(item));
    }
}

impl IntoIterator for Events {
    type Item = Event;
    type IntoIter = ::std::vec::IntoIter<Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.into_iter()
    }
}

impl FromIterator<Event> for Events {
    fn from_iter<I: IntoIterator<Item = Event>>(iter: I) -> Self {
        let mut c = Events::new();

        for i in iter {
            c.events.push(i);
        }

        c
    }
}

// macro to build a whole Events vector, instead of adding them one at a time
macro_rules! events {
    ( ) => {
        {
            use crate::core::events::Events;
            Events::new()
        }
    };
    ( $( $x:expr ),* $(,)*) => {
        {
            use crate::core::events::Events;
            let mut temp_vec = Events::new();
            $(
                temp_vec.push($x);
            )*
            temp_vec
        }
    };
}
