use events::Events;
// we process these
use events::NameplateEvent;
// we emit these
use events::RendezvousEvent::{TxClaim as RC_TxClaim, TxRelease as RC_TxRelease};
use events::TerminatorEvent::{NameplateDone as T_NameplateDone};
use events::InputEvent::{GotWordlist as I_GotWordlist};
use events::MailboxEvent::{GotMailbox as M_GotMailbox};

// all -A states are not-connected, while -B states are yes-connected
// B states serialize as A, so we wake up disconnected
#[derive(Debug, PartialEq)]
enum State {
    // S0: we know nothing
    S0A,
    S0B,
    // S1: nameplate known, but never claimed
    S1A,
    // S2: nameplate known, maybe claimed
    S2A,
    S2B,
    // S3: nameplate claimed
    S3A,
    S3B,
    // S4: maybe released
    S4A,
    S4B,
    // S5: released. we no longer care whether we're connected or not
    S5,
}

pub struct Nameplate {
    state: State,
}

impl Nameplate {
    pub fn new() -> Nameplate {
        Nameplate { state: State::S0A }
    }

    pub fn process(&mut self, event: NameplateEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0A => self.do_S0A(event),
            S0B => self.do_S0B(event),
            S1A => self.do_S1A(event),
            S2A => self.do_S2A(event),
            S2B => self.do_S2B(event),
            //S3A => self.do_S3A(event),
            //S3B => self.do_S3B(event),
            //S4A => self.do_S4A(event),
            //S4B => self.do_S4B(event),
            //S5 => self.do_S5(event),
            _ => panic!(),
        };
        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        actions


    fn do_S0A(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (Some(State::S0B), events![]),
            Lost => panic!(),
            RxClaimed => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => {
                // TODO: validate_nameplate(nameplate)
                self.nameplate = nameplate.to_string();
                (None, events![]),
            }
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_S0B(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => (State::S0A, events![]),
            RxClaimed => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => {
                // TODO: validate_nameplate(nameplate)
                self.nameplate = nameplate.to_string();
                (None, events![RC_TxClaim(nameplate.to_string())])
            }
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_S1A(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (Some(State::S2B),
                          events![RC_TxClaim(self.nameplate.to_string())]),
            Lost => panic!(),
            RxClaimed => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => panic!(),
            Release => panic!(),
            Close => (Some(State::S5), events![T_NameplateDone]),
        }
    }

    fn do_S2A(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => (Some(State::S2B),
                          events![RC_TxClaim(self.nameplate.to_string())]),
            Lost => panic!(),
            RxClaimed => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => panic!(),
            Release => panic!(),
            Close => (Some(State::S4A), events![]),
        }
    }

    fn do_S2B(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => (Some(State::S2A), events![]),
            RxClaimed(mailbox) => (Some(State::S3B),
                                   events![I_GotWordlist, // TODO: ->wordlist
                                           M_GotMailbox(mailbox)]),
            RxReleased => panic!(),
            SetNameplate(nameplate) => panic!(),
            Release => panic!(),
            Close => (Some(State::S4B), events![RC_TxRelease]),
        }
    }

        // TODO: S3B and beyond

    fn do_(&mut self, event: NameplateEvent) -> (Option<State>, Events) {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => panic!(),
            Connected => panic!(),
            Lost => panic!(),
            RxClaimed => panic!(),
            RxReleased => panic!(),
            SetNameplate(nameplate) => panic!(),
            Release => panic!(),
            Close => panic!(),
        }
    }

}
