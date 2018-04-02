// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use serde_json;
use api::{TimerHandle, WSHandle, IOAction, IOEvent};
use events::RendezvousResult;
use server_messages::{bind, deserialize, Message};

#[derive(Debug, PartialEq)]
pub enum RendezvousEvent {
    Start,
    TxOpen,
    TxAdd,
    TxClose,
    Stop,
    TxClaim,
    TxRelease,
    TxAllocate,
    TxList,
}

#[derive(Debug, PartialEq)]
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

// TODO: move this to Rendezvous::new()
pub fn create(appid: &str, relay_url: &str, side: &str, retry_timer: f32) -> Rendezvous {
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
    pub fn process_io_event(&mut self, event: IOEvent) -> Vec<RendezvousResult> {
        match event {
            IOEvent::WebSocketConnectionMade(wsh) => self.connection_made(wsh),
            IOEvent::WebSocketMessageReceived(wsh, message) => self.message_received(wsh, &message),
            IOEvent::WebSocketConnectionLost(wsh) => self.connection_lost(wsh),
            IOEvent::TimerExpired(th) => self.timer_expired(th),
        }
    }

    pub fn execute(&mut self, event: RendezvousEvent) -> Vec<RendezvousResult> {
        match event {
            RendezvousEvent::Start => self.start(),
            RendezvousEvent::TxOpen => vec![],
            RendezvousEvent::TxAdd => vec![],
            RendezvousEvent::TxClose => vec![],
            RendezvousEvent::Stop => self.stop(),
            RendezvousEvent::TxClaim => vec![],
            RendezvousEvent::TxRelease => vec![],
            RendezvousEvent::TxAllocate => vec![],
            RendezvousEvent::TxList => vec![],
        }
    }

    fn start(&mut self) -> Vec<RendezvousResult> {
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        let results;
        let newstate = match self.state {
            State::Idle => {
                results = vec![
                    RendezvousResult::IO(IOAction::WebSocketOpen(self.wsh, self.relay_url.to_lowercase())),
                ];
                //"url".to_string());
                State::Connecting
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        results
    }

    fn connection_made(&mut self, _handle: WSHandle) -> Vec<RendezvousResult> {
        let mut results = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting => {
                let b = bind(&self.appid, &self.side);
                results.extend(self.send(b));
                State::Connected
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        results
    }

    fn message_received(&mut self, _handle: WSHandle, message: &str) -> Vec<RendezvousResult> {
        let m = deserialize(&message);
        println!("msg is {:?}", m);
        vec![]
    }

    fn connection_lost(&mut self, _handle: WSHandle) -> Vec<RendezvousResult> {
        let mut results = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting | State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                // I.. don't know how to copy a String
                let wait = IOAction::StartTimer(new_handle, self.retry_timer);
                results.push(RendezvousResult::IO(wait));
                State::Waiting
            }
            State::Disconnecting => State::Stopped,
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        results
    }

    fn timer_expired(&mut self, _handle: TimerHandle) -> Vec<RendezvousResult> {
        let mut results = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Waiting => {
                let new_handle = WSHandle::new(2);
                // I.. don't know how to copy a String
                let open = IOAction::WebSocketOpen(new_handle, self.relay_url.to_lowercase());
                results.push(RendezvousResult::IO(open));
                State::Connecting
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        results
    }

    fn stop(&mut self) -> Vec<RendezvousResult> {
        let mut results = Vec::new();
        let newstate = match self.state {
            State::Idle | State::Stopped => State::Stopped,
            State::Connecting | State::Connected => {
                let close = IOAction::WebSocketClose(self.wsh);
                results.push(RendezvousResult::IO(close));
                State::Disconnecting
            }
            State::Waiting => {
                let cancel = IOAction::CancelTimer(self.reconnect_timer.unwrap());
                results.push(RendezvousResult::IO(cancel));
                State::Stopped
            }
            State::Disconnecting => State::Disconnecting,
        };
        self.state = newstate;
        results
    }

    fn send(&mut self, m: Message) -> Vec<RendezvousResult> {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let s = IOAction::WebSocketSendMessage(self.wsh, serde_json::to_string(&m).unwrap());
        vec![RendezvousResult::IO(s)]
    }
}

/*
#[cfg(test)]
mod test {
    use std::collections::VecDeque;
    use server_messages::{deserialize, Message};
    use super::super::traits::Action;
    use super::super::traits::Action::{StartTimer, WebSocketOpen, WebSocketSendMessage};
    use super::super::traits::{TimerHandle, WSHandle};

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
            }
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() {
            panic!()
        };

        r.connection_made(&mut actions, wsh);
        match actions.pop_front() {
            Some(WebSocketSendMessage(_handle, m)) => {
                //assert_eq!(handle, wsh);
                if let Message::Bind { appid, side } = deserialize(&m) {
                    assert_eq!(appid, "appid");
                    assert_eq!(side, "side1");
                } else {
                    panic!();
                }
            }
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() {
            panic!()
        };

        r.connection_lost(&mut actions, wsh);
        match actions.pop_front() {
            Some(StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            }
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() {
            panic!()
        };

        r.timer_expired(&mut actions, th);
        match actions.pop_front() {
            Some(WebSocketOpen(handle, url)) => {
                assert_eq!(url, "url");
                wsh = handle;
            }
            _ => panic!(),
        }
        if let Some(_) = actions.pop_front() {
            panic!()
        };

        r.stop(&mut actions);
    }
}
*/
