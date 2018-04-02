extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

mod api;
mod events;
mod allocator;
mod boss;
mod code;
mod input;
mod key;
mod lister;
mod mailbox;
mod nameplate;
mod order;
mod receive;
mod rendezvous;
mod server_messages;
mod send;
mod terminator;
mod wordlist;

use std::collections::VecDeque;
use events::{MachineEvent, ProcessEvent, Result};
use rendezvous::RendezvousEvent;
pub use api::{APIAction, APIEvent, Action, IOAction, IOEvent, TimerHandle, WSHandle};

pub struct WormholeCore {
    boss: boss::Boss,
    rendezvous: rendezvous::Rendezvous,
}

pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let side = "side1"; // TODO: generate randomly
    WormholeCore {
        boss: boss::Boss::new(),
        rendezvous: rendezvous::create(appid, relay_url, side, 5.0),
    }
}

// I don't know how to write this
/*fn to_results<Vec<T>>(from: Vec<T>) -> Vec<Result> {
    from.into_iter().map(|r| Result::from(r)).collect::<Vec<Result>>()
}*/

impl WormholeCore {
    pub fn start(&mut self) -> Vec<Action> {
        // TODO: replace with Boss::Start, which will start rendezvous
        self._execute(ProcessEvent::from(RendezvousEvent::Start))
    }

    pub fn do_api(&mut self, event: APIEvent) -> Vec<Action> {
        self._execute(ProcessEvent::from(event))
    }

    pub fn do_io(&mut self, event: IOEvent) -> Vec<Action> {
        self._execute(ProcessEvent::from(event))
    }

    pub fn derive_key(&mut self, _purpose: &str, _length: u8) -> Vec<u8> {
        // TODO: only valid after GotVerifiedKey, but should return
        // synchronously. Maybe the Core should expose the conversion
        // function (which requires the key as input) and let the IO glue
        // layer decide how to manage the synchronization?
        Vec::new()
    }

    fn _execute(&mut self, event: ProcessEvent) -> Vec<Action> {
        let mut action_queue: Vec<Action> = Vec::new(); // returned
        let mut event_queue: VecDeque<ProcessEvent> = VecDeque::new();
        event_queue.push_back(ProcessEvent::from(event));
        while let Some(event) = event_queue.pop_front() {
            // TODO: factor out some of this common stuff. Each of our child
            // functions returns a Vec of types that can be turned into a
            // Result, but I can't find a way to take advantage of that. The
            // arms of the match must have equivalent types, and the closest
            // I can get is to have them all return Iterators (since we don't
            // really need a collection; we're only passing it to "for"
            // below), but I don't know how to explain what type of iterator
            // I want, and between Result::from() and collect() there's too
            // much flexibility for type inferencing to get it right.
            let results: Vec<Result> = match event {
                // customers speak only to the Boss
                ProcessEvent::API(e) => {
                    let x = self.boss.process_api_event(e);
                    x.into_iter().map(|r| Result::from(r)).collect()
                }
                // Rendezvous is currently the only machine that does IO. If
                // this changes (e.g. something else in the protocol wants to
                // use a timer), we'll need a registration table, and this
                // will need to dispatch according to the handle
                ProcessEvent::IO(e) => {
                    let x = self.rendezvous.process_io_event(e);
                    x.into_iter().map(|r| Result::from(r)).collect()
                }
                // All other machines consume and emit only MachineEvents
                ProcessEvent::Machine(e) => self.process_machine_event(e),
            };
            for r in results {
                match r {
                    Result::API(e) => action_queue.push(Action::API(e)),
                    Result::IO(e) => action_queue.push(Action::IO(e)),
                    Result::Machine(e) => event_queue.push_back(ProcessEvent::Machine(e)),
                }
            }
        }
        action_queue
    }

    fn process_machine_event(&mut self, event: MachineEvent) -> Vec<Result> {
        // Most machines have only the .execute() method, which only accepts
        // a MachineEvent. The Boss can accept APIEvents through
        // .process_api_event(), and Rendezvous can accept IOEvents through
        // .process_io_events.

        // For most machines, .execute() returns a vector of MachineEvents.
        // Boss emits a Vec<BossResult> (which includes APIActions), and
        // Rendezvous emits Vec<RendezvousResult> (which includes IOAction).
        // All three must be merged into the Vec<Result> that we return.

        match event {
            //Allocator(e) => self.allocator.execute(e),
            //Boss(e) => self.boss.execute(e),
            //Code(e) => self.code.execute(e),
            //Input(e) => self.input.execute(e),
            //Key(e) => self.key.execute(e),
            //Lister(e) => self.lister.execute(e),
            //Mailbox(e) => self.mailbox.execute(e),
            //Nameplate(e) => self.nameplate.execute(e),
            //Order(e) => self.order.execute(e),
            //Receive(e) => self.receive.execute(e),
            MachineEvent::Rendezvous(e) => self.rendezvous
                .execute(e)
                .into_iter()
                .map(|r| Result::from(r))
                .collect(),
            //Send(e) => self.send.execute(e),
            //Terminator(e) => self.terminator.execute(e),
        }
    }
}

/*
#[cfg(test)]
mod test {
    use super::create_core;
    use super::traits::Core;
    use super::traits::Action::{StartTimer, WebSocketOpen, WebSocketSendMessage};
    use super::traits::{TimerHandle, WSHandle};
    use serde_json;
    use serde_json::Value;

    #[test]
    fn create() {
        let mut w = create_core("appid", "url");
        let mut wsh: WSHandle;
        let th: TimerHandle;

        match w.get_action() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            }
            _ => panic!(),
        }
        if let Some(_) = w.get_action() {
            panic!()
        };

        w.websocket_connection_made(wsh);
        match w.get_action() {
            Some(WebSocketSendMessage(_handle, m)) => {
                //assert_eq!(handle, wsh);
                let b: Value = serde_json::from_str(&m).unwrap();
                assert_eq!(b["type"], "bind");
            }
            _ => panic!(),
        }
        if let Some(_) = w.get_action() {
            panic!()
        };

        w.websocket_connection_lost(wsh);
        match w.get_action() {
            Some(StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            }
            _ => panic!(),
        }
        if let Some(_) = w.get_action() {
            panic!()
        };

        w.timer_expired(th);
        match w.get_action() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            }
            _ => panic!(),
        }
        if let Some(_) = w.get_action() {
            panic!()
        };
    }
}
*/

// TODO: is there a generic way (e.g. impl From) to convert a Vec<A> into
// Vec<B> when we've got an A->B convertor?
