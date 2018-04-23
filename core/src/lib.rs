extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
mod events;
extern crate hkdf;
extern crate sha2;
extern crate sodiumoxide;
extern crate spake2;

mod api;
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
mod util;

use std::collections::VecDeque;
use events::{Event, Events};
pub use api::{APIAction, APIEvent, Action, IOAction, IOEvent, TimerHandle,
              WSHandle};

pub struct WormholeCore {
    allocator: allocator::Allocator,
    boss: boss::Boss,
    code: code::Code,
    input: input::Input,
    key: key::Key,
    lister: lister::Lister,
    mailbox: mailbox::Mailbox,
    nameplate: nameplate::Nameplate,
    order: order::Order,
    receive: receive::Receive,
    rendezvous: rendezvous::Rendezvous,
    send: send::Send,
    terminator: terminator::Terminator,
}

// I don't know how to write this
/*fn to_results<Vec<T>>(from: Vec<T>) -> Vec<Result> {
    from.into_iter().map(|r| Result::from(r)).collect::<Vec<Result>>()
}*/

impl WormholeCore {
    pub fn new(appid: &str, relay_url: &str) -> WormholeCore {
        let side = "side1"; // TODO: generate randomly
        WormholeCore {
            allocator: allocator::Allocator::new(),
            boss: boss::Boss::new(),
            code: code::Code::new(),
            input: input::Input::new(),
            key: key::Key::new(appid, side),
            lister: lister::Lister::new(),
            mailbox: mailbox::Mailbox::new(&side),
            nameplate: nameplate::Nameplate::new(),
            order: order::Order::new(),
            receive: receive::Receive::new(),
            rendezvous: rendezvous::Rendezvous::new(
                appid,
                relay_url,
                side,
                5.0,
            ),
            send: send::Send::new(side),
            terminator: terminator::Terminator::new(),
        }
    }

    pub fn start(&mut self) -> Vec<Action> {
        // TODO: replace with Boss::Start, which will start rendezvous
        self._execute(events![events::RendezvousEvent::Start])
    }

    pub fn do_api(&mut self, event: APIEvent) -> Vec<Action> {
        let events = self.boss.process_api(event);
        self._execute(events)
    }

    pub fn do_io(&mut self, event: IOEvent) -> Vec<Action> {
        let events = self.rendezvous.process_io(event);
        self._execute(events)
    }

    pub fn derive_key(&mut self, _purpose: &str, _length: u8) -> Vec<u8> {
        // TODO: only valid after GotVerifiedKey, but should return
        // synchronously. Maybe the Core should expose the conversion
        // function (which requires the key as input) and let the IO glue
        // layer decide how to manage the synchronization?
        Vec::new()
    }

    fn _execute(&mut self, events: Events) -> Vec<Action> {
        let mut action_queue: Vec<Action> = Vec::new(); // returned
        let mut event_queue: VecDeque<Event> = VecDeque::new();

        event_queue.append(&mut VecDeque::from(events.events));

        while let Some(e) = event_queue.pop_front() {
            println!("event: {:?}", e);
            use events::Event::*; // machine names
            let actions: Events = match e {
                API(a) => {
                    action_queue.push(Action::API(a));
                    events![]
                }
                IO(a) => {
                    action_queue.push(Action::IO(a));
                    events![]
                }
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
                _ => panic!(),
            };

            for a in actions.events {
                // TODO use iter
                // TODO: insert in front of queue: depth-first processing
                println!("  out: {:?}", a);
                event_queue.push_back(a);
            }
        }
        action_queue
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
