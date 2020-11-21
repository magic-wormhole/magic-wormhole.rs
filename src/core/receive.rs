use super::events::{Events, Key, Phase, TheirSide};
use super::key;
use log::trace;

// we process these
use super::events::ReceiveEvent;
// we emit these
use super::events::{CoreEvent};

#[derive(Debug, PartialEq)]
enum State {
    S0NoPake(Vec<(TheirSide, Phase, Vec<u8>)>), // Message queue
    S1YesPake,
    S2Unverified(Key),
    S3Verified(Key),
}

pub struct ReceiveMachine {
    state: Option<State>,
}

impl ReceiveMachine {
    pub fn new() -> ReceiveMachine {
        ReceiveMachine {
            state: Some(State::S0NoPake(Vec::new())),
        }
    }

    pub fn process(&mut self, event: ReceiveEvent) -> anyhow::Result<Events> {
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
            S0NoPake(mut queue) => match event {
                GotMessage(side, phase, body) => {
                    if phase.is_pake() {
                        // got a pake message
                        actions.push(BossEvent::ToKey(KeyEvent::GotPake(body)));
                        for (side, phase, body) in queue {
                            actions.push(ReceiveEvent::GotMessage(side, phase, body));
                        }
                        S1YesPake
                    } else {
                        // not a  pake message, queue it.
                        queue.push((side, phase, body));
                        S0NoPake(queue)
                    }
                },
                GotKey(_) => unreachable!(),
            },
            S1YesPake => match event {
                GotMessage(..) => panic!(), // TODO not sure if this is correct
                GotKey(key) => S2Unverified(key),
            },
            S2Unverified(key) => match event {
                GotKey(_) => panic!(),
                GotMessage(side, phase, body) => {
                    match Self::derive_key_and_decrypt(&side, &key, &phase, &body) {
                        Some(plaintext) => {
                            // got_message_good
                            let verifier = key::derive_verifier(&key);
                            actions.push(CoreEvent::FirstVerifiedMessage {
                                verifier,
                                key: key.clone(),
                            });
                            actions.push(CoreEvent::GotDecryptedMessage(phase, plaintext));
                            S3Verified(key)
                        },
                        None => {
                            // got_message_bad
                            anyhow::bail!("Got bad message that could not be decrypted");
                        },
                    }
                },
            },
            S3Verified(ref key) => match event {
                GotKey(_) => panic!(),
                GotMessage(side, phase, body) => {
                    match Self::derive_key_and_decrypt(&side, &key, &phase, &body) {
                        Some(plaintext) => {
                            // got_message_good
                            actions.push(CoreEvent::GotDecryptedMessage(phase, plaintext));
                            S3Verified(key.clone())
                        },
                        None => {
                            // got_message_bad
                            anyhow::bail!("Got bad message that could not be decrypted");
                        },
                    }
                },
            },
        });

        Ok(actions)
    }

    pub fn derive_key_and_decrypt(
        side: &TheirSide,
        key: &Key,
        phase: &Phase,
        body: &[u8],
    ) -> Option<Vec<u8>> {
        let data_key = key::derive_phase_key(&side, &key, &phase);
        key::decrypt_data(&data_key, body)
    }
}
