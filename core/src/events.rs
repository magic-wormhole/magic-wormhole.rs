use std::collections::HashMap;
// Events come into the core, Actions go out of it (to the IO glue layer)
use api::{APIAction, APIEvent, IOAction, IOEvent, Mood, TimerHandle, WSHandle};

#[derive(Debug, PartialEq)]
pub enum Machine {
}

// machines (or IO, or the API) emit these events, and each is routed to a
// specific machine (or IO or the API)

#[derive(Debug, PartialEq)]
pub enum AllocatorEvent {
    Connected,
    Lost,
    RxAllocated,
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
    GotMessage(String, String, Vec<u8>),
}

#[derive(Debug, PartialEq)]
pub enum CodeEvent {
    AllocateCode,
    InputCode,
    SetCode(String),
    Allocated,
    GotNameplate,
    FinishedInput,
}

#[derive(Debug, PartialEq)]
pub enum InputEvent {
    Start,
    GotNameplates,
    GotWordlist,
}

#[derive(Debug, PartialEq)]
pub enum KeyEvent {
    GotPake,
    GotMessage,
}

#[derive(Debug, PartialEq)]
pub enum ListerEvent {
    Connected,
    Lost,
    RxNameplates,
    Refresh,
}

#[derive(Debug, PartialEq)]
pub enum MailboxEvent {
    Connected,
    Lost,
    RxMessage,
    RxClosed,
    Close,
    GotMailbox,
    GotMessage,
    AddMessage, // PAKE+VERSION from Key, PHASE from Send
}

#[derive(Debug, PartialEq)]
pub enum NameplateEvent {
    NameplateDone,
    Connected,
    Lost,
    RxClaimed,
    RxReleased,
    SetNameplate,
    Release,
}

#[derive(Debug, PartialEq)]
pub enum OrderEvent {
    GotMessage,
}

#[derive(Debug, PartialEq)]
pub enum ReceiveEvent {
    GotCode,
    GotKey,
}

use server_messages::Message;

#[derive(Debug, PartialEq)]
pub enum RendezvousEvent {
    Start,
    TxBind(Message),
    TxOpen,
    TxAdd,
    TxClose,
    Stop,
    TxClaim,
    TxRelease,
    TxAllocate,
    TxList,
}

#[derive(Debug, PartialEq)]
pub enum SendEvent {
    Send(Vec<u8>),
    GotVerifiedKey,
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
    pub fn push<T>(&mut self, item: T) where Event: From<T> {
        self.events.push(Event::from(item));
    }
    // TODO: iter
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
