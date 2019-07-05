use super::events::{Events, Nameplate};
use super::wordlist::default_wordlist;
use std::sync::Arc;
// we process these
use super::events::NameplateEvent;
// we emit these
use super::events::InputEvent::GotWordlist as I_GotWordlist;
use super::events::MailboxEvent::GotMailbox as M_GotMailbox;
use super::events::RendezvousEvent::{
    TxClaim as RC_TxClaim, TxRelease as RC_TxRelease,
};
use super::events::TerminatorEvent::NameplateDone as T_NameplateDone;

// all -A states are not-connected, while -B states are yes-connected
// B states serialize as A, so we wake up disconnected
#[derive(Debug, PartialEq)]
enum State {
    // S0: we know nothing
    S0A,
    S0B,
    // S1: nameplate known, but never claimed
    S1A(Nameplate),
    // S2: nameplate known, maybe claimed
    S2A(Nameplate),
    S2B(Nameplate),
    // S3: nameplate claimed
    S3A(Nameplate),
    S3B(Nameplate),
    // S4: maybe released
    S4A(Nameplate),
    S4B(Nameplate),
    // S5: released. we no longer care whether we're connected or not
    S5,
}

pub(crate) struct NameplateMachine {
    state: Option<State>,
}

impl NameplateMachine {
    pub fn new() -> NameplateMachine {
        NameplateMachine {
            state: Some(State::S0A),
        }
    }

