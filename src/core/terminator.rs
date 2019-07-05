use super::events::Events;
// we process these
use super::events::TerminatorEvent;
// we emit these
use super::events::BossEvent::Closed as B_Closed;
use super::events::MailboxEvent::Close as M_Close;
use super::events::NameplateEvent::Close as N_Close;
use super::events::RendezvousEvent::Stop as RC_Stop;

// we start in Snmo, each letter of m/n/o is dropped by an event:
//  MailboxDone drops "m"
//  NameplateDone drops "n"
//  Close drops "o"
// when all three are dropped, we move to SStopping until we get Stopped

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Snmo,
    Sno,
    Smo,
    So,
    Snm,
    Sn,
    Sm,
    SStopping,
    SStopped,
}

pub struct TerminatorMachine {
    state: Option<State>,
}

impl TerminatorMachine {
    pub fn new() -> TerminatorMachine {
        TerminatorMachine {
            state: Some(State::Snmo),
        }
    }

    pub fn process(&mut self, event: TerminatorEvent) -> Events {
        use State::*;
        use TerminatorEvent::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Snmo => match event {
                // nameplate_mailbox_open
                MailboxDone => Sno,
                NameplateDone => Smo,
                Close(mood) => {
                    actions.push(N_Close);
                    actions.push(M_Close(mood));
                    Snm
                },
                Stopped => panic!(
                    "Got stopped to early. Nameplate and Mailbox are still active"
                ),
            }
            Sno => match event {
                // nameplate_open
                NameplateDone => So,
                Close(mood) => {
                    actions.push(N_Close);
                    actions.push(M_Close(mood));
                    Sn
                },
                _ => panic!("Got too early, Nameplate still active"),
            },

            Smo => match event {
                // mailbox_open
                MailboxDone => So,
                Close(mood) => {
                    actions.push(N_Close);
                    actions.push(M_Close(mood));
                    Sm
                },
                _ => panic!("Got too early, Mailbox still active"),
            },
            So => match event {
                // open
                Close(mood) => {
                    actions.push(N_Close);
                    actions.push(M_Close(mood));
                    actions.push(RC_Stop);
                    SStopping
                },
                MailboxDone | NameplateDone => panic!("Got {:?} too late", event),
                _ => panic!("Too early to stop"),
            },
            Snm => match event {
                // nameplate_mailbox_active(event)
                MailboxDone => Sn,
                NameplateDone => Sm,
                Close(_) => panic!("Too late already closing"),
                Stopped => panic!("Still not stopping"),
            },

            Sn => match event {
                // nameplate_active
                NameplateDone => {
                    actions.push(RC_Stop);
                    SStopping
                },
                _ => panic!("Too early/late"),
            },
            Sm => match event {
                // mailbox_active
                MailboxDone => {
                    actions.push(RC_Stop);
                    SStopping
                },
                _ => panic!("Too early or late"),
            },
            SStopping => match event {
                // stopping
                Stopped => {
                    actions.push(B_Closed);
                    SStopped
                },
                _ => panic!("Too late"),
            },
            SStopped => panic!("Already stopped"),
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::TerminatorEvent::*;
    use super::*;
    use crate::core::api::Mood::*;
    use crate::core::events::BossEvent::Closed as B_Closed;
    use crate::core::events::MailboxEvent::Close as M_Close;
    use crate::core::events::NameplateEvent::Close as N_Close;
    use crate::core::events::RendezvousEvent::Stop as RC_Stop;

    #[test]
    fn test_transitions1() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.state, Some(State::Snmo));

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.state, Some(State::Sno));

        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(Happy)]
        );
        assert_eq!(terminator.state, Some(State::Sn));

        assert_eq!(terminator.process(NameplateDone), events![RC_Stop]);
        assert_eq!(terminator.state, Some(State::SStopping));

        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    fn test_transitions2() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(Happy)]
        );

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(terminator.process(MailboxDone), events![RC_Stop]);
        assert_eq!(terminator.process(Stopped), events![B_Closed]);

        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    fn test_transitions3() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(Happy)]
        );
        assert_eq!(terminator.process(MailboxDone), events![RC_Stop]);
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    fn test_transitions21() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(
            terminator.process(Close(Lonely)),
            events![N_Close, M_Close(Lonely)]
        );

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.process(NameplateDone), events![RC_Stop]);
        assert_eq!(terminator.state, Some(State::SStopping));
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    fn test_transitions32() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.state, Some(State::So));
        assert_eq!(
            terminator.process(Close(Scared)),
            events![N_Close, M_Close(Scared), RC_Stop]
        );
        assert_eq!(terminator.state, Some(State::SStopping));
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    fn test_transitions12() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(terminator.state, Some(State::So));

        assert_eq!(
            terminator.process(Close(Errory)),
            events![N_Close, M_Close(Errory), RC_Stop]
        );
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, Some(State::SStopped));
    }

    #[test]
    #[should_panic]
    fn panic1() {
        let mut terminator = TerminatorMachine::new();
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic2() {
        let mut terminator = TerminatorMachine::new();
        terminator.process(MailboxDone);
        terminator.process(MailboxDone);
    }

    #[test]
    #[should_panic]
    fn panic3() {
        let mut terminator = TerminatorMachine::new();
        terminator.process(MailboxDone);
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic4() {
        let mut terminator = TerminatorMachine::new();
        terminator.process(Close(Happy));
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic5() {
        let mut terminator = TerminatorMachine::new();
        terminator.process(NameplateDone);
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic6() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::So);
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic7() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sn);
        terminator.process(MailboxDone);
    }

    #[test]
    #[should_panic]
    fn panic8() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sn);
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic9() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sn);
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic10() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sm);
        terminator.process(NameplateDone);
    }

    #[test]
    #[should_panic]
    fn panic11() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sm);
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic12() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::Sm);
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic13() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::SStopping);
        terminator.process(NameplateDone);
    }

    #[test]
    #[should_panic]
    fn panic14() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = Some(State::SStopped);
        terminator.process(MailboxDone);
    }

    #[test]
    #[should_panic]
    fn panic15() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::So);
        terminator.process(MailboxDone);
    }

    #[test]
    #[should_panic]
    fn panic16() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::So);
        terminator.process(NameplateDone);
    }

    #[test]
    #[should_panic]
    fn panic17() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::Snm);
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic18() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::Sn);
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic19() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::Sm);
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic20() {
        let mut terminator = TerminatorMachine::new();

        terminator.state = Some(State::SStopping);
        terminator.process(Close(Happy));
    }

}
