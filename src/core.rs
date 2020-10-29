use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use std::collections::VecDeque;

#[macro_use]
mod events;
mod allocator;
mod api;
mod boss;
mod code;
mod input;
pub mod key;
mod lister;
mod mailbox;
mod nameplate;
mod order;
mod receive;
mod rendezvous;
mod send;
mod server_messages;
mod terminator;
#[cfg(test)]
mod test;
mod timing;
mod transfer;
mod util;
mod wordlist;
mod io;

pub use self::events::{AppID, Code};
use self::events::{Event, Events, MySide};
use log::*;

pub use self::api::{
    APIAction, APIEvent, IOAction, IOEvent, InputHelperError, Mood,
    TimerHandle, WSHandle,
};
pub use self::transfer::{
    Abilities, AnswerType,
    DirectType, Hints, OfferType, PeerMessage, RelayType, TransitType,
    TransitAck,
};

/// Set up a WormholeCore and run it
/// 
/// This will create a new WormholeCore, connect its IO and API interfaces together
/// and spawn a new task that runs the event loop. A channel pair to make API calls is returned.
pub fn run(appid: &str, relay_url: &str) ->
    (UnboundedSender<APIEvent>, UnboundedReceiver<APIAction>)
{
    use futures::channel::mpsc::unbounded;
    use futures::StreamExt;
    use futures::SinkExt;

    let (tx_io_to_core, mut rx_io_to_core) = unbounded();
    let (tx_api_to_core, mut rx_api_to_core) = unbounded();
    let (mut tx_api_from_core, rx_api_from_core) = unbounded();
    let mut core = WormholeCore::new(appid, relay_url, tx_io_to_core);

    async_std::task::spawn(async move {
        loop {
            let actions = futures::select! {
                action = rx_api_to_core.select_next_some() => {
                    debug!("Doing API {:?}", action);
                    core.do_api(action)
                },
                action = rx_io_to_core.select_next_some() => {
                    debug!("Doing IO {:?}", action);
                    core.do_io(action)
                },
            };
            debug!("Done API/IO {:?}", &actions);
            for action in actions {
                tx_api_from_core.send(action).await.unwrap();
            }
        }
    });

    (tx_api_to_core, rx_api_from_core)
}

/// The core implementation of the protocol(s)
///
/// This is a big composite state machine that implements the Client-Server and Client-Client protocols
/// in a rather weird way. All state machines communicate with each other by sending events and actions around
/// like crazy. The wormhole is driven by processing APIActions that generate APIEvents.
/// 
/// Due to the inherent asynchronous nature of IO together with these synchronous blocking state machines, generated IOEvents
/// are sent to a channel. The holder of the struct must then take care of letting the core process these by calling `do_io`.
struct WormholeCore {
    allocator: allocator::AllocatorMachine,
    boss: boss::BossMachine,
    code: code::CodeMachine,
    input: input::InputMachine,
    key: key::KeyMachine,
    lister: lister::ListerMachine,
    mailbox: mailbox::MailboxMachine,
    nameplate: nameplate::NameplateMachine,
    order: order::OrderMachine,
    receive: receive::ReceiveMachine,
    rendezvous: rendezvous::RendezvousMachine,
    send: send::SendMachine,
    terminator: terminator::TerminatorMachine,
    timing: timing::Timing,
    io: io::WormholeIO,
}

impl WormholeCore {
    fn new<T>(appid: T, relay_url: &str, io_to_core: futures::channel::mpsc::UnboundedSender<IOEvent>) -> Self
    where
        T: Into<AppID>,
    {
        let appid: AppID = appid.into();
        let side = MySide::generate();
        WormholeCore {
            allocator: allocator::AllocatorMachine::new(),
            boss: boss::BossMachine::new(),
            code: code::CodeMachine::new(),
            input: input::InputMachine::new(),
            key: key::KeyMachine::new(&appid, &side),
            lister: lister::ListerMachine::new(),
            mailbox: mailbox::MailboxMachine::new(&side),
            nameplate: nameplate::NameplateMachine::new(),
            order: order::OrderMachine::new(),
            receive: receive::ReceiveMachine::new(),
            rendezvous: rendezvous::RendezvousMachine::new(
                &appid, relay_url, &side, 5.0,
            ),
            send: send::SendMachine::new(&side),
            terminator: terminator::TerminatorMachine::new(),
            timing: timing::Timing::new(),
            io: io::WormholeIO::new(io_to_core),
        }
    }

    #[must_use = "You must execute these actions to make things work"]
    pub fn do_api(&mut self, event: APIEvent) -> Vec<APIAction> {
        // run with RUST_LOG=magic_wormhole=trace to see these
        trace!("  api: {:?}", event);
        let events = self.boss.process_api(event);
        self._execute(events)
    }

    #[must_use = "You must execute these actions to make things work"]
    pub fn do_io(&mut self, event: IOEvent) -> Vec<APIAction> {
        trace!("   io: {:?}", event);
        let events = self.rendezvous.process_io(event);
        self._execute(events)
    }

    fn _execute(&mut self, events: Events) -> Vec<APIAction> {
        let mut action_queue: Vec<APIAction> = Vec::new(); // returned
        let mut event_queue: VecDeque<Event> = VecDeque::new();

        event_queue.append(&mut VecDeque::from(events.events));

        while let Some(e) = event_queue.pop_front() {
            trace!("event: {:?}", e);
            use self::events::Event::*; // machine names
            let actions: Events = match e {
                API(a) => {
                    action_queue.push(a);
                    events![]
                },
                IO(a) => {self.io.process(a); events![]},
                Allocator(e) => self.allocator.process(e),
                Boss(e) => self.boss.process(e),
                Code(e) => self.code.process(e),
                Input(e) => self.input.process(e),
                Key(e) => self.key.process(e),
                Lister(e) => self.lister.process(e),
                Mailbox(e) => self.mailbox.process(e),
                Nameplate(e) => self.nameplate.process(e),
                Order(e) => self.order.process(e),
                Receive(e) => self.receive.process(e),
                Rendezvous(e) => self.rendezvous.process(e),
                Send(e) => self.send.process(e),
                Terminator(e) => self.terminator.process(e),
                Timing(_) => events![], // TODO: unimplemented
            };

            for a in actions.events {
                // TODO use iter
                // TODO: insert in front of queue: depth-first processing
                trace!("  out: {:?}", a);
                match a {
                    Timing(e) => self.timing.add(e),
                    _ => event_queue.push_back(a),
                }
            }
        }
        action_queue
    }
}
