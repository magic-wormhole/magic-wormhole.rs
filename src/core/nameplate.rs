use super::events::{Events, Nameplate};
use super::wordlist::default_wordlist;
use std::sync::Arc;
// we process these
use super::events::NameplateEvent;
// we emit these
use super::events::MailboxEvent::GotMailbox as M_GotMailbox;
use super::events::RendezvousEvent::{TxClaim as RC_TxClaim, TxRelease as RC_TxRelease};
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
    S2B(Nameplate),
    // S3: nameplate claimed
    S3B(Nameplate),
    // S4: maybe released
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
                },
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                },
                _ => panic!(),
            },
            S0B => match event {
                // Lost => S0A,
                SetNameplate(nameplate) => {
                    // TODO: validate_nameplate(nameplate)
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2B(nameplate)
                },
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                },
                _ => panic!(),
            },
            S1A(nameplate) => match event {
                Connected => {
                    actions.push(RC_TxClaim(nameplate.clone()));
                    S2B(nameplate)
                },
                Close => {
                    actions.push(T_NameplateDone);
                    S5
                },
                _ => panic!(),
            },
            S2B(nameplate) => match event {
                // Lost => S2A(nameplate),
                RxClaimed(mailbox) => {
                    // TODO: use nameplate attributes to pick which wordlist we use
                    actions.push(M_GotMailbox(mailbox));
                    S3B(nameplate)
                },
                Close => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                },
                _ => panic!(),
            },
            S3B(nameplate) => match event {
                Release => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                },
                Close => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate)
                },
                _ => panic!(),
            },
            S4B(ref nameplate) => match event {
                Connected => {
                    actions.push(RC_TxRelease(nameplate.clone()));
                    S4B(nameplate.clone())
                },
                RxClaimed(_mailbox) => old_state,
                RxReleased => {
                    actions.push(T_NameplateDone);
                    S5
                },
                Release => old_state,
                Close => old_state,
                _ => panic!(),
            },
            S5 => match event {
                Connected => old_state,
                Release => old_state,
                Close => old_state,
                _ => panic!(),
            },
        });
        actions
    }
}
