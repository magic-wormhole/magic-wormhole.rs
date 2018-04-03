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
use events::{machine_for_event, Event};
pub use api::{APIAction, APIEvent, Action, IOAction, IOEvent, TimerHandle,
              WSHandle};
use events::Event::RC_Start;

pub struct WormholeCore {
    boss: boss::Boss,
    rendezvous: rendezvous::Rendezvous,
    nameplate: nameplate::Nameplate,
}

pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let side = "side1"; // TODO: generate randomly
    WormholeCore {
        boss: boss::Boss::new(),
        rendezvous: rendezvous::create(appid, relay_url, side, 5.0),
        nameplate: nameplate::new(),
    }
}

// I don't know how to write this
/*fn to_results<Vec<T>>(from: Vec<T>) -> Vec<Result> {
    from.into_iter().map(|r| Result::from(r)).collect::<Vec<Result>>()
}*/

impl WormholeCore {
    pub fn start(&mut self) -> Vec<Action> {
        // TODO: replace with Boss::Start, which will start rendezvous
        self.execute(RC_Start)
    }

    pub fn do_api(&mut self, event: APIEvent) -> Vec<Action> {
        self.execute(Event::from(event))
    }

    pub fn do_io(&mut self, event: IOEvent) -> Vec<Action> {
        self.execute(Event::from(event))
    }

    pub fn derive_key(&mut self, _purpose: &str, _length: u8) -> Vec<u8> {
        // TODO: only valid after GotVerifiedKey, but should return
        // synchronously. Maybe the Core should expose the conversion
        // function (which requires the key as input) and let the IO glue
        // layer decide how to manage the synchronization?
        Vec::new()
    }

    fn execute(&mut self, event: Event) -> Vec<Action> {
        let mut action_queue: Vec<Action> = Vec::new(); // returned
        let mut event_queue: VecDeque<Event> = VecDeque::new();
        event_queue.push_back(event);

        while let Some(e) = event_queue.pop_front() {
            let machine = machine_for_event(&e);
            use events::Machine::*;
            let actions: Vec<Event> = match machine {
                API_Action => {
                    action_queue.push(Action::API(APIAction::from(e)));
                    vec![]
                }
                IO_Action => {
                    action_queue.push(Action::IO(IOAction::from(e)));
                    vec![]
                }
                //Allocator => self.allocator.process(e),
                //Boss => self.boss.process(e),
                //Code => self.code.process(e),
                //Input => self.input.process(e),
                //Key => self.key.process(e),
                //Lister => self.lister.process(e),
                //Mailbox => self.mailbox.process(e),
                Nameplate => self.nameplate.process(e),
                //Order => self.order.process(e),
                //Receive => self.receive.process(e),
                Rendezvous => self.rendezvous.process(e),
                //Send => self.send.process(e),
                //Terminator => self.terminator.process(e),
                _ => panic!(),
            };

            for a in actions {
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
