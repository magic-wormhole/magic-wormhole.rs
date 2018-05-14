use events::{Events, Wordlist};
// we process these
use events::NameplateEvent;
// we emit these
use events::InputEvent::GotWordlist as I_GotWordlist;
use events::MailboxEvent::GotMailbox as M_GotMailbox;
use events::RendezvousEvent::{TxClaim as RC_TxClaim, TxRelease as RC_TxRelease};
use events::TerminatorEvent::NameplateDone as T_NameplateDone;

// all -A states are not-connected, while -B states are yes-connected
// B states serialize as A, so we wake up disconnected
#[derive(Debug, PartialEq)]
enum State {
    // S0: we know nothing
    S0A,
    S0B,
    // S1: nameplate known, but never claimed
    S1A(String),
    // S2: nameplate known, maybe claimed
    S2A(String),
    S2B(String),
    // S3: nameplate claimed
    S3A(String),
    S3B(String),
    // S4: maybe released
    S4A(String),
    S4B(String),
    // S5: released. we no longer care whether we're connected or not
    S5,
}

pub(crate) struct Nameplate {
    state: State,
}

impl Nameplate {
    pub fn new() -> Nameplate {
        Nameplate {
            state: State::S0A,
        }
    }

    pub fn process(&mut self, event: NameplateEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0A => self.do_s0a(event),
            S0B => self.do_s0b(event),
            S1A(ref nameplate) => self.do_s1a(&nameplate, event),
            S2A(ref nameplate) => self.do_s2a(&nameplate, event),
            S2B(ref nameplate) => self.do_s2b(&nameplate, event),
            S3A(ref nameplate) => self.do_s3a(&nameplate, event),
            S3B(ref nameplate) => self.do_s3b(&nameplate, event),
            S4A(ref nameplate) => self.do_s4a(&nameplate, event),
            S4B(ref nameplate) => self.do_s4b(&nameplate, event),
            S5 => self.do_s5(event),
        };
        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        actions
    }

    fn do_s0a(&self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (Some(State::S0B), events![]),
            Lost => panic!(),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => {
                // TODO: validate_nameplate(nameplate)
                (
                    Some(State::S1A(nameplate.to_string())),
                    events![],
                )
            }
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_s0b(&self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => (Some(State::S0A), events![]),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => {
                // TODO: validate_nameplate(nameplate)
                (
                    Some(State::S2B(nameplate.to_string())),
                    events![RC_TxClaim(nameplate.to_string())],
                )
            }
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_s1a(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (
                Some(State::S2B(nameplate.to_string())),
                events![RC_TxClaim(nameplate.to_string())],
            ),
            Lost => panic!(),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_s2a(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (
                Some(State::S2B(nameplate.to_string())),
                events![RC_TxClaim(nameplate.to_string())],
            ),
            Lost => panic!(),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => panic!(),
            Close => (
                Some(State::S4A(nameplate.to_string())),
                events![],
            ),
        }
    }

    fn do_s2b(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => (
                Some(State::S2A(nameplate.to_string())),
                events![],
            ),
            RxClaimed(mailbox) => (
                Some(State::S3B(nameplate.to_string())),
                events![
                    I_GotWordlist(Wordlist {}), // TODO: ->Wordlist is just placeholder should use PGPWordList instead I guess
                    M_GotMailbox(mailbox)
                ],
            ),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => panic!(),
            Close => (
                Some(State::S4B(nameplate.to_string())),
                events![RC_TxRelease(nameplate.to_string())],
            ),
        }
    }

    fn do_s3a(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (
                Some(State::S3B(nameplate.to_string())),
                events![],
            ),
            Lost => panic!(),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => panic!(),
            Close => (
                Some(State::S4A(nameplate.to_string())),
                events![],
            ),
        }
    }

    fn do_s3b(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => (
                Some(State::S3A(nameplate.to_string())),
                events![],
            ),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => (
                Some(State::S4B(nameplate.to_string())),
                events![RC_TxRelease(nameplate.to_string())],
            ),
            Close => (
                Some(State::S4B(nameplate.to_string())),
                events![RC_TxRelease(nameplate.to_string())],
            ),
        }
    }

    fn do_s4a(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (
                Some(State::S4B(nameplate.to_string())),
                events![RC_TxRelease(nameplate.to_string())],
            ),
            Lost => (None, events![]),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => panic!(),
            Close => (None, events![]),
        }
    }

    fn do_s4b(
        &self,
        nameplate: &str,
        event: NameplateEvent,
    ) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (
                Some(State::S4B(nameplate.to_string())),
                events![RC_TxRelease(nameplate.to_string())],
            ),
            Lost => (
                Some(State::S4A(nameplate.to_string())),
                events![],
            ),
            RxClaimed(_mailbox) => (None, events![]),
            RxReleased => (Some(State::S5), events![T_NameplateDone]),
            SetNameplate(_nameplate) => panic!(),
            Release => (None, events![]),
            Close => (None, events![]),
        }
    }

    fn do_s5(&self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (None, events![]),
            Lost => (None, events![]),
            RxClaimed(_mailbox) => panic!(),
            RxReleased => panic!(),
            SetNameplate(_nameplate) => panic!(),
            Release => (None, events![]),
            Close => (None, events![]),
        }
    }
}
