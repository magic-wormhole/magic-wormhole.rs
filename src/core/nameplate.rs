use super::events::{Events, Nameplate};
// we process these
use super::events::NameplateEvent;
// we emit these
use super::events::MailboxEvent::GotMailbox as M_GotMailbox;
use super::events::RendezvousEvent::{TxClaim as RC_TxClaim, TxRelease as RC_TxRelease};

#[derive(Debug, PartialEq)]
enum State {
    // S0: we know nothing
    S0,
    // S2: nameplate known, maybe claimed
    S2(Nameplate),
    // S3: nameplate claimed
    S3(Nameplate),
    // S4: maybe released
    S4,
    // S5: released. we no longer care whether we're connected or not
    S5,
}

pub(crate) struct NameplateMachine {
    state: Option<State>,
}

impl NameplateMachine {
    pub fn new() -> NameplateMachine {
        NameplateMachine {
            state: Some(State::S0),
        }
    }

    pub fn process(&mut self, event: NameplateEvent) -> Events {
        use self::State::*;
        use NameplateEvent::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0 => match event {
                SetNameplate(nameplate) => {
                    // TODO: validate_nameplate(nameplate)
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2(nameplate)
                },
                Close => S5,
                _ => panic!(),
            },
            S2(nameplate) => match event {
                RxClaimed(mailbox) => {
                    // TODO: use nameplate attributes to pick which wordlist we use
                    actions.push(M_GotMailbox(mailbox));
                    S3(nameplate)
                },
                Close => {
                    actions.push(RC_TxRelease(nameplate));
                    S4
                },
                _ => panic!(),
            },
            S3(nameplate) => match event {
                Release | Close => {
                    actions.push(RC_TxRelease(nameplate));
                    S4
                },
                _ => panic!(),
            },
            S4 => match event {
                RxClaimed(_mailbox) => old_state,
                RxReleased => S5,
                Release => old_state,
                Close => old_state,
                _ => panic!(),
            },
            S5 => match event {
                Release | Close => old_state,
                _ => panic!(),
            },
        });
        actions
    }
}
