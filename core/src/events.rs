// Events come into the core, Actions go out of it (to the IO glue layer)
use super::api::{APIAction, APIEvent, IOAction, IOEvent};
use super::allocator::AllocatorEvent;
use super::boss::BossEvent;
use super::code::CodeEvent;
use super::input::InputEvent;
use super::key::KeyEvent;
use super::lister::ListerEvent;
use super::mailbox::MailboxEvent;
use super::nameplate::NameplateEvent;
use super::order::OrderEvent;
use super::receive::ReceiveEvent;
use super::rendezvous::RendezvousEvent;
use super::send::SendEvent;
use super::terminator::TerminatorEvent;

pub enum MachineEvent {
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

pub enum InboundEvent {
    IO(IOEvent),
    API(APIEvent),
}

pub enum Action {
    // outbound
    IO(IOAction),
    API(APIAction),
}

pub enum ProcessEvent {
    API(APIEvent),
    IO(IOEvent),
    Machine(MachineEvent),
}

pub enum ProcessResultEvent {
    API(APIAction),
    IO(IOAction),
    Machine(MachineEvent),
}

/*
process_one_event(inbound api, inbound io, machines) -> (outbound api, outbound io, machines)
execute(inbound api, inbound io) -> (outbound api, outbound io)

*/
