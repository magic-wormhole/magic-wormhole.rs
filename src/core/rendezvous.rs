// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

extern crate hex;

use api::{TimerHandle, WSHandle};
use events::{AppID, Events, Mailbox, MySide, Nameplate, Phase, TheirSide};
use serde_json;
use server_messages::{
    add, allocate, bind, claim, close, deserialize, list, open, release,
    InboundMessage, OutboundMessage,
};
// we process these
use api::IOEvent;
use events::RendezvousEvent;
// we emit these
use api::IOAction;
use events::AllocatorEvent::{
    Connected as A_Connected, Lost as A_Lost, RxAllocated as A_RxAllocated,
};
use events::BossEvent::RxWelcome as B_RxWelcome;
use events::ListerEvent::{
    Connected as L_Connected, Lost as L_Lost, RxNameplates as L_RxNamePlates,
};
use events::MailboxEvent::{
    Connected as M_Connected, Lost as M_Lost, RxClosed as M_RxClosed,
    RxMessage as M_RxMessage,
};
use events::NameplateEvent::{
    Connected as N_Connected, Lost as N_Lost, RxClaimed as N_RxClaimed,
    RxReleased as N_RxReleased,
};
use events::RendezvousEvent::TxBind as RC_TxBind; // loops around
use events::TerminatorEvent::Stopped as T_Stopped;

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
pub struct RendezvousMachine {
    appid: AppID,
    relay_url: String,
    side: MySide,
    retry_timer: f32,
    state: State,
    connected_at_least_once: bool,
    wsh: WSHandle,
    reconnect_timer: Option<TimerHandle>,
}

impl RendezvousMachine {
    pub fn new(
        appid: &AppID,
        relay_url: &str,
        side: &MySide,
        retry_timer: f32,
    ) -> RendezvousMachine {
        // we use a handle here just in case we need to open multiple
        // connections in the future. For now we ignore it, but the IO layer
        // is supposed to pass this back in websocket_* messages
        let wsh = WSHandle::new(1);
        RendezvousMachine {
            appid: appid.clone(),
            relay_url: relay_url.to_string(),
            side: side.clone(),
            retry_timer,
            state: State::Idle,
            connected_at_least_once: false,
            wsh,
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
            TxBind(appid, side) => self.send(&bind(&appid, &side)),
            TxOpen(mailbox) => self.send(&open(&mailbox)),
            TxAdd(phase, body) => self.send(&add(&phase, &body)),
            TxClose(mailbox, mood) => self.send(&close(&mailbox, mood)),
            Stop => self.stop(),
            TxClaim(nameplate) => self.send(&claim(&nameplate)),
            TxRelease(nameplate) => self.send(&release(&nameplate)),
            TxAllocate => self.send(&allocate()),
            TxList => self.send(&list()),
        }
    }

    fn start(&mut self) -> Events {
        // I want this to be stable, but that makes the lifetime weird
        //let wsh = self.wsh;
        //let wsh = WSHandle{};
        let actions;
        let newstate = match self.state {
            State::Idle => {
                actions = events![IOAction::WebSocketOpen(
                    self.wsh,
                    self.relay_url.clone()
                )];
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
                    RC_TxBind(self.appid.clone(), self.side.clone()),
                    N_Connected,
                    M_Connected,
                    L_Connected,
                    A_Connected
                ];
                (a, State::Connected)
            }
            _ => panic!("bad transition from {:?}", self),
        };
        self.state = newstate;
        actions
    }

    fn message_received(&mut self, _handle: WSHandle, message: &str) -> Events {
        println!("msg is {:?}", message);
        // TODO: log+ignore unrecognized messages. They should flunk unit
        // tests, but not break normal operation
        let m = deserialize(message);
        use self::InboundMessage::*;
        match m {
            Welcome { ref welcome } => events![B_RxWelcome(welcome.clone())],
            Released { .. } => events![N_RxReleased],
            Closed { .. } => events![M_RxClosed],
            Pong { .. } => events![], // TODO
            Ack { .. } => events![], // we ignore this, it's only for the timing log
            Claimed { mailbox } => events![N_RxClaimed(Mailbox(mailbox))],
            Message {
                side,
                phase,
                body,
                //id,
            } => events![M_RxMessage(
                TheirSide(side),
                Phase(phase),
                hex::decode(body).unwrap()
            )],
            Allocated { nameplate } => {
                events![A_RxAllocated(Nameplate(nameplate))]
            }
            Nameplates { nameplates } => {
                let nids: Vec<Nameplate> = nameplates
                    .iter()
                    .map(|n| Nameplate(n.id.to_owned()))
                    .collect();
                events![L_RxNamePlates(nids)]
            }
        }
    }

    fn connection_lost(&mut self, _handle: WSHandle) -> Events {
        // TODO: assert handle == self.handle
        let (actions, newstate) = match self.state {
            State::Connecting => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                (
                    events![IOAction::StartTimer(new_handle, self.retry_timer)],
                    State::Waiting,
                )
            }
            State::Connected => {
                let new_handle = TimerHandle::new(2);
                self.reconnect_timer = Some(new_handle);
                (
                    events![
                        IOAction::StartTimer(new_handle, self.retry_timer),
                        N_Lost,
                        M_Lost,
                        L_Lost,
                        A_Lost
                    ],
                    State::Waiting,
                )
            }
            State::Disconnecting => (events![T_Stopped], State::Stopped),
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

    fn send(&mut self, m: &OutboundMessage) -> Events {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let s = IOAction::WebSocketSendMessage(
            self.wsh,
            serde_json::to_string(m).unwrap(),
        );
        events![s]
    }
}

#[cfg(test)]
mod test {
    use api::IOAction;
    use api::IOEvent;
    use api::{TimerHandle, WSHandle};
    use events::AllocatorEvent::Lost as A_Lost;
    use events::Event::{Nameplate, Rendezvous, Terminator, IO};
    use events::ListerEvent::Lost as L_Lost;
    use events::MailboxEvent::Lost as M_Lost;
    use events::NameplateEvent::{Connected as N_Connected, Lost as N_Lost};
    use events::RendezvousEvent::{Stop as RC_Stop, TxBind as RC_TxBind};
    use events::TerminatorEvent::Stopped as T_Stopped;
    use events::{AppID, MySide};
    use server_messages::{deserialize_outbound, OutboundMessage};

    #[test]
    fn create() {
        let side = MySide("side1".to_string());
        let mut r = super::RendezvousMachine::new(
            &AppID("appid".to_string()),
            "url",
            &side,
            5.0,
        );

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
                        assert_eq!(appid0.to_string(), "appid");
                        assert_eq!(side0, &MySide("side1".to_string()));
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
                if let OutboundMessage::Bind { appid, side } =
                    deserialize_outbound(&m)
                {
                    assert_eq!(appid, "appid");
                    assert_eq!(side, "side1");
                } else {
                    panic!();
                }
            }
            _ => panic!(),
        }

        actions = r.process_io(IOEvent::WebSocketConnectionLost(wsh)).events;
        assert_eq!(actions.len(), 5);
        let e = actions.remove(0);
        match e {
            IO(IOAction::StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            }
            _ => panic!(),
        }
        assert_eq!(actions, events![N_Lost, M_Lost, L_Lost, A_Lost].events);

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
        assert_eq!(actions, vec![Terminator(T_Stopped)]);
    }
}
