use events::Events;
// we process these
use events::TerminatorEvent::{self, Close, MailboxDone, NameplateDone, Stopped};
// we emit these
use events::BossEvent::Closed as B_Closed;
use events::MailboxEvent::Close as M_Close;
use events::NameplateEvent::Close as N_Close;
use events::RendezvousEvent::Stop as RC_Stop;

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

pub struct Terminator {
    state: State,
}

impl Terminator {
    pub fn new() -> Terminator {
        Terminator {
            state: State::Snmo,
        }
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
            Close(mood) => (
                State::Snm,
                events![N_Close, M_Close(mood.to_string())],
            ),
            Stopped => panic!(
                "Got stopped to early. Nameplate and Mailbox are still active"
            ),
        }
    }

    fn in_nameplate_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            NameplateDone => (State::S0o, events![]),
            Close(mood) => (
                State::Sn,
                events![N_Close, M_Close(mood.to_string())],
            ),
            _ => panic!("Got {:?} too early, Nameplate still active"),
        }
    }

    fn in_mailbox_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            MailboxDone => (State::S0o, events![]),
            Close(mood) => (
                State::Sm,
                events![N_Close, M_Close(mood.to_string())],
            ),
            _ => panic!("Got {:?} too early, Mailbox still active"),
        }
    }

    fn in_open(&self, event: TerminatorEvent) -> (State, Events) {
        match event {
            Close(mood) => (
                State::SStopping,
                events![N_Close, M_Close(mood.to_string()), RC_Stop],
            ),
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

#[cfg(test)]
mod test {
    use super::*;
    use api::Mood::*;
    use events::BossEvent::Closed as B_Closed;
    use events::MailboxEvent::Close as M_Close;
    use events::NameplateEvent::Close as N_Close;
    use events::RendezvousEvent::Stop as RC_Stop;
    use events::TerminatorEvent::*;

    #[test]
    fn test_transitions1() {
        let mut terminator = Terminator::new();

        assert_eq!(terminator.state, State::Snmo);

        assert_eq!(terminator.process(MailboxDone), events![]);
        assert_eq!(terminator.state, State::Sno);

        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(String::from("happy"))]
        );
        assert_eq!(terminator.state, State::Sn);

        assert_eq!(
            terminator.process(NameplateDone),
            events![RC_Stop]
        );
        assert_eq!(terminator.state, State::SStopping);

        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
    }

    #[test]
    fn test_transitions2() {
        let mut terminator = Terminator::new();

        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(String::from("happy"))]
        );

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(
            terminator.process(MailboxDone),
            events![RC_Stop]
        );
        assert_eq!(terminator.process(Stopped), events![B_Closed]);

        assert_eq!(terminator.state, State::SStopped);
    }

    #[test]
    fn test_transitions3() {
        let mut terminator = Terminator::new();

        assert_eq!(terminator.process(NameplateDone), events![]);
        assert_eq!(
            terminator.process(Close(Happy)),
            events![N_Close, M_Close(String::from("happy"))]
        );
        assert_eq!(
            terminator.process(MailboxDone),
            events![RC_Stop]
        );
        assert_eq!(terminator.process(Stopped), events![B_Closed]);
        assert_eq!(terminator.state, State::SStopped);
    }
}
