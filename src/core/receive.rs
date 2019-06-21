use super::events::{Events, Key, Phase, TheirSide};
use super::key;
use log::trace;
// we process these
use super::events::ReceiveEvent;
// we emit these
use super::events::BossEvent::{
    GotMessage as B_GotMessage, GotVerifier as B_GotVerifier, Happy as B_Happy,
    Scared as B_Scared,
};
use super::events::SendEvent::GotVerifiedKey as S_GotVerifiedKey;

#[derive(Debug, PartialEq)]
enum State {
    S0UnknownKey,
    S1UnverifiedKey(Key),
    S2VerifiedKey(Key),
    S3Scared,
}

pub struct ReceiveMachine {
    state: Option<State>,
}

impl ReceiveMachine {
    pub fn new() -> ReceiveMachine {
        ReceiveMachine {
            state: Some(State::S0UnknownKey),
        }
    }

    pub fn process(&mut self, event: ReceiveEvent) -> Events {
        use self::State::*;
        use ReceiveEvent::*;

        trace!(
            "receive: current state = {:?}, got event = {:?}",
            self.state,
            event
        );

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();

        self.state = Some(match old_state {
            S0UnknownKey => match event {
                GotMessage(..) => panic!(),
                GotKey(key) => S1UnverifiedKey(key.clone()),
            },
            S1UnverifiedKey(ref key) => match event {
                GotKey(_) => panic!(),
                GotMessage(side, phase, body) => {
                    match Self::derive_key_and_decrypt(
                        &side, &key, &phase, &body,
                    ) {
                        Some(plaintext) => {
                            // got_message_good
                            let msg =
                                key::derive_key(&key, b"wormhole:verifier", 32);
                            // TODO: replace 32 with KEY_SIZE const
                            actions.push(S_GotVerifiedKey(key.clone()));
                            actions.push(B_Happy);
                            actions.push(B_GotVerifier(msg));
                            actions.push(B_GotMessage(phase, plaintext));
                            S2VerifiedKey(key.clone())
                        }
                        None => {
                            // got_message_bad
                            actions.push(B_Scared);
                            S3Scared
                        }
                    }
                }
            },
            S2VerifiedKey(ref key) => match event {
                GotKey(_) => panic!(),
                GotMessage(side, phase, body) => {
                    match Self::derive_key_and_decrypt(
                        &side, &key, &phase, &body,
                    ) {
                        Some(plaintext) => {
                            // got_message_good
                            actions.push(B_GotMessage(phase, plaintext));
                            S2VerifiedKey(key.clone())
                        }
                        None => {
                            // got_message_bad
                            actions.push(B_Scared);
                            S3Scared
                        }
                    }
                }
            },
            S3Scared => match event {
                GotKey(..) => panic!(),
                GotMessage(..) => S3Scared,
            },
        });

        actions
    }

    fn derive_key_and_decrypt(
        side: &TheirSide,
        key: &Key,
        phase: &Phase,
        body: &[u8],
    ) -> Option<Vec<u8>> {
        let data_key = key::derive_phase_key(&side, &key, &phase);
        key::decrypt_data(&data_key, body)
    }
}
