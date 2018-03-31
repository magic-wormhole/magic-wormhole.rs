// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use std::collections::VecDeque;
use core::traits::{TimerHandle, WSHandle, Action};

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
    wsh: WSHandle,
    relay_url: String,
    retry_timer: f32,
    state: State,
    connected_at_least_once: bool,
    reconnect_timer: Option<TimerHandle>,
}

pub fn create(relay_url: &str, retry_timer: f32) -> Rendezvous {
    // we use a handle here just in case we need to open multiple connections
    // in the future. For now we ignore it, but the IO layer is supposed to
    // pass this back in websocket_* messages
    let wsh = WSHandle::new(1);
    Rendezvous {
        relay_url: relay_url.to_string(),
        wsh: wsh,
        retry_timer: retry_timer,
        state: State::Idle,
        connected_at_least_once: false,
        reconnect_timer: None,
    }
}

impl Rendezvous {
    pub fn start(&mut self, actions: &mut VecDeque<Action>) -> () {
        let newstate: State;
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        match self.state {
            State::Idle => {
                let open = Action::WebSocketOpen(self.wsh,
                                                 self.relay_url.to_lowercase());
                //"url".to_string());
                actions.push_back(open);
                newstate = State::Connecting;
            },
            _ => panic!("bad transition from {:?}", self),
        }
        self.state = newstate;
    }

    pub fn connection_made(&mut self,
                           actions: &mut VecDeque<Action>,
                           handle: WSHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate: State;
        match self.state {
            State::Connecting => {
                // TODO: send BIND
                newstate = State::Connected;
            },
            _ => panic!("bad transition from {:?}", self),
        }
        self.state = newstate;
    }

    pub fn connection_lost(&mut self,
                           actions: &mut VecDeque<Action>,
                           handle: WSHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate: State;
        match self.state {
            State::Connecting | State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                // I.. don't know how to copy a String
                let wait = Action::StartTimer(new_handle, self.retry_timer);
                actions.push_back(wait);
                newstate = State::Waiting;
            },
            State::Disconnecting => {
                newstate = State::Stopped;
            },
            _ => panic!("bad transition from {:?}", self),
        }
        self.state = newstate;
    }

    pub fn timer_expired(&mut self,
                         actions: &mut VecDeque<Action>,
                         handle: TimerHandle) -> () {
        // TODO: assert handle == self.handle
        let newstate: State;
        match self.state {
            State::Waiting => {
                let new_handle = WSHandle::new(2);
                // I.. don't know how to copy a String
                let open = Action::WebSocketOpen(new_handle,
                                                 self.relay_url.to_lowercase());
                actions.push_back(open);
                newstate = State::Connecting;
            },
            _ => panic!("bad transition from {:?}", self),
        }
        self.state = newstate;
    }

    pub fn stop(&mut self,
                actions: &mut VecDeque<Action>) -> () {
        let newstate: State;
        match self.state {
            State::Idle | State::Stopped => {
                newstate = State::Stopped;
            },
            State::Connecting | State::Connected => {
                let close = Action::WebSocketClose(self.wsh);
                actions.push_back(close);
                newstate = State::Disconnecting;
            },
            State::Waiting => {
                let cancel = Action::CancelTimer(self.reconnect_timer.unwrap());
                actions.push_back(cancel);
                newstate = State::Stopped;
            },
            State::Disconnecting => {
                newstate = State::Disconnecting;
            },
            _ => panic!("bad transition from {:?}", self),
        }
        self.state = newstate;
    }

}
