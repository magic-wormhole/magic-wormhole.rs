extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
extern crate hex;
extern crate hkdf;
extern crate rand;
extern crate regex;
extern crate rustc_serialize;
extern crate sha2;
extern crate sodiumoxide;
extern crate spake2;

#[macro_use]
mod events;
mod allocator;
mod api;
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
mod send;
mod server_messages;
mod terminator;
mod util;
mod wordlist;

use rustc_serialize::hex::ToHex;
use std::collections::VecDeque;

pub use events::{AppID, Code};
use events::{Event, Events, MySide, Nameplate};
use util::random_bytes;

pub use api::{APIAction, APIEvent, Action, IOAction, IOEvent,
              InputHelperError, Mood, TimerHandle, WSHandle};
pub use server_messages::{AnswerType, OfferType, PeerMessage};

pub struct WormholeCore {
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
}

// I don't know how to write this
/*fn to_results<Vec<T>>(from: Vec<T>) -> Vec<Result> {
    from.into_iter().map(|r| Result::from(r)).collect::<Vec<Result>>()
}*/

fn generate_side() -> String {
    let mut bytes: [u8; 5] = [0; 5];
    random_bytes(&mut bytes);
    bytes.to_hex()
}

impl WormholeCore {
    pub fn new<T>(appid: T, relay_url: &str) -> WormholeCore
    where
        T: Into<AppID>,
    {
        let appid: AppID = appid.into();
        let side = MySide(generate_side());
        WormholeCore {
            allocator: allocator::AllocatorMachine::new(),
            boss: boss::BossMachine::new(),
            code: code::CodeMachine::new(),
            input: input::InputMachine::new(),
            key: key::KeyMachine::new(&appid.clone(), &side),
            lister: lister::ListerMachine::new(),
            mailbox: mailbox::MailboxMachine::new(&side),
            nameplate: nameplate::NameplateMachine::new(),
            order: order::OrderMachine::new(),
            receive: receive::ReceiveMachine::new(),
            rendezvous: rendezvous::RendezvousMachine::new(
                &appid.clone(),
                relay_url,
                &side,
                5.0,
            ),
            send: send::SendMachine::new(&side),
            terminator: terminator::TerminatorMachine::new(),
        }
    }

    // the IO layer must either call start() or do_api(APIEvent::Start), and
    // must act upon all the Actions it gets back
    pub fn start(&mut self) -> Vec<Action> {
        self.do_api(APIEvent::Start)
    }

    pub fn do_api(&mut self, event: APIEvent) -> Vec<Action> {
        println!("api: {:?}", event);
        let events = self.boss.process_api(event);
        self._execute(events)
    }

    pub fn do_io(&mut self, event: IOEvent) -> Vec<Action> {
        println!("io: {:?}", event);
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

    pub fn input_helper_get_nameplate_completions(
        &mut self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        self.input.get_nameplate_completions(prefix)
    }

    pub fn input_helper_get_word_completions(
        &mut self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        self.input.get_word_completions(prefix)
    }

    // TODO: remove this, the helper should remember whether it's called
    // choose_nameplate yet or not instead of asking the core
    pub fn input_helper_committed_nameplate(&self) -> Option<&Nameplate> {
        self.input.committed_nameplate()
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
