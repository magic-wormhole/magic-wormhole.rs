// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use serde_json;
use api::{TimerHandle, WSHandle};
use events::Event;
// we handle these
use events::Event::{RC_Start, RC_Stop, RC_TxAdd, RC_TxAllocate, RC_TxClaim,
                    RC_TxClose, RC_TxList, RC_TxOpen, RC_TxRelease};
use events::Event::{IO_TimerExpired, IO_WebSocketConnectionLost,
                    IO_WebSocketConnectionMade, IO_WebSocketMessageReceived};
// we emit these
use events::Event::N_Connected;
use events::Event::{IO_CancelTimer, IO_StartTimer, IO_WebSocketClose,
                    IO_WebSocketOpen, IO_WebSocketSendMessage};
use server_messages::{bind, deserialize, Message};

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
pub fn create(
    appid: &str,
    relay_url: &str,
    side: &str,
    retry_timer: f32,
) -> Rendezvous {
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
    pub fn process(&mut self, e: Event) -> Vec<Event> {
        match e {
            RC_Start => self.start(),
            RC_TxOpen => vec![],
            RC_TxAdd => vec![],
            RC_TxClose => vec![],
            RC_Stop => self.stop(),
            RC_TxClaim => vec![],
            RC_TxRelease => vec![],
            RC_TxAllocate => vec![],
            RC_TxList => vec![],
            IO_WebSocketConnectionMade(wsh) => self.connection_made(wsh),
            IO_WebSocketMessageReceived(wsh, message) => {
                self.message_received(wsh, &message)
            }
            IO_WebSocketConnectionLost(wsh) => self.connection_lost(wsh),
            IO_TimerExpired(th) => self.timer_expired(th),
            _ => panic!(),
        }
    }

    fn start(&mut self) -> Vec<Event> {
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        let actions;
        let newstate = match self.state {
            State::Idle => {
                actions = vec![
                    IO_WebSocketOpen(self.wsh, self.relay_url.to_lowercase()),
                ];
                //"url".to_string());
                State::Connecting
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn connection_made(&mut self, _handle: WSHandle) -> Vec<Event> {
        let mut actions = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting => {
                // TODO: maybe emit TxBind instead, let it loop around?
                let b = bind(&self.appid, &self.side);
                actions.extend(self.send(b));
                // TODO: does the order of the matter? if so, oh boy.
                actions.push(N_Connected);
                //actions.push(A_Connected);
                //actions.push(L_Connected);
                //actions.push(M_Connected);
                State::Connected
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn message_received(
        &mut self,
        _handle: WSHandle,
        message: &str,
    ) -> Vec<Event> {
        let m = deserialize(&message);
        println!("msg is {:?}", m);
        vec![]
    }

    fn connection_lost(&mut self, _handle: WSHandle) -> Vec<Event> {
        let mut actions = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Connecting | State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                actions.push(IO_StartTimer(new_handle, self.retry_timer));
                State::Waiting
            }
            State::Disconnecting => State::Stopped,
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn timer_expired(&mut self, _handle: TimerHandle) -> Vec<Event> {
        let mut actions = Vec::new();
        // TODO: assert handle == self.handle
        let newstate = match self.state {
            State::Waiting => {
                let new_handle = WSHandle::new(2);
                // I.. don't know how to copy a String
                let open =
                    IO_WebSocketOpen(new_handle, self.relay_url.to_lowercase());
                actions.push(open);
                State::Connecting
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn stop(&mut self) -> Vec<Event> {
        let mut actions = Vec::new();
        let newstate = match self.state {
            State::Idle | State::Stopped => State::Stopped,
            State::Connecting | State::Connected => {
                let close = IO_WebSocketClose(self.wsh);
                actions.push(close);
                State::Disconnecting
            }
            State::Waiting => {
                let cancel = IO_CancelTimer(self.reconnect_timer.unwrap());
                actions.push(cancel);
                State::Stopped
            }
            State::Disconnecting => State::Disconnecting,
        };
        self.state = newstate;
        actions
    }

    fn send(&mut self, m: Message) -> Vec<Event> {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let s = IO_WebSocketSendMessage(
            self.wsh,
            serde_json::to_string(&m).unwrap(),
        );
        vec![s]
    }
}

#[cfg(test)]
mod test {
    use server_messages::{deserialize, Message};
    use api::{TimerHandle, WSHandle};
    use events::Event::IO_WebSocketOpen;

    #[test]
    fn create() {
        let mut r = super::create("appid", "url", "side1", 5.0);

        let wsh: WSHandle;
        let th: TimerHandle;

        let mut actions = r.start();
        assert_eq!(actions.len(), 1);
        let e = actions.pop().unwrap();
        // TODO: I want to:
        // * assert that actions[0] is a
        //   RendezvousResult::IO(IOAction::WebSocketOpen())
        // * extract that WebSocketOpen, call it "o"
        // * stash o.0 in "wsh" for later comparisons
        // * assert that o.1 is "url"

        match e {
            IO_WebSocketOpen(wsh0, url0) => {
                wsh = wsh0;
                assert_eq!(url0, "url");
            }
            _ => panic!(),
        }

        /*

        // now we tell it we're connected
        actions = r.process_io_event(IOEvent::WebSocketConnectionMade(wsh));
        // it should send a BIND
        // then it should notify several other machines

        assert_eq!(actions.len(), 2);
        let e = actions.remove(0);
        println!("e is {:?}", e);
        let io = match e {
            RendezvousResult::IO(io) => io,
            _ => panic!(),
        };
        match io {
            IOAction::WebSocketSendMessage(wsh0, m) => {
                assert_eq!(wsh0, wsh);
                if let Message::Bind { appid, side } = deserialize(&m) {
                    assert_eq!(appid, "appid");
                    assert_eq!(side, "side1");
                } else {
                    panic!();
                }
            }
            _ => panic!(),
        }

        let e = actions.remove(0);
        let m = match e {
            RendezvousResult::Machine(MachineEvent::Nameplate(m)) => m,
            _ => panic!(),
        };
        match m {
            NameplateEvent::Connected => {
                // yay
            }
            _ => panic!(),
        }

        //assert_eq!(actions, vec![RendezvousResult::IO(IOAction::WebSocketOpen(wsh, "url".to_string()))]);
        assert_eq!(actions, vec![IOAction::WebSocketOpen(wsh

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
        */
    }
}
