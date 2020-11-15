use futures::channel::mpsc::UnboundedReceiver;
use futures::channel::mpsc::UnboundedSender;
use std::collections::VecDeque;

#[macro_use]
mod events;
mod allocator;
mod api;
mod boss;
mod code;
mod io;
pub mod key;
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
mod util;
mod wordlist;

pub use self::events::{AppID, Code};
use self::events::{Event, Events, MySide};
use log::*;

pub use self::api::{APIAction, APIEvent, IOAction, IOEvent, Mood, WSHandle};

/// Set up a WormholeCore and run it
///
/// This will create a new WormholeCore, connect its IO and API interfaces together
/// and spawn a new task that runs the event loop. A channel pair to make API calls is returned.
pub fn run(
    appid: AppID,
    versions: serde_json::Value,
    relay_url: &str,
    #[cfg(test)] eventloop_task: &mut Option<async_std::task::JoinHandle<()>>,
) -> (UnboundedSender<APIEvent>, UnboundedReceiver<APIAction>) {
    use futures::channel::mpsc::unbounded;
    use futures::SinkExt;
    use futures::StreamExt;

    let (tx_io_to_core, mut rx_io_to_core) = unbounded();
    let (tx_api_to_core, mut rx_api_to_core) = unbounded();
    let (mut tx_api_from_core, rx_api_from_core) = unbounded();
    let mut core = WormholeCore::new(appid, versions, relay_url, tx_io_to_core);

    #[allow(unused_variables)]
    let join_handle = async_std::task::spawn(async move {
        'outer: loop {
            let actions = futures::select! {
                action = rx_api_to_core.next() => {
                    debug!("Doing API {:?}", action);
                    core.do_api(action.unwrap_or(APIEvent::Close))
                },
                action = rx_io_to_core.select_next_some() => {
                    debug!("Doing IO {:?}", action);
                    match core.do_io(action) {
                        Ok(events) => events,
                        Err(e) => {
                            // TODO propagate that error to the outside
                            log::error!("Got error from core: {}", e);
                            tx_api_from_core.close_channel();
                            rx_api_to_core.close();
                            rx_io_to_core.close();
                            break 'outer;
                        },
                    }
                },
            };
            debug!("Done API/IO {:?}", &actions);
            for action in actions {
                if let APIAction::GotClosed(_) = action {
                    tx_api_from_core.close_channel();
                    debug!("Stopping wormhole event loop");
                    break 'outer;
                } else {
                    tx_api_from_core
                        .send(action)
                        .await
                        .expect("Don't close the receiver before shutting down the wormhole!");
                }
            }
        }
    });
    #[cfg(test)]
    {
        *eventloop_task = Some(join_handle);
    }

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
    key: key::KeyMachine,
    mailbox: mailbox::MailboxMachine,
    nameplate: nameplate::NameplateMachine,
    order: order::OrderMachine,
    receive: receive::ReceiveMachine,
    rendezvous: rendezvous::RendezvousMachine,
    send: send::SendMachine,
    terminator: terminator::TerminatorMachine,
    io: io::WormholeIO,
}

impl WormholeCore {
    fn new(
        appid: AppID,
        versions: serde_json::Value,
        relay_url: &str,
        io_to_core: futures::channel::mpsc::UnboundedSender<IOEvent>,
    ) -> Self {
        let side = MySide::generate();
        WormholeCore {
            allocator: allocator::AllocatorMachine::new(),
            boss: boss::BossMachine::new(),
            code: code::CodeMachine::new(),
            key: key::KeyMachine::new(&appid, &side, versions),
            mailbox: mailbox::MailboxMachine::new(&side),
            nameplate: nameplate::NameplateMachine::new(),
            order: order::OrderMachine::new(),
            receive: receive::ReceiveMachine::new(),
            rendezvous: rendezvous::RendezvousMachine::new(&appid, relay_url, &side, 5.0),
            send: send::SendMachine::new(&side),
            terminator: terminator::TerminatorMachine::new(),
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
    pub fn do_io(&mut self, event: IOEvent) -> anyhow::Result<Vec<APIAction>> {
        trace!("   io: {:?}", event);
        let events = self.rendezvous.process_io(event)?;
        Ok(self._execute(events))
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
                IO(a) => {
                    self.io.process(a);
                    events![]
                },
                Allocator(e) => self.allocator.process(e),
                Boss(e) => self.boss.process(e),
                Code(e) => self.code.process(e),
                Key(e) => self.key.process(e),
                Mailbox(e) => self.mailbox.process(e),
                Nameplate(e) => self.nameplate.process(e),
                Order(e) => self.order.process(e),
                Receive(e) => self.receive.process(e),
                Rendezvous(e) => self.rendezvous.process(e),
                Send(e) => self.send.process(e),
                Terminator(e) => self.terminator.process(e),
            };

            for a in actions.events {
                // TODO use iter
                // TODO: insert in front of queue: depth-first processing
                event_queue.push_back(a);
            }
        }
        action_queue
    }
}