    pub fn process(&mut self, event: NameplateEvent) -> Events {
        use self::State::*;
        use NameplateEvent::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0A => match event {
                Connected => S0B,
                SetNameplate(nameplate) => {
                    // TODO: validate_nameplate(nameplate)
                    S1A(nameplate)
                }
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                }
                _ => panic!(),
            },
            S0B => match event {
                Lost => S0A,
                SetNameplate(nameplate) => {
                    // TODO: validate_nameplate(nameplate)
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2B(nameplate)
                }
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                }
                _ => panic!(),
            },
            S1A(nameplate) => match event {
                Connected => {
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2B(nameplate)
                }
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                }
                _ => panic!(),
            },
            S2A(nameplate) => match event {
                Connected => {
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2B(nameplate)
                }
                Close => S4A(nameplate),
                _ => panic!(),
            },
            S2B(nameplate) => match event {
                Lost => S2A(nameplate),
                RxClaimed(mailbox) => {
                    // TODO: use nameplate attributes to pick which wordlist we use
                    let wordlist = Arc::new(default_wordlist(2)); // TODO: num_words
                    actions.push(I_GotWordlist(wordlist));
                    actions.push(M_GotMailbox(mailbox));
                    S3B(nameplate)
                }
                Close => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                }
                _ => panic!(),
            },
            S3A(nameplate) => match event {
                Connected => S3B(nameplate),
                Close => S4A(nameplate),
                _ => panic!(),
            },
            S3B(nameplate) => match event {
                Lost => S3A(nameplate),
                Release => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                }
                Close => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                }
                _ => panic!(),
            },
            S4A(ref nameplate) => match event {
                Connected => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate.clone())
                }
                Lost => old_state,
                Close => old_state,
                _ => panic!(),
            },
            S4B(ref nameplate) => match event {
                Connected => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate.clone())
                }
                Lost => S4A(nameplate.clone()),
                RxClaimed(_mailbox) => old_state,
                RxReleased => {
                    actions.push(T_NameplateDone);
                    S5
                }
                Release => old_state,
                Close => old_state,
                _ => panic!(),
            },
            S5 => match event {
                Connected => old_state,
                Lost => old_state,
                Release => old_state,
                Close => old_state,
                _ => panic!(),
            },
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::events::{
        Event, InputEvent, Mailbox, MailboxEvent, NameplateEvent::*,
        RendezvousEvent, TerminatorEvent,
    };

    #[test]
    fn test_nc() {
        // set_N_ameplate, then _C_onnected
        let name1 = Nameplate("n1".to_string());
        let mbox1 = Mailbox("mbox1".to_string());
        let mut n = NameplateMachine::new();

        let mut e = n.process(SetNameplate(name1.clone()));
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(RxClaimed(mbox1.clone()));
        let e0 = e.events.remove(0);
        match e0 {
            Event::Input(InputEvent::GotWordlist(w)) => {
                // TODO: for now, we use a hard-coded wordlist, but it (or a
                // short identifier) will eventually be included in the
                // RxClaimed response, supplied by the server
                dbg!(w);
                //assert_eq!(w.num_words, 2);
            }
            _ => panic!(e0),
        };
        assert_eq!(e, events![MailboxEvent::GotMailbox(mbox1.clone())]);

        e = n.process(Release);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone())]);

        e = n.process(RxReleased);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);

        e = n.process(Lost);
        assert_eq!(e, events![]);
    }

    #[test]
    fn test_cn() {
        // connect, then SetNameplate. Also test a bunch of lost+reconnect
        // paths.
        let name1 = Nameplate("n1".to_string());
        let mbox1 = Mailbox("mbox1".to_string());
        let mut n = NameplateMachine::new();

        let mut e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(Lost);
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(SetNameplate(name1.clone()));
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(Lost);
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(RxClaimed(mbox1.clone()));
        let e0 = e.events.remove(0);
        match e0 {
            Event::Input(InputEvent::GotWordlist(w)) => {
                // TODO: for now, we use a hard-coded wordlist, but it (or a
                // short identifier) will eventually be included in the
                // RxClaimed response, supplied by the server
                dbg!(w);
                //assert_eq!(w.num_words, 2);
            }
            _ => panic!(e0),
        };
        assert_eq!(e, events![MailboxEvent::GotMailbox(mbox1.clone())]);

        e = n.process(Lost);
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(Release);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone())]);

        e = n.process(Lost);
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone())]);
    }

    #[test]
    fn test_close1() {
        let mut n = NameplateMachine::new();

        let e = n.process(Close);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

    #[test]
    fn test_close2() {
        let mut n = NameplateMachine::new();
        let name1 = Nameplate("n1".to_string());

        let mut e = n.process(SetNameplate(name1));
        assert_eq!(e, events![]);

        e = n.process(Close);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

    #[test]
    fn test_close3() {
        let mut n = NameplateMachine::new();
        let name1 = Nameplate("n1".to_string());

        let mut e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(SetNameplate(name1.clone()));
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(Lost);
        assert_eq!(e, events![]);

        e = n.process(Close);
        assert_eq!(e, events![]);
        // we're now in S4A "maybe released". We can't signal Terminator
        // until we get an ack, which require a connection

        e = n.process(Close); // duplicate close is ok in S4A
        assert_eq!(e, events![]);

        e = n.process(Connected);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone())]);

        e = n.process(Close); // duplicate close is ok in S4B too
        assert_eq!(e, events![]);

        e = n.process(RxReleased);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

    #[test]
    fn test_close4() {
        let mut n = NameplateMachine::new();

        let mut e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(Close);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

    #[test]
    fn test_close5() {
        let mut n = NameplateMachine::new();
        let name1 = Nameplate("n1".to_string());

        let mut e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(SetNameplate(name1.clone()));
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(Close);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone()),]);
        e = n.process(RxReleased);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

    #[test]
    fn test_close6() {
        let mut n = NameplateMachine::new();
        let name1 = Nameplate("n1".to_string());
        let mbox1 = Mailbox("mbox1".to_string());

        let mut e = n.process(Connected);
        assert_eq!(e, events![]);

        e = n.process(SetNameplate(name1.clone()));
        assert_eq!(e, events![RendezvousEvent::TxClaim(name1.clone()),]);

        e = n.process(RxClaimed(mbox1.clone()));
        let e0 = e.events.remove(0);
        match e0 {
            Event::Input(InputEvent::GotWordlist(_w)) => (),
            _ => panic!(e0),
        };
        assert_eq!(e, events![MailboxEvent::GotMailbox(mbox1.clone())]);

        e = n.process(Close);
        assert_eq!(e, events![RendezvousEvent::TxRelease(name1.clone()),]);
        e = n.process(RxReleased);
        assert_eq!(e, events![TerminatorEvent::NameplateDone]);
    }

}
