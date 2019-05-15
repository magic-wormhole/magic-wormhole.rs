use super::events::{Events, Key, MySide, Phase};
use super::key;
use log::trace;
// we process these
use super::events::SendEvent;
// we emit these
use super::events::MailboxEvent::AddMessage as M_AddMessage;

#[derive(Debug, PartialEq)]
enum State {
    S0NoKey,
    S1HaveVerifiedKey(Key),
}

pub struct SendMachine {
    state: Option<State>,
    side: MySide,
    queue: Vec<(Phase, Vec<u8>)>,
}

impl SendMachine {
    pub fn new(side: &MySide) -> SendMachine {
        SendMachine {
            state: Some(State::S0NoKey),
            side: side.clone(),
            queue: Vec::new(),
        }
    }

    pub fn process(&mut self, event: SendEvent) -> Events {
        trace!(
            "send: current state = {:?}, got event = {:?}",
            self.state,
            event
        );
        use super::events::SendEvent::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            State::S0NoKey => {
                match event {
                    GotVerifiedKey(ref key) => {
                        for (phase, plaintext) in self.queue.drain(..) {
                            let data_key =
                                key::derive_phase_key(&self.side, &key, &phase);
                            let (_nonce, encrypted) =
                                key::encrypt_data(&data_key, &plaintext);
                            actions.push(M_AddMessage(phase, encrypted));
                        }
                        State::S1HaveVerifiedKey(key.clone())
                    }
                    Send(phase, plaintext) => {
                        // we don't have a verified key, yet we got messages to
                        // send, so queue it up.
                        self.queue.push((phase, plaintext));
                        State::S0NoKey
                    }
                }
            }
            State::S1HaveVerifiedKey(ref key) => match event {
                GotVerifiedKey(_) => panic!(),
                Send(phase, plaintext) => {
                    let data_key =
                        key::derive_phase_key(&self.side, &key, &phase);
                    let (_nonce, encrypted) =
                        key::encrypt_data(&data_key, &plaintext);
                    actions.push(M_AddMessage(phase, encrypted));
                    State::S1HaveVerifiedKey(key.clone())
                }
            },
        });

        actions
    }
}
