// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use super::api::{TimerHandle, WSHandle};
use super::events::{
    AppID, Events, Mailbox, MySide, Nameplate, Phase, TheirSide,
};
use hex;
use log::trace;
use serde_json;
// we process these
use super::api::IOEvent;
use super::events::RendezvousEvent;
// we emit these
use super::api::IOAction;
use super::events::{
    AllocatorEvent, BossEvent, ListerEvent, MailboxEvent, NameplateEvent,
    TerminatorEvent,
};

use super::events::RendezvousEvent::TxBind as RC_TxBind; // loops around
use super::timing::new_timelog;

#[derive(Debug, PartialEq)]
enum State {
    Idle,
    Connecting(WSHandle),
    Connected(WSHandle),
    Disconnecting(WSHandle), // -> Stopped
    Waiting(TimerHandle),
    Stopped,
}

#[derive(Debug)]
pub struct RendezvousMachine {
    appid: AppID,
    relay_url: String,
    side: MySide,
    retry_delay: f32,
    state: Option<State>,
    connected_at_least_once: bool,
    next_timer_id: u32,
    next_wsh_id: u32,
}

impl RendezvousMachine {
    pub fn new(
        appid: &AppID,
        relay_url: &str,
        side: &MySide,
        retry_delay: f32,
    ) -> RendezvousMachine {
        RendezvousMachine {
            appid: appid.clone(),
            relay_url: relay_url.to_string(),
            side: side.clone(),
            retry_delay,
            state: Some(State::Idle),
            connected_at_least_once: false,
            next_timer_id: 1,
            next_wsh_id: 1,
        }
    }

    fn new_timer_handle(&mut self) -> TimerHandle {
        let th = TimerHandle::new(self.next_timer_id);
        self.next_timer_id += 1;
        th
    }

    fn new_ws_handle(&mut self) -> WSHandle {
        let wsh = WSHandle::new(self.next_wsh_id);
        self.next_wsh_id += 1;
        wsh
    }

