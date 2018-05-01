// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

extern crate hex;

use serde_json;
use api::{TimerHandle, WSHandle};
use events::Events;
use server_messages::{add, allocate, bind, claim, deserialize, list, open,
                      Message};
// we process these
use events::RendezvousEvent;
use api::IOEvent;
// we emit these
use api::IOAction;
use events::NameplateEvent::{Connected as N_Connected,
                             RxClaimed as N_RxClaimed};
use events::AllocatorEvent::{Connected as A_Connected,
                             RxAllocated as A_RxAllocated};
use events::MailboxEvent::{Connected as M_Connected, RxMessage as M_RxMessage};
use events::ListerEvent::Connected as L_Connected;
use events::RendezvousEvent::TxBind as RC_TxBind; // loops around

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

impl Rendezvous {
    pub fn new(
        appid: &str,
        relay_url: &str,
        side: &str,
        retry_timer: f32,
    ) -> Rendezvous {
        // we use a handle here just in case we need to open multiple
        // connections in the future. For now we ignore it, but the IO layer
        // is supposed to pass this back in websocket_* messages
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

    pub fn process_io(&mut self, event: IOEvent) -> Events {
        use api::IOEvent::*;
        match event {
            WebSocketConnectionMade(wsh) => self.connection_made(wsh),
            WebSocketMessageReceived(wsh, message) => {
                self.message_received(wsh, &message)
            }
            WebSocketConnectionLost(wsh) => self.connection_lost(wsh),
            TimerExpired(th) => self.timer_expired(th),
        }
    }

    pub fn process(&mut self, e: RendezvousEvent) -> Events {
        use events::RendezvousEvent::*;
        println!("rendezvous: {:?}", e);
        match e {
            Start => self.start(),
            TxBind(appid, side) => self.send(bind(&appid, &side)),
            TxOpen(mailbox) => self.send(open(&mailbox)),
            TxAdd(phase, body) => self.send(add(&phase, &body)),
            TxClose => events![],
            Stop => self.stop(),
            TxClaim(nameplate) => self.send(claim(&nameplate)),
            TxRelease(_nameplate) => events![],
            TxAllocate => self.send(allocate()),
            TxList => self.send(list()),
        }
    }

    fn start(&mut self) -> Events {
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        let actions;
        let newstate = match self.state {
            State::Idle => {
                actions = events![
                    IOAction::WebSocketOpen(self.wsh, self.relay_url.clone())
                ];
                //"url".to_string());
                State::Connecting
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn connection_made(&mut self, _handle: WSHandle) -> Events {
        // TODO: assert handle == self.handle
        let (actions, newstate) = match self.state {
            State::Connecting => {
                // TODO: does the order of this matter? if so, oh boy.
                let a = events![
                    RC_TxBind(self.appid.to_string(), self.side.to_string()),
                    N_Connected,
                    M_Connected,
                    L_Connected,
                    A_Connected
                ];
                //actions.push(A_Connected);
                //actions.push(L_Connected);
                (a, State::Connected)
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn message_received(&mut self, _handle: WSHandle, message: &str) -> Events {
        println!("msg is {:?}", message);
        let m = deserialize(message);
        match m {
            Message::Claimed { mailbox } => {
                events![N_RxClaimed(mailbox.to_string())]
            }
            Message::Message {
                side,
                phase,
                body,
                //id,
            } => events![M_RxMessage(side, phase, hex::decode(body).unwrap())],
            Message::Allocated { nameplate } => {
                events![A_RxAllocated(nameplate)]
            }
            _ => events![], // TODO
        }
    }

    fn connection_lost(&mut self, _handle: WSHandle) -> Events {
        // TODO: assert handle == self.handle
        let (actions, newstate) = match self.state {
            State::Connecting | State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                (
                    events![IOAction::StartTimer(new_handle, self.retry_timer)],
                    State::Waiting,
                )
            }
            State::Disconnecting => (events![], State::Stopped),
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn timer_expired(&mut self, _handle: TimerHandle) -> Events {
        // TODO: assert handle == self.handle
        let (actions, newstate) = match self.state {
            State::Waiting => {
                let new_handle = WSHandle::new(2);
                let open =
                    IOAction::WebSocketOpen(new_handle, self.relay_url.clone());
                (events![open], State::Connecting)
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn stop(&mut self) -> Events {
        let (actions, newstate) = match self.state {
            State::Idle | State::Stopped => (events![], State::Stopped),
            State::Connecting | State::Connected => {
                let close = IOAction::WebSocketClose(self.wsh);
                (events![close], State::Disconnecting)
            }
            State::Waiting => {
                let cancel =
                    IOAction::CancelTimer(self.reconnect_timer.unwrap());
                (events![cancel], State::Stopped)
            }
            State::Disconnecting => (events![], State::Disconnecting),
        };
        self.state = newstate;
        actions
    }

    fn send(&mut self, m: Message) -> Events {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let s = IOAction::WebSocketSendMessage(
            self.wsh,
            serde_json::to_string(&m).unwrap(),
        );
        events![s]
    }
}

#[cfg(test)]
mod test {
    use server_messages::{deserialize, Message};
    use api::{TimerHandle, WSHandle};
    use events::Event::{Nameplate, Rendezvous, API, IO};
    use api::IOAction;
    use api::IOEvent;
    use events::RendezvousEvent::{Stop as RC_Stop, TxBind as RC_TxBind};
    use events::NameplateEvent::Connected as N_Connected;

    #[test]
    fn create() {
        let mut r = super::Rendezvous::new("appid", "url", "side1", 5.0);

        let wsh: WSHandle;
        let th: TimerHandle;

        let mut actions = r.start().events;
        assert_eq!(actions.len(), 1);
        let e = actions.pop().unwrap();
        // TODO: I want to:
        // * assert that actions[0] is a
        //   RendezvousResult::IO(IOAction::WebSocketOpen())
        // * extract that WebSocketOpen, call it "o"
        // * stash o.0 in "wsh" for later comparisons
        // * assert that o.1 is "url"

        match e {
            IO(IOAction::WebSocketOpen(wsh0, url0)) => {
                wsh = wsh0;
                assert_eq!(url0, "url");
            }
            _ => panic!(),
        }

        // now we tell it we're connected
        actions = r.process_io(IOEvent::WebSocketConnectionMade(wsh)).events;
        // it should tell itself to send a BIND then it should notify several
        // other machines at this point, we have BIND, N_Connected, M_Connected
        // L_Connected and A_Connected
        assert_eq!(actions.len(), 5);
        let e = actions.remove(0);
        println!("e is {:?}", e);
        let b;
        match e {
            Rendezvous(b0) => {
                b = b0;
                match &b {
                    &RC_TxBind(ref appid0, ref side0) => {
                        assert_eq!(appid0, "appid");
                        assert_eq!(side0, "side1");
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }

        let e = actions.remove(0);

        match e {
            Nameplate(N_Connected) => {
                // yay
            }
            _ => panic!(),
        }

        // we let the TxBind loop around
        actions = r.process(b).events;
        assert_eq!(actions.len(), 1);
        let e = actions.remove(0);
        println!("e is {:?}", e);
        match e {
            IO(IOAction::WebSocketSendMessage(wsh0, m)) => {
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

        actions = r.process_io(IOEvent::WebSocketConnectionLost(wsh)).events;
        assert_eq!(actions.len(), 1);
        let e = actions.pop().unwrap();
        match e {
            IO(IOAction::StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            }
            _ => panic!(),
        }

        actions = r.process_io(IOEvent::TimerExpired(th)).events;
        assert_eq!(actions.len(), 1);
        let e = actions.pop().unwrap();
        let wsh2;
        match e {
            IO(IOAction::WebSocketOpen(wsh0, url0)) => {
                // TODO: should be a different handle, once implemented
                wsh2 = wsh0;
                assert_eq!(url0, "url");
            }
            _ => panic!(),
        }

        actions = r.process(RC_Stop).events;
        // we were Connecting, so we should see a close and then wait for
        // disconnect
        assert_eq!(actions.len(), 1);
        let e = actions.pop().unwrap();
        match e {
            IO(IOAction::WebSocketClose(_wsh0)) => {
                //assert_eq!(wsh0, wsh2);
            }
            _ => panic!(),
        }

        actions = r.process_io(IOEvent::WebSocketConnectionLost(wsh2)).events;
        assert_eq!(actions.len(), 0);
    }
}
