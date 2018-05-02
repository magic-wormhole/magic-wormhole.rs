use std::collections::HashMap;
use std::iter::FromIterator;
use std::str;
// Events come into the core, Actions go out of it (to the IO glue layer)
use api::{APIAction, APIEvent, IOAction, IOEvent, Mood, TimerHandle, WSHandle};

// A unit structure only used for state machine purpose. Actual wordlist is
// implemented by wordlist::PGPWordList.
// They are implemented differently, as when we add HashMap to structure it
// can't be copied and hence can't be used in pattern matching in state machine
// logic.
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Wordlist {}

// machines (or IO, or the API) emit these events, and each is routed to a
// specific machine (or IO or the API)

#[derive(Debug, PartialEq)]
pub enum AllocatorEvent {
    Allocate(u8, Wordlist),
    Connected,
    Lost,
    RxAllocated(String),
}

#[derive(Debug, PartialEq)]
pub enum BossEvent {
    RxWelcome,
    RxError,
    Error,
    Closed,
    GotCode(String),
    GotKey(Vec<u8>), // TODO: fixed length?
    Scared,
    Happy,
    GotVerifier(Vec<u8>), // TODO: fixed length (sha256)
    GotMessage(String, Vec<u8>),
}

#[derive(Debug, PartialEq)]
pub enum CodeEvent {
    AllocateCode(u8, Wordlist), // length, wordlist
    InputCode,
    SetCode(String),
    Allocated(String, String),
    GotNameplate(String),
    FinishedInput(String),
}

#[derive(Debug, PartialEq)]
pub enum InputEvent {
    Start,
    GotNameplates(Vec<String>),
    GotWordlist(Wordlist),
    ChooseNameplate(String),
    ChooseWords(String),
    RefreshNameplates,
}

#[derive(Debug, PartialEq)]
pub enum KeyEvent {
    GotCode(String),
    GotPake(Vec<u8>),
    GotMessage,
}

#[derive(Debug, PartialEq)]
pub enum ListerEvent {
    Connected,
    Lost,
    RxNameplates(Vec<String>),
    Refresh,
}

#[derive(Debug, PartialEq)]
pub enum MailboxEvent {
    Connected,
    Lost,
    RxMessage(String, String, Vec<u8>),
    RxClosed,
    Close(String),
    GotMailbox(String),
    GotMessage,
    AddMessage(String, Vec<u8>), // PAKE+VERSION from Key, PHASE from Send
}

#[derive(Debug, PartialEq)]
pub enum NameplateEvent {
    NameplateDone,
    Connected,
    Lost,
    RxClaimed(String),
    RxReleased,
    SetNameplate(String),
    Release,
    Close,
}

#[derive(Debug, PartialEq)]
pub enum OrderEvent {
    GotMessage(String, String, Vec<u8>),
}

#[derive(Debug, PartialEq)]
pub enum ReceiveEvent {
    GotMessage(String, String, Vec<u8>),
    GotKey(Vec<u8>),
}

use server_messages::Message;

#[derive(Debug, PartialEq)]
pub enum RendezvousEvent {
    Start,
    TxBind(String, String), // appid, side
    TxOpen(String),         // mailbox
    TxAdd(String, Vec<u8>), // phase, body
    TxClose,
    Stop,
    TxClaim(String),
    TxRelease(String),
    TxAllocate,
    TxList,
}

#[derive(PartialEq)]
pub enum SendEvent {
    Send(String, Vec<u8>), // phase, plaintext
    GotVerifiedKey(Vec<u8>),
}
use std::fmt;
impl fmt::Debug for SendEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &SendEvent::Send(ref phase, ref plaintext) => {
                let p = str::from_utf8(phase.as_bytes());
                match p {
                    Ok(p1) => write!(f, "Send({})", p1),
                    Err(_) => write!(f, "Send(non-UTF8)"),
                }
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
// TODO: tolerate events![] (causes warning)
macro_rules! events {
    ( $( $x:expr ),* ) => {
        {
            use events::Events;
            let mut temp_vec = Events::new();
            $(
                temp_vec.push($x);
            )*
            temp_vec
        }
    };
}