    pub fn process_io(&mut self, event: IOEvent) -> Events {
        use super::api::IOEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Idle => panic!(),
            Connecting(wsh) => match event {
                WebSocketConnectionMade(h) => {
                    assert!(wsh == h);
                    // TODO: does the order of this matter? if so, oh boy.
                    let txb = RC_TxBind(self.appid.clone(), self.side.clone());
                    actions.push(txb);
                    actions.push(NameplateEvent::Connected);
                    actions.push(MailboxEvent::Connected);
                    actions.push(ListerEvent::Connected);
                    actions.push(AllocatorEvent::Connected);
                    Connected(wsh)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(h) => {
                    assert!(wsh == h);
                    if !self.connected_at_least_once {
                        // TODO: WebSocketConnectionLost(wsh, reason)
                        let e = BossEvent::Error(String::from(
                            "initial WebSocket connection lost",
                        ));
                        actions.push(e);
                        Stopped
                    } else {
                        let th = self.new_timer_handle();
                        actions
                            .push(IOAction::StartTimer(th, self.retry_delay));
                        Waiting(th)
                    }
                }
                TimerExpired(..) => panic!(),
            },
            Connected(wsh) => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(h, message) => {
                    assert!(wsh == h);
                    self.receive(&message, &mut actions);
                    old_state
                }
                WebSocketConnectionLost(h) => {
                    assert!(wsh == h);
                    let th = self.new_timer_handle();
                    actions.push(IOAction::StartTimer(th, self.retry_delay));
                    actions.push(NameplateEvent::Lost);
                    actions.push(MailboxEvent::Lost);
                    actions.push(ListerEvent::Lost);
                    actions.push(AllocatorEvent::Lost);
                    Waiting(th)
                }
                TimerExpired(..) => panic!(),
            },
            Waiting(th) => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(..) => panic!(),
                TimerExpired(h) => {
                    assert!(th == h);
                    let wsh = self.new_ws_handle();
                    actions.push(IOAction::WebSocketOpen(
                        wsh,
                        self.relay_url.clone(),
                    ));
                    Connecting(wsh)
                }
            },
            Disconnecting(wsh) => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(h) => {
                    assert!(wsh == h);
                    actions.push(TerminatorEvent::Stopped);
                    Stopped
                }
                TimerExpired(..) => panic!(),
            },
            Stopped => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(..) => panic!(),
                TimerExpired(..) => panic!(),
            },
        });
        actions
    }

    pub fn process(&mut self, event: RendezvousEvent) -> Events {
        use super::events::RendezvousEvent::*;
        use State::*;
        trace!("rendezvous: {:?}", event);
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Idle => match event {
                Start => {
                    // We use a handle here just in case we need to open
                    // multiple connections in the future. The IO layer is
                    // supposed to pass this back in websocket_* messages.
                    let wsh = self.new_ws_handle();
                    actions.push(IOAction::WebSocketOpen(
                        wsh,
                        self.relay_url.clone(),
                    ));
                    Connecting(wsh)
                }
                Stop => Stopped,
                _ => panic!(),
            },
            Connecting(wsh) => match event {
                Stop => {
                    actions.push(IOAction::WebSocketClose(wsh));
                    Disconnecting(wsh)
                }
                _ => panic!(),
            },
            Connected(wsh) => match event {
                Start => panic!(),
                TxBind(..) | TxOpen(..) | TxAdd(..) | TxClose(..)
                | TxClaim(..) | TxRelease(..) | TxAllocate | TxList => {
                    for a in self.send(event, wsh) {
                        actions.push(a)
                    }
                    old_state
                }
                Stop => {
                    actions.push(IOAction::WebSocketClose(wsh));
                    Disconnecting(wsh)
                }
            },
            Waiting(th) => match event {
                Stop => {
                    actions.push(IOAction::CancelTimer(th));
                    Stopped
                }
                _ => panic!(),
            },
            Disconnecting(_) => match event {
                Stop => old_state,
                _ => panic!(),
            },
            Stopped => match event {
                Stop => Stopped,
                _ => panic!(),
            },
        });
        actions
    }

    fn receive(&mut self, message: &str, actions: &mut Events) {
        trace!("msg is {:?}", message);
        use super::server_messages::deserialize;
        let m = deserialize(message);

        let mut t = new_timelog("ws_receive", None);
        t.detail("_side", &self.side);
        t.detail_json("message", &serde_json::to_value(&m).unwrap());
        actions.push(t);

        // TODO: log+ignore unrecognized messages. They should flunk unit
        // tests, but not break normal operation

        use super::server_messages::InboundMessage::*;
        match m {
            Welcome { ref welcome } => {
                actions.push(BossEvent::RxWelcome(welcome.clone()))
            }
            Released { .. } => actions.push(NameplateEvent::RxReleased),
            Closed { .. } => actions.push(MailboxEvent::RxClosed),
            Pong { .. } => (), // TODO
            Ack { .. } => (),  // we ignore this, it's only for the timing log
            Claimed { mailbox } => {
                actions.push(NameplateEvent::RxClaimed(Mailbox(mailbox)))
            }
            Message { side, phase, body } => {
                actions.push(MailboxEvent::RxMessage(
                    TheirSide(side),
                    Phase(phase),
                    hex::decode(body).unwrap(),
                ))
            }
            Allocated { nameplate } => {
                actions.push(AllocatorEvent::RxAllocated(Nameplate(nameplate)))
            }
            Nameplates { nameplates } => {
                let nids: Vec<Nameplate> = nameplates
                    .iter()
                    .map(|n| Nameplate(n.id.to_owned()))
                    .collect();
                actions.push(ListerEvent::RxNameplates(nids));
            }
        };
    }

    fn send(&mut self, e: RendezvousEvent, wsh: WSHandle) -> Events {
        use super::events::RendezvousEvent::*;
        use super::server_messages::{
            add, allocate, bind, claim, close, list, open, release,
        };
        let m = match e {
            TxBind(appid, side) => bind(&appid, &side),
            TxOpen(mailbox) => open(&mailbox),
            TxAdd(phase, body) => add(&phase, &body),
            TxClose(mailbox, mood) => close(&mailbox, mood),
            TxClaim(nameplate) => claim(&nameplate),
            TxRelease(nameplate) => release(&nameplate),
            TxAllocate => allocate(),
            TxList => list(),
            Start | Stop => panic!(),
        };
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let mut t = new_timelog("ws_send", None);
        t.detail("_side", &self.side);
        //t.detail("id", id);
        //t.detail("type",
        // TODO: the Python version merges all the keys of 'm' at the top of
        // the event dict, rather than putting them down in the ["message"]
        // key
        t.detail_json("message", &serde_json::to_value(&m).unwrap());

        let ms = serde_json::to_string(&m).unwrap();
        let s = IOAction::WebSocketSendMessage(wsh, ms);
        events![t, s]
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use crate::core::api::IOAction;
    use crate::core::api::IOEvent;
    use crate::core::api::{TimerHandle, WSHandle};
    use crate::core::events::Event::{Nameplate, Rendezvous, Terminator, IO};
    use crate::core::events::RendezvousEvent::{
        Start as RC_Start, Stop as RC_Stop, TxBind as RC_TxBind,
    };
    use crate::core::events::TerminatorEvent::Stopped as T_Stopped;
    use crate::core::events::{
        AllocatorEvent, BossEvent, ListerEvent, MailboxEvent, NameplateEvent,
    };
    use crate::core::events::{AppID, MySide};
    use crate::core::server_messages::{deserialize_outbound, OutboundMessage};
    use crate::core::test::filt;
    use log::trace;

    #[test]
    fn create() {
        let side = MySide(String::from("side1"));
        let mut r = super::RendezvousMachine::new(
            &AppID(String::from("appid")),
            "url",
            &side,
            5.0,
        );

        let wsh: WSHandle;
        let th: TimerHandle;

        let mut actions = filt(r.process(RC_Start)).events;
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
        actions =
            filt(r.process_io(IOEvent::WebSocketConnectionMade(wsh))).events;
        // it should tell itself to send a BIND then it should notify several
        // other machines at this point, we have BIND, N_Connected, M_Connected
        // L_Connected and A_Connected
        assert_eq!(actions.len(), 5);
        let e = actions.remove(0);
        trace!("e is {:?}", e);
        let b;
        match e {
            Rendezvous(b0) => {
                b = b0;
                match &b {
                    &RC_TxBind(ref appid0, ref side0) => {
                        assert_eq!(appid0.to_string(), "appid");
                        assert_eq!(side0, &MySide(String::from("side1")));
                    }
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }

        let e = actions.remove(0);

        match e {
            Nameplate(NameplateEvent::Connected) => {
                // yay
            }
            _ => panic!(),
        }

        // we let the TxBind loop around
        actions = filt(r.process(b)).events;
        assert_eq!(actions.len(), 1);
        let e = actions.remove(0);
        trace!("e is {:?}", e);
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

        // once connected, we react to connection loss by starting the
        // reconnection timer
        actions =
            filt(r.process_io(IOEvent::WebSocketConnectionLost(wsh))).events;
        assert_eq!(actions.len(), 5);
        let e = actions.remove(0);
        match e {
            IO(IOAction::StartTimer(handle, duration)) => {
                assert_eq!(duration, 5.0);
                th = handle;
            }
            _ => panic!(),
        }
        assert_eq!(
            actions,
            events![
                NameplateEvent::Lost,
                MailboxEvent::Lost,
                ListerEvent::Lost,
                AllocatorEvent::Lost
            ]
            .events
        );

        actions = filt(r.process_io(IOEvent::TimerExpired(th))).events;
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

        actions = filt(r.process(RC_Stop)).events;
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

        actions =
            filt(r.process_io(IOEvent::WebSocketConnectionLost(wsh2))).events;
        assert_eq!(actions, vec![Terminator(T_Stopped)]);
    }

    #[test]
    fn first_connect_fails() {
        let side = MySide(String::from("side1"));
        let mut r = super::RendezvousMachine::new(
            &AppID(String::from("appid")),
            "url",
            &side,
            5.0,
        );
        let wsh: WSHandle;
        let mut actions = filt(r.process(RC_Start)).events;
        if let IO(IOAction::WebSocketOpen(wsh0, ..)) = actions.remove(0) {
            wsh = wsh0;
        } else {
            panic!();
        }
        assert!(actions.is_empty());

        let actions = filt(r.process_io(IOEvent::WebSocketConnectionLost(wsh)));
        assert_eq!(
            actions,
            events![BossEvent::Error(String::from(
                "initial WebSocket connection lost"
            ))]
        );
    }
}
