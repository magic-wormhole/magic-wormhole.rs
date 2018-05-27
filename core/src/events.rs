use hex;
use serde_json::Value;
use std::fmt;
use std::iter::FromIterator;
use std::ops::Deref;
use std::sync::Arc;
// Events come into the core, Actions go out of it (to the IO glue layer)
use api::{APIAction, IOAction, Mood};
use util::maybe_utf8;

pub use wordlist::Wordlist;

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AppID(pub String);
impl Deref for AppID {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl fmt::Display for AppID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

impl<'a> From<&'a str> for AppID {
    fn from(s: &'a str) -> AppID {
        AppID(s.to_string())
    }
}

#[derive(PartialEq, Eq, Clone)]
pub struct Key(pub Vec<u8>);
impl Deref for Key {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Key(REDACTED)")
    }
}

// MySide is used for the String that we send in all our outbound messages
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct MySide(pub String);
impl Deref for MySide {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

// TheirSide is used for the String that arrives inside inbound messages
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct TheirSide(pub String);
impl Deref for TheirSide {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Hash)]
pub struct Phase(pub String);
impl Deref for Phase {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", &self.0)
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Mailbox(pub String);
impl Deref for Mailbox {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
    }
}
impl fmt::Display for Mailbox {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Mailbox({})", &self.0)
    }
}

#[derive(PartialEq, Eq, Clone, Debug)]
pub struct Nameplate(pub String);
impl Deref for Nameplate {
    type Target = String;
    fn deref(&self) -> &String {
        &self.0
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

// machines (or IO, or the API) emit these events, and each is routed to a
// specific machine (or IO or the API)
#[derive(Debug, PartialEq)]
pub enum AllocatorEvent {
    Allocate(Arc<Wordlist>),
    Connected,
    Lost,
    RxAllocated(Nameplate),
}

#[allow(dead_code)] // TODO: drop dead code directive once core is complete
#[derive(PartialEq)]
pub enum BossEvent {
    RxWelcome(Value),
    RxError,
    Error,
    Closed,
    GotCode(Code),
    GotKey(Key), // TODO: fixed length?
    Scared,
    Happy,
    GotVerifier(Vec<u8>), // TODO: fixed length (sha256)
    GotMessage(Phase, Vec<u8>),
}

impl fmt::Debug for BossEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::BossEvent::*;
        let t = match *self {
            RxWelcome(ref v) => format!("RxWelcome({:?})", v),
            RxError => "RxError".to_string(),
            Error => "Error".to_string(),
            Closed => "Closed".to_string(),
            GotCode(ref code) => format!("GotCode({:?})", code),
            GotKey(ref _key) => "GotKey(REDACTED)".to_string(),
            Scared => "Scared".to_string(),
            Happy => "Happy".to_string(),
            GotVerifier(ref v) => format!("GotVerifier({})", maybe_utf8(v)),
            GotMessage(ref phase, ref msg) => {
                format!("GotMessage({:?}, {})", phase, maybe_utf8(msg))
            }
        };
        write!(f, "BossEvent::{}", t)
    }
}

#[derive(Debug, PartialEq)]
pub enum CodeEvent {
    AllocateCode(Arc<Wordlist>),
    InputCode,
    SetCode(Code),
    Allocated(Nameplate, Code),
    GotNameplate(Nameplate),
    FinishedInput(Code),
}

#[derive(Debug, PartialEq)]
pub enum InputEvent {
    Start,
    ChooseNameplate(Nameplate),
    ChooseWords(String),
    GotNameplates(Vec<Nameplate>),
    GotWordlist(Arc<Wordlist>),
    RefreshNameplates,
}

#[allow(dead_code)] // TODO: Drop dead code directive once core is complete
#[derive(PartialEq)]
pub enum KeyEvent {
    GotCode(Code),
    GotPake(Vec<u8>),
}

impl fmt::Debug for KeyEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::KeyEvent::*;
        let t = match *self {
            GotCode(ref code) => format!("GotCode({:?})", code),
            GotPake(ref pake) => format!("GotPake({})", hex::encode(pake)),
        };
        write!(f, "KeyEvent::{}", t)
    }
}

