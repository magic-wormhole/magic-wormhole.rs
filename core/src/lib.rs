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
//use self::traits::{Action, WSHandle, TimerHandle};
pub use self::traits::*;

pub struct WormholeCore {
    rendezvous: rendezvous::Rendezvous,
    actions: VecDeque<Action>,
}

pub fn create_core(appid: &str, relay_url: &str) -> WormholeCore {
    let action_queue = VecDeque::new();

    let mut wc = WormholeCore{
        rendezvous: rendezvous::create(appid, relay_url, 5.0),
        actions: action_queue,
    };
    wc.rendezvous.start(&mut wc.actions);
    wc
}

impl traits::Core for WormholeCore {
    fn allocate_code(&mut self) -> () {}
    fn set_code(&mut self, _code: &str) -> () {}
    fn derive_key(&mut self, _purpose: &str, _length: u8) -> Vec<u8> {
        Vec::new()
    }
    fn close(&mut self) -> () {}

    fn get_action(&mut self) -> Option<Action> {
        self.actions.pop_front()
    }

    fn timer_expired(&mut self, handle: TimerHandle) -> () {
        // TODO: dispatch to whatever is waiting for this particular timer.
        // Maybe TimerHandle should be an enum of different sub-machines.
        self.rendezvous.timer_expired(&mut self.actions, handle);
    }

    fn websocket_connection_made(&mut self, handle: WSHandle) -> () {
        self.rendezvous.connection_made(&mut self.actions, handle);
    }
    fn websocket_message_received(&mut self, _handle: WSHandle, _message: &str) -> () {
    }
    fn websocket_connection_lost(&mut self, handle: WSHandle) -> () {
        self.rendezvous.connection_lost(&mut self.actions, handle);
    }
}



#[cfg(test)]
mod test {
    use super::create_core;
    use super::traits::Core;
    use super::traits::Action::{WebSocketOpen, StartTimer,
                                WebSocketSendMessage};
    use super::traits::{WSHandle, TimerHandle};

    #[test]
    fn create() {
        let mut w = create_core("appid", "url");
        let mut wsh: WSHandle;
        let mut th: TimerHandle;

        match w.get_action() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = w.get_action() { panic!() };

        w.websocket_connection_made(wsh);
        match w.get_action() {
            Some(WebSocketSendMessage(handle, m)) => {
                //assert_eq!(handle, wsh);
                assert_eq!(m, r#"{"type": "bind", "appid": "appid", "side": "side1"}"#);
            },
            _ => panic!(),
        }
        if let Some(_) = w.get_action() { panic!() };

        w.websocket_connection_lost(wsh);
        match w.get_action() {
            Some(StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = w.get_action() { panic!() };

        w.timer_expired(th);
        match w.get_action() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = w.get_action() { panic!() };
        
    }
}
