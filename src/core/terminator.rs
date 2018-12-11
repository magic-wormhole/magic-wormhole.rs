use super::events::Events;
// we process these
use super::events::TerminatorEvent::{
    self, Close, MailboxDone, NameplateDone, Stopped,
};
// we emit these
use super::events::BossEvent::Closed as B_Closed;
use super::events::MailboxEvent::Close as M_Close;
use super::events::NameplateEvent::Close as N_Close;
use super::events::RendezvousEvent::Stop as RC_Stop;

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Snmo,
    Sno,
    Smo,
    S0o,
    Snm,
    Sn,
    Sm,
    SStopping,
    SStopped,
}

pub struct TerminatorMachine {
    state: State,
}

impl TerminatorMachine {
    pub fn new() -> TerminatorMachine {
        TerminatorMachine { state: State::Snmo }
    }

    pub fn process(&mut self, event: TerminatorEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            Snmo => self.in_nameplate_mailbox_open(event),
            Sno => self.in_nameplate_open(event),
            Smo => self.in_mailbox_open(event),
            S0o => self.in_open(event),
            Snm => self.in_nameplate_mailbox_active(event),
            Sn => self.in_nameplate_active(event),
            Sm => self.in_mailbox_active(event),
            SStopping => self.in_stopping(event),
            SStopped => panic!("Already stopped"),
        };

        self.state = newstate;
        actions
    }

    fn in_nameplate_mailbox_open(
        &self,
        event: TerminatorEvent,
    ) -> (State, Events) {
        match event {
            MailboxDone => (State::Sno, events![]),
            NameplateDone => (State::Smo, events![]),
            Close(mood) => (State::Snm, events![N_Close, M_Close(mood)]),
            Stopped => panic!(
                "Got stopped to early. Nameplate and Mailbox are still active"
            ),
        }
    }

    fn in_nameplate_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            NameplateDone => (State::S0o, events![]),
            Close(mood) => (State::Sn, events![N_Close, M_Close(mood)]),
            _ => panic!("Got too early, Nameplate still active"),
        }
    }

    fn in_mailbox_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            MailboxDone => (State::S0o, events![]),
            Close(mood) => (State::Sm, events![N_Close, M_Close(mood)]),
            _ => panic!("Got too early, Mailbox still active"),
        }
    }

    fn in_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            Close(mood) => {
                (State::SStopping, events![N_Close, M_Close(mood), RC_Stop])
            }
            MailboxDone | NameplateDone => panic!("Got {:?} too late", event),
            _ => panic!("Too early to stop"),
        }
    }
    fn in_nameplate_mailbox_active(
        &self,
        event: TerminatorEvent,
    ) -> (State, Events) {
        match event {
            MailboxDone => (State::Sn, events![]),
            NameplateDone => (State::Sm, events![]),
            Close(_) => panic!("Too late already closing"),
            Stopped => panic!("Still not stopping"),
        }
    }

    fn in_nameplate_active(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            NameplateDone => (State::SStopping, events![RC_Stop]),
            _ => panic!("Too early/late"),
        }
    }

    fn in_mailbox_active(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            MailboxDone => (State::SStopping, events![RC_Stop]),
            _ => panic!("Too early or late"),
        }
    }

    fn in_stopping(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            Stopped => (State::SStopped, events![B_Closed]),
            _ => panic!("Too late"),
        }
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::api::Mood::*;
    use crate::core::events::BossEvent::Closed as B_Closed;
    use crate::core::events::MailboxEvent::Close as M_Close;
    use crate::core::events::NameplateEvent::Close as N_Close;
    use crate::core::events::RendezvousEvent::Stop as RC_Stop;

    #[test]
    fn test_transitions1() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.state, State::Snmo);

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.state, State::Sno);

        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(Happy)]
        );
        assert_eq!(terminator.state, State::Sn);

        assert_eq!(terminator.process(NameplateDone), events![RC_Stop]);
        assert_eq!(terminator.state, State::SStopping);

        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
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

        assert_eq!(terminator.state, State::SStopped);
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
        assert_eq!(terminator.state, State::SStopped);
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
        assert_eq!(terminator.state, State::SStopping);
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
    }

    #[test]
    fn test_transitions32() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.state, State::S0o);
        assert_eq!(
            terminator.process(Close(Scared)),
            events![N_Close, M_Close(Scared), RC_Stop]
        );
        assert_eq!(terminator.state, State::SStopping);
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
    }

    #[test]
    fn test_transitions12() {
        let mut terminator = TerminatorMachine::new();

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(terminator.state, State::S0o);

        assert_eq!(
            terminator.process(Close(Error)),
            events![N_Close, M_Close(Error), RC_Stop]
        );
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
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
        terminator.state = State::S0o;
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic7() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sn;
        terminator.process(MailboxDone);
    }

    #[test]
    #[should_panic]
    fn panic8() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sn;
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic9() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sn;
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic10() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sm;
        terminator.process(NameplateDone);
    }

    #[test]
    #[should_panic]
    fn panic11() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sm;
        terminator.process(Close(Happy));
    }

    #[test]
    #[should_panic]
    fn panic12() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::Sm;
        terminator.process(Stopped);
    }

    #[test]
    #[should_panic]
    fn panic13() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::SStopping;
        terminator.process(NameplateDone);
    }

    #[test]
    #[should_panic]
    fn panic14() {
        let mut terminator = TerminatorMachine::new();
        terminator.state = State::SStopped;
        terminator.process(MailboxDone);
    }
}
