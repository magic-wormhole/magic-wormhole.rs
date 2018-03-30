mod traits;
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
mod send;
mod terminator;
mod wordlist;

use std::collections::VecDeque;
use core::traits::{Action, WSHandle, TimerHandle};

pub struct WormholeCore {
    appid: String,
    rendezvous: rendezvous::Rendezvous,
    actions: VecDeque<Action>,
}

pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let mut action_queue = VecDeque::new();

    let mut wc = WormholeCore{
        rendezvous: rendezvous::create(relay_url, 5.0),
        appid: String::from(appid),
        actions: action_queue,
    };
    wc.rendezvous.start(&mut wc.actions);
    wc
}

impl traits::Core for WormholeCore {
    fn allocate_code(&mut self) -> () {}
    fn set_code(&mut self, code: &str) -> () {}
    fn derive_key(&mut self, purpose: &str, length: u8) -> Vec<u8> {
        Vec::new()
    }
    fn close(&mut self) -> () {}

    fn get_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    fn timer_expired(&mut self, handle: TimerHandle) -> () {
    }

    fn websocket_connection_made(&mut self, handle: WSHandle) -> () {
    }
    fn websocket_message_received(&mut self, handle: WSHandle, message: &Vec<u8>) -> () {
    }
    fn websocket_connection_lost(&mut self, handle: WSHandle) -> () {
        self.rendezvous.connection_lost(&mut self.actions, handle);
    }
}



#[cfg(test)]
mod test {
    use core::create_core;
    use core::traits::Core;
    use core::traits::Action::WebSocketOpen;
    #[test]
    fn create() {
        let mut w = create_core("appid", "url");
        match w.get_action() {
            Some(WebSocketOpen(_, url)) => assert_eq!(url, "url"),
            _ => assert!(false),
        }
        match w.get_action() {
            None => (),
            _ => assert!(false),
        }
    }
}
