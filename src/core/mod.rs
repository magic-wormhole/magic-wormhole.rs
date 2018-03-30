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
    relay_url: String,
    actions: VecDeque<Action>,
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
        let wsh = WSHandle{};
        // I.. don't know how to copy a String
        let open = Action::WebSocketOpen(wsh, self.relay_url.to_lowercase());
        self.actions.push_back(open);
    }
}


pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let mut action_queue = VecDeque::new();
    // we use a handle here just in case we need to open multiple connections
    // in the future. For now we ignore it, but the IO layer is supposed to
    // pass this back in websocket_* messages
    let wsh = WSHandle{};
    action_queue.push_back(Action::WebSocketOpen(wsh, String::from(relay_url)));

    WormholeCore{
        appid: String::from(appid),
        relay_url: String::from(relay_url),
        actions: action_queue,
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
