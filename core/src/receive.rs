use events::Events;
use key;
use std::str;
// we process these
use events::ReceiveEvent;
// we emit these
use events::BossEvent::{GotMessage as B_GotMessage,
                        GotVerifier as B_GotVerifier, Happy as B_Happy,
                        Scared as B_Scared};
use events::SendEvent::GotVerifiedKey as S_GotVerifiedKey;

#[derive(Debug, PartialEq)]
enum State {
    S0UnknownKey,
    S1UnverifiedKey(Vec<u8>),
    S2VerifiedKey(Vec<u8>),
    S3Scared,
}

pub struct ReceiveMachine {
    state: State,
}

impl ReceiveMachine {
    pub fn new() -> ReceiveMachine {
        ReceiveMachine {
            state: State::S0UnknownKey,
        }
    }

    pub fn process(&mut self, event: ReceiveEvent) -> Events {
        use self::State::*;

        println!(
            "receive: current state = {:?}, got event = {:?}",
            self.state, event
        );

        let (newstate, actions) = match self.state {
            S0UnknownKey => self.in_unknown_key(event),
            S1UnverifiedKey(ref key) => self.in_unverified_key(key, event),
            S2VerifiedKey(ref key) => self.in_verified_key(key, event),
            S3Scared => self.in_scared(event),
        };

        self.state = newstate;
        actions
    }

    fn in_unknown_key(&self, event: ReceiveEvent) -> (State, Events) {
        use events::ReceiveEvent::*;
        match event {
            GotMessage(..) => panic!(),
            GotKey(key) => (State::S1UnverifiedKey(key.to_vec()), events![]),
        }
    }

    fn derive_key_and_decrypt(
        side: &str,
        key: &[u8],
        phase: &str,
        body: Vec<u8>,
    ) -> Option<Vec<u8>> {
        let data_key = key::derive_phase_key(&side, &key, &phase);

        key::decrypt_data(data_key.clone(), &body)
    }

    fn in_unverified_key(
        &self,
        key: &[u8],
        event: ReceiveEvent,
    ) -> (State, Events) {
        use events::ReceiveEvent::*;
        match event {
            GotKey(_) => panic!(),
            GotMessage(side, phase, body) => {
                match Self::derive_key_and_decrypt(&side, &key, &phase, body) {
                    Some(plaintext) => {
                        // got_message_good
                        let msg =
                            key::derive_key(&key, b"wormhole:verifier", 32); // TODO: replace 32 with KEY_SIZE const
                        (
                            State::S2VerifiedKey(key.to_vec()),
                            events![
                                S_GotVerifiedKey(key.to_vec()),
                                B_Happy,
                                B_GotVerifier(msg),
                                B_GotMessage(phase, plaintext)
                            ],
                        )
                    }
                    None => {
                        // got_message_bad
                        (State::S3Scared, events![B_Scared])
                    }
                }
            }
        }
    }

    fn in_verified_key(
        &self,
        key: &[u8],
        event: ReceiveEvent,
    ) -> (State, Events) {
        use events::ReceiveEvent::*;
        match event {
            GotKey(_) => panic!(),
            GotMessage(side, phase, body) => {
                match Self::derive_key_and_decrypt(&side, &key, &phase, body) {
                    Some(plaintext) => {
                        // got_message_good
                        (
                            State::S2VerifiedKey(key.to_vec()),
                            events![B_GotMessage(phase, plaintext)],
                        )
                    }
                    None => {
                        // got_message_bad
                        (State::S3Scared, events![B_Scared])
                    }
                }
            }
        }
    }

    fn in_scared(&self, event: ReceiveEvent) -> (State, Events) {
        use events::ReceiveEvent::*;
        match event {
            GotKey(_) => panic!(),
            GotMessage(_, _, _) => (State::S3Scared, events![]),
        }
    }
}