#[derive(Debug, PartialEq)]
pub enum ListerEvent {
    Connected,
    Lost,
    RxNameplates(Vec<Nameplate>),
    Refresh,
}

#[derive(PartialEq)]
pub enum MailboxEvent {
    Connected,
    Lost,
    RxMessage(TheirSide, Phase, Vec<u8>), // side, phase, body
    RxClosed,
    Close(Mood),
    GotMailbox(Mailbox),
    AddMessage(Phase, Vec<u8>), // PAKE+VERSION from Key, PHASE from Send
}

impl fmt::Debug for MailboxEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::MailboxEvent::*;
        let t = match *self {
            Connected => "Connected".to_string(),
            Lost => "Lost".to_string(),
            RxMessage(ref side, ref phase, ref body) => format!(
                "RxMessage(side={:?}, phase={}, body={})",
                side,
                phase,
                maybe_utf8(body)
            ),
            RxClosed => "RxClosed".to_string(),
            Close(ref mood) => format!("Close({:?})", mood),
            GotMailbox(ref mailbox) => format!("GotMailbox({})", mailbox),
            AddMessage(ref phase, ref body) => {
                format!("AddMessage({}, {})", phase, maybe_utf8(body))
            }
        };
        write!(f, "MailboxEvent::{}", t)
    }
}

#[derive(Debug, PartialEq)]
pub enum NameplateEvent {
    Connected,
    Lost,
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::OrderEvent::*;
        let t = match *self {
            GotMessage(ref side, ref phase, ref body) => format!(
                "GotMessage(side={:?}, phase={}, body={})",
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::ReceiveEvent::*;
        let t = match *self {
            GotMessage(ref side, ref phase, ref body) => format!(
                "GotMessage(side={:?}, phase={}, body={})",
                side,
                phase,
                maybe_utf8(body)
            ),
            GotKey(ref _key) => "GotKey(REDACTED)".to_string(),
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::RendezvousEvent::*;
        let t = match *self {
            Start => "Start".to_string(),
            TxBind(ref appid, ref side) => {
                format!("TxBind(appid={}, side={:?})", appid, side)
            }
            TxOpen(ref mailbox) => format!("TxOpen({})", mailbox),
            TxAdd(ref phase, ref body) => {
                format!("TxAdd({}, {})", phase, maybe_utf8(body))
            }
            TxClose(ref mailbox, ref mood) => {
                format!("TxClose({}, {:?})", mailbox, mood)
            }
            Stop => "Stop".to_string(),
            TxClaim(ref nameplate) => format!("TxClaim({:?})", nameplate),
            TxRelease(ref nameplate) => format!("TxRelease({:?})", nameplate),
            TxAllocate => "TxAllocate".to_string(),
            TxList => "TxList".to_string(),
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
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &SendEvent::Send(ref phase, ref plaintext) => {
                write!(f, "Send({}, {})", phase, maybe_utf8(plaintext))
            }
            &SendEvent::GotVerifiedKey(_) => write!(f, "Send(GotVerifiedKey)"),
        }
    }
}

#[derive(Debug, PartialEq)]
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
    Input(InputEvent),
    Key(KeyEvent),
    Lister(ListerEvent),
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

impl From<InputEvent> for Event {
    fn from(r: InputEvent) -> Self {
        Event::Input(r)
    }
}

impl From<KeyEvent> for Event {
    fn from(r: KeyEvent) -> Self {
        Event::Key(r)
    }
}

impl From<ListerEvent> for Event {
    fn from(r: ListerEvent) -> Self {
        Event::Lister(r)
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
    pub fn new() -> Events {
        Events { events: vec![] }
    }
    //fn add<T>(&mut self, item: T) where T: Into<Event> {
    pub fn push<T>(&mut self, item: T)
    where
        Event: From<T>,
    {
        self.events.push(Event::from(item));
    }

    pub fn append(&mut self, other: &mut Events) {
        self.events.append(&mut other.events);
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
// TODO: tolerate events![first,] (trailing comma breaks it)
macro_rules! events {
    ( $( $x:expr ),* ) => {
        {
            use events::Events;
            #[allow(unused_mut)] // hush warning on events![]
            let mut temp_vec = Events::new();
            $(
                temp_vec.push($x);
            )*
            temp_vec
        }
    };
}
