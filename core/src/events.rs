// Events come into the core, Actions go out of it (to the IO glue layer)
use api::{APIAction, APIEvent, IOAction, IOEvent};
//use allocator::AllocatorEvent;
//use boss::BossEvent;
//use code::CodeEvent;
//use input::InputEvent;
//use key::KeyEvent;
//use lister::ListerEvent;
//use mailbox::MailboxEvent;
//use nameplate::NameplateEvent;
//use order::OrderEvent;
//use receive::ReceiveEvent;
use rendezvous::RendezvousEvent;
//use send::SendEvent;
//use terminator::TerminatorEvent;

#[derive(Debug, PartialEq)]
pub enum MachineEvent {
    //Allocator(AllocatorEvent),
    //Boss(BossEvent),
    //Code(CodeEvent),
    //Input(InputEvent),
    //Key(KeyEvent),
    //Lister(ListerEvent),
    //Mailbox(MailboxEvent),
    //Nameplate(NameplateEvent),
    //Order(OrderEvent),
    //Receive(ReceiveEvent),
    Rendezvous(RendezvousEvent),
    //Send(SendEvent),
    //Terminator(TerminatorEvent),
}

pub enum InboundEvent { // from IO glue layer
    IO(IOEvent),
    API(APIEvent),
}

pub enum Action { // to IO glue later
    // outbound
    IO(IOAction),
    API(APIAction),
}

pub enum ProcessEvent {
    API(APIEvent),
    IO(IOEvent),
    Machine(MachineEvent),
}

impl From<InboundEvent> for ProcessEvent {
    fn from(r: InboundEvent) -> Self {
        match r {
            InboundEvent::API(a) => ProcessEvent::API(a),
            InboundEvent::IO(a) => ProcessEvent::IO(a),
        }
    }
}

impl From<RendezvousEvent> for ProcessEvent {
    fn from(r: RendezvousEvent) -> Self {
        ProcessEvent::Machine(MachineEvent::Rendezvous(r))
    }
}


pub enum Result { // superset for WormholeCore::execute internals
    API(APIAction),
    IO(IOAction),
    Machine(MachineEvent),
}

#[derive(Debug, PartialEq)]
pub enum BossResult { // only Boss is allowed to emit APIActions
    API(APIAction),
    Machine(MachineEvent),
}

impl From<BossResult> for Result {
    fn from(r: BossResult) -> Self {
        match r {
            BossResult::API(a) => Result::API(a),
            BossResult::Machine(e) => Result::Machine(e),
        }
    }
}

pub enum RendezvousResult { // only Rendezvous is allowed to emit IOAction
    IO(IOAction),
    Machine(MachineEvent),
}

impl From<RendezvousResult> for Result {
    fn from(r: RendezvousResult) -> Self {
        match r {
            RendezvousResult::IO(a) => Result::IO(a),
            RendezvousResult::Machine(e) => Result::Machine(e),
        }
    }
}

// other machines are only allowed to emit MachineEvent
impl From<MachineEvent> for Result {
    fn from(e: MachineEvent) -> Self {
        Result::Machine(e)
    }
}
