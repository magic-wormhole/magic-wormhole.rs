// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use std::collections::VecDeque;
use serde_json;
use super::traits::{TimerHandle, WSHandle, Action};
use server_messages::{bind, Message, deserialize};

#[derive(Debug)]
enum State {
    Idle,
    Connecting,
    Connected,
    Waiting,
    Disconnecting, // -> Stopped
    Stopped,
}

#[derive(Debug)]
pub struct Rendezvous {
    appid: String,
    relay_url: String,
    side: String,
    retry_timer: f32,
    state: State,
    connected_at_least_once: bool,
    wsh: WSHandle,
    reconnect_timer: Option<TimerHandle>,
}

pub fn create(appid: &str, relay_url: &str, side: &str,
              retry_timer: f32) -> Rendezvous {
    // we use a handle here just in case we need to open multiple connections
    // in the future. For now we ignore it, but the IO layer is supposed to
    // pass this back in websocket_* messages
    let wsh = WSHandle::new(1);
    Rendezvous {
        appid: appid.to_string(),
        relay_url: relay_url.to_string(),
        side: side.to_string(),
        retry_timer: retry_timer,
        state: State::Idle,
        connected_at_least_once: false,
        wsh: wsh,
        reconnect_timer: None,
    }
}

impl Rendezvous {
    pub fn start(&mut self, actions: &mut VecDeque<Action>) -> () {
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        let newstate = match self.state {
            State::Idle => {
                let open = Action::WebSocketOpen(self.wsh,
                                                 self.relay_url.to_lowercase());
                //"url".to_string());
                actions.push_back(open);
                State::Connecting
            },
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
    }

    pub fn connection_made(&mut self,
                           mut actions: &mut VecDeque<Action>,
                           _handle: WSHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting => {
                let b = bind(&self.appid, &self.side);
                self.send(b, &mut actions);
                State::Connected
            },
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
    }

    pub fn message_received(&mut self,
                            _actions: &mut VecDeque<Action>,
                            _handle: WSHandle,
                            message: &str) -> () {
        let m = deserialize(&message);
        println!("msg is {:?}", m);
    }

    pub fn connection_lost(&mut self,
                           actions: &mut VecDeque<Action>,
                           _handle: WSHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting | State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                // I.. don't know how to copy a String
                let wait = Action::StartTimer(new_handle, self.retry_timer);
                actions.push_back(wait);
                State::Waiting
            },
            State::Disconnecting => {
                State::Stopped
            },
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
    }

    pub fn timer_expired(&mut self,
                         actions: &mut VecDeque<Action>,
                         _handle: TimerHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Waiting => {
                let new_handle = WSHandle::new(2);
                // I.. don't know how to copy a String
                let open = Action::WebSocketOpen(new_handle,
                                                 self.relay_url.to_lowercase());
                actions.push_back(open);
                State::Connecting
            },
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
    }

    pub fn stop(&mut self,
                actions: &mut VecDeque<Action>) -> () {
        let newstate = match self.state {
            State::Idle | State::Stopped => {
                State::Stopped
            },
            State::Connecting | State::Connected => {
                let close = Action::WebSocketClose(self.wsh);
                actions.push_back(close);
                State::Disconnecting
            },
            State::Waiting => {
                let cancel = Action::CancelTimer(self.reconnect_timer.unwrap());
                actions.push_back(cancel);
                State::Stopped
            },
            State::Disconnecting => {
                State::Disconnecting
            },
        };
        self.state = newstate;
    }

    pub fn send(&mut self, m: Message,
                actions: &mut VecDeque<Action>) -> () {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let s = Action::WebSocketSendMessage(self.wsh,
                                             serde_json::to_string(&m).unwrap(),
                                             );
        actions.push_back(s);
    }
}


#[cfg(test)]
mod test {
    use std::collections::VecDeque;
    use server_messages::{deserialize, Message};
    use super::super::traits::Action;
    use super::super::traits::Action::{WebSocketOpen, StartTimer,
                                       WebSocketSendMessage};
    use super::super::traits::{WSHandle, TimerHandle};

    #[test]
    fn create() {
        let mut actions: VecDeque<Action> = VecDeque::new();
        let mut r = super::create("appid", "url", "side1", 5.0);

        let mut wsh: WSHandle;
        let th: TimerHandle;

        r.start(&mut actions);

        match actions.pop_front() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() { panic!() };

        r.connection_made(&mut actions, wsh);
        match actions.pop_front() {
            Some(WebSocketSendMessage(_handle, m)) => {
                //assert_eq!(handle, wsh);
                if let Message::Bind{appid, side} = deserialize(&m) {
                    assert_eq!(appid, "appid");
                    assert_eq!(side, "side1");
                } else {
                    panic!();
                }
            },
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() { panic!() };

        r.connection_lost(&mut actions, wsh);
        match actions.pop_front() {
            Some(StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() { panic!() };

        r.timer_expired(&mut actions, th);
        match actions.pop_front() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            },
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() { panic!() };

        r.stop(&mut actions);

    }
}
