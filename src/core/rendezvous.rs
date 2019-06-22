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
use super::server_messages::{
    add, allocate, bind, claim, close, deserialize, list, open, release,
    InboundMessage, OutboundMessage,
};
use hex;
use log::trace;
use serde_json;
// we process these
use super::api::IOEvent;
use super::events::RendezvousEvent;
// we emit these
use super::api::IOAction;
use super::events::AllocatorEvent::{
    Connected as A_Connected, Lost as A_Lost, RxAllocated as A_RxAllocated,
};
use super::events::BossEvent::RxWelcome as B_RxWelcome;
use super::events::ListerEvent::{
    Connected as L_Connected, Lost as L_Lost, RxNameplates as L_RxNamePlates,
};
use super::events::MailboxEvent::{
    Connected as M_Connected, Lost as M_Lost, RxClosed as M_RxClosed,
    RxMessage as M_RxMessage,
};
use super::events::NameplateEvent::{
    Connected as N_Connected, Lost as N_Lost, RxClaimed as N_RxClaimed,
    RxReleased as N_RxReleased,
};

use super::events::RendezvousEvent::TxBind as RC_TxBind; // loops around
use super::events::TerminatorEvent::Stopped as T_Stopped;
use super::timing::new_timelog;

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
    state: Option<State>,
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
            state: Some(State::Idle),
            connected_at_least_once: false,
            wsh,
            reconnect_timer: None,
        }
    }

    pub fn process_io(&mut self, event: IOEvent) -> Events {
        use super::api::IOEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Idle => panic!(),
            Connecting => match event {
                WebSocketConnectionMade(_wsh) => {
                    // TODO: assert wsh == self.handle
                    // TODO: does the order of this matter? if so, oh boy.
                    actions
                        .push(RC_TxBind(self.appid.clone(), self.side.clone()));
                    actions.push(N_Connected);
                    actions.push(M_Connected);
                    actions.push(L_Connected);
                    actions.push(A_Connected);
                    Connected
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(_wsh) => {
                    // TODO: assert wsh == self.handle
                    let new_handle = TimerHandle::new(2);
                    self.reconnect_timer = Some(new_handle);
                    actions.push(IOAction::StartTimer(
                        new_handle,
                        self.retry_timer,
                    ));
                    Waiting
                }
                TimerExpired(..) => panic!(),
            },
            Connected => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(wsh, message) => {
                    self.receive(wsh, &message, &mut actions);
                    old_state
                }
                WebSocketConnectionLost(_wsh) => {
                    // TODO: assert wsh == self.handle
                    let new_handle = TimerHandle::new(2);
                    self.reconnect_timer = Some(new_handle);
                    actions.push(IOAction::StartTimer(
                        new_handle,
                        self.retry_timer,
                    ));
                    actions.push(N_Lost);
                    actions.push(M_Lost);
                    actions.push(L_Lost);
                    actions.push(A_Lost);
                    Waiting
                }
                TimerExpired(..) => panic!(),
            },
            Waiting => match event {
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(..) => panic!(),
                TimerExpired(_th) => {
                    // TODO: assert th == something
                    let new_handle = WSHandle::new(2);
                    actions.push(IOAction::WebSocketOpen(
                        new_handle,
                        self.relay_url.clone(),
                    ));
                    Connecting
                }
            },
            Disconnecting => match event {
                // -> Stopped
                WebSocketConnectionMade(..) => {
                    panic!("bad transition from {:?}", self)
                }
                WebSocketMessageReceived(..) => panic!(),
                WebSocketConnectionLost(_wsh) => {
                    // TODO: assert wsh == self.handle
                    actions.push(T_Stopped);
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
                    // I want this to be stable, but that makes the lifetime weird
                    //let wsh = self.wsh;
                    //let wsh = WSHandle{};
                    actions.push(IOAction::WebSocketOpen(
                        self.wsh,
                        self.relay_url.clone(),
                    ));
                    //"url".to_string());
                    Connecting
                }
                Stop => Stopped,
                _ => panic!(),
            },
            Connecting => match event {
                Stop => {
                    actions.push(IOAction::WebSocketClose(self.wsh));
                    Disconnecting
                }
                _ => panic!(),
            },
            Connected => match event {
                Start => panic!(),
                TxBind(appid, side) => {
                    self.send(&bind(&appid, &side), &mut actions)
                }
                TxOpen(mailbox) => self.send(&open(&mailbox), &mut actions),
                TxAdd(phase, body) => {
                    self.send(&add(&phase, &body), &mut actions)
                }
                TxClose(mailbox, mood) => {
                    self.send(&close(&mailbox, mood), &mut actions)
                }
                TxClaim(nameplate) => {
                    self.send(&claim(&nameplate), &mut actions)
                }
                TxRelease(nameplate) => {
                    self.send(&release(&nameplate), &mut actions)
                }
                TxAllocate => self.send(&allocate(), &mut actions),
                TxList => self.send(&list(), &mut actions),
                Stop => {
                    actions.push(IOAction::WebSocketClose(self.wsh));
                    Disconnecting
                }
            },
            Waiting => match event {
                Stop => {
                    actions.push(IOAction::CancelTimer(
                        self.reconnect_timer.unwrap(),
                    ));
                    Stopped
                }
                _ => panic!(),
            },
            Disconnecting => match event {
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

    fn receive(
        &mut self,
        _handle: WSHandle,
        message: &str,
        actions: &mut Events,
    ) {
        trace!("msg is {:?}", message);
        let m = deserialize(message);

        let mut t = new_timelog("ws_receive", None);
        t.detail("_side", &self.side);
        t.detail_json("message", &serde_json::to_value(&m).unwrap());
        actions.push(t);

        // TODO: log+ignore unrecognized messages. They should flunk unit
        // tests, but not break normal operation

        use self::InboundMessage::*;
        match m {
            Welcome { ref welcome } => {
                actions.push(B_RxWelcome(welcome.clone()))
            }
            Released { .. } => actions.push(N_RxReleased),
            Closed { .. } => actions.push(M_RxClosed),
            Pong { .. } => (), // TODO
            Ack { .. } => (),  // we ignore this, it's only for the timing log
            Claimed { mailbox } => actions.push(N_RxClaimed(Mailbox(mailbox))),
            Message {
                side,
                phase,
                body,
                //id,
            } => actions.push(M_RxMessage(
                TheirSide(side),
                Phase(phase),
                hex::decode(body).unwrap(),
            )),
            Allocated { nameplate } => {
                actions.push(A_RxAllocated(Nameplate(nameplate)))
            }
            Nameplates { nameplates } => {
                let nids: Vec<Nameplate> = nameplates
                    .iter()
                    .map(|n| Nameplate(n.id.to_owned()))
                    .collect();
                actions.push(L_RxNamePlates(nids));
            }
        };
    }

    fn send(&mut self, m: &OutboundMessage, actions: &mut Events) -> State {
        // TODO: add 'id' (a random string, used to correlate 'ack' responses
        // for timing-graph instrumentation)
        let mut t = new_timelog("ws_send", None);
        t.detail("_side", &self.side);
        //t.detail("id", id);
        //t.detail("type",
        // TODO: the Python version merges all the keys of 'm' at the top of
        // the event dict, rather than putting them down in the ["message"]
        // key
        t.detail_json("message", &serde_json::to_value(m).unwrap());
        actions.push(t);

        let ms = serde_json::to_string(m).unwrap();
        let s = IOAction::WebSocketSendMessage(self.wsh, ms);
        actions.push(s);
        State::Connected
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use crate::core::api::IOAction;
    use crate::core::api::IOEvent;
    use crate::core::api::{TimerHandle, WSHandle};
    use crate::core::events::AllocatorEvent::Lost as A_Lost;
    use crate::core::events::Event::{Nameplate, Rendezvous, Terminator, IO};
    use crate::core::events::ListerEvent::Lost as L_Lost;
    use crate::core::events::MailboxEvent::Lost as M_Lost;
    use crate::core::events::NameplateEvent::{
        Connected as N_Connected, Lost as N_Lost,
    };
    use crate::core::events::RendezvousEvent::{
        Start as RC_Start, Stop as RC_Stop, TxBind as RC_TxBind,
    };
    use crate::core::events::TerminatorEvent::Stopped as T_Stopped;
    use crate::core::events::{AppID, MySide};
    use crate::core::server_messages::{deserialize_outbound, OutboundMessage};
    use crate::core::test::filt;
    use log::trace;

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
        assert_eq!(actions, events![N_Lost, M_Lost, L_Lost, A_Lost].events);

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
}
