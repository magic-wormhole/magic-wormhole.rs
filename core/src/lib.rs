extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

mod traits;
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
//use self::traits::{Action, WSHandle, TimerHandle};
pub use self::traits::*;
use self::events::Event;

pub struct WormholeCore {
    rendezvous: rendezvous::Rendezvous,
}

pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let side = "side1"; // TODO: generate randomly

    let mut wc = WormholeCore {
        rendezvous: rendezvous::create(appid, relay_url, side, 5.0),
    };
    wc
}

impl traits::Core for WormholeCore {
    fn start(&mut self) -> Vec<Action> {
        // TODO: Boss::Start
        self.execute(vec![RendezvousEvent::Start])
    }

    fn execute(&mut self, event: InboundEvent) -> Vec<Action> {
        let action_queue: Vec<Action> = Vec::new(); // returned
        let event_queue: VecDeque<ProcessEvent> = VecDeque::new();
        event_queue.push_back(event); // TODO: might need to map
        while let Some(e) = event_queue.pop_front() {
            let new_actions: Vec<ProcessResultEvent> = match event {
                API(e) => self.process_api_event(e),
                IO(e) => self.process_io_event(e),
                Machine(e) => self.process_machine_event(e),
            };
            for new_action in new_action {
                match new_action {
                    API(e) => action_queue.push(API(e)),
                    IO(e) => action_queue.push(IO(e)),
                    Machine(e) => event_queue.push_back(Machine(e)),
                }
            }
        }
        action_queue
    }

    fn derive_key(&mut self, _purpose: &str, _length: u8) -> Vec<u8> {
        Vec::new()
    }
}

impl WormholeCore {
    fn process_api_event(&mut self, event: APIEvent) -> Vec<ProcessResultEvent> {
        match event {
            AllocateCode => vec![],
            SetCode(code) => vec![],
            Close => self.rendezvous.stop(), // eventually signals GotClosed
            Send => vec![],
        }
    }

    fn process_io_event(&mut self, event: IOEvent) -> Vec<ProcessResultEvent> {
        match event {
            // TODO: dispatch to whatever is waiting for this particular
            // timer. Maybep TimerHandle should be an enum of different
            // sub-machines.
            TimerExpired
            | WebSocketConnectionMade
            | WebSocketMessageReceived
            | WebSocketConnectionLost => self.rendezvous.process_io_event(event),
        }
    }

    fn process_machine_event(&mut self, event: MachineEvent) -> Vec<ProcessResultEvent> {
        match event {
            Allocator(e) => self.allocator.execute(e),
            Boss(e) => self.boss.execute(e),
            Code(e) => self.code.execute(e),
            Input(e) => self.input.execute(e),
            Key(e) => self.key.execute(e),
            Lister(e) => self.lister.execute(e),
            Mailbox(e) => self.mailbox.execute(e),
            Nameplate(e) => self.nameplate.execute(e),
            Order(e) => self.order.execute(e),
            Receive(e) => self.receive.execute(e),
            Rendezvous(e) => self.rendezvous.execute(e),
            Send(e) => self.send.execute(e),
            Terminator(e) => self.terminator.execute(e),
        }
    }
}

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
