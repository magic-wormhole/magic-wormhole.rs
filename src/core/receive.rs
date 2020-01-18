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
                GotKey(key) => S1UnverifiedKey(key),
            },
            S1UnverifiedKey(ref key) => match event {
                GotKey(_) => panic!(),
                GotMessage(side, phase, body) => {
                    match Self::derive_key_and_decrypt(
                        &side, &key, &phase, &body,
                    ) {
                        Some(plaintext) => {
                            // got_message_good
                            let msg = key::derive_verifier(&key);
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

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::events::{
        BossEvent, EitherSide, ReceiveEvent::*, SendEvent, TheirSide,
    };
    use crate::core::key::{derive_phase_key, derive_verifier, encrypt_data};

    #[test]
    fn test_happy_then_scared() {
        use super::State::*;

        let masterkey = Key(b"0123456789abcdef0123456789abcdef".to_vec());
        let verifier = derive_verifier(&masterkey);
        let side1 = String::from("side1");
        let t1 = TheirSide::from(side1.clone());
        let phase1 = Phase(String::from("phase1"));
        let phasekey1 = derive_phase_key(
            &EitherSide::from(&side1[..]),
            &masterkey,
            &phase1,
        );
        let plaintext1 = b"plaintext1";
        let (_, nonce_and_ciphertext1) = encrypt_data(&phasekey1, plaintext1);

        let mut r = ReceiveMachine::new();

        let mut e = r.process(GotKey(masterkey.clone()));
        assert_eq!(e, events![]);

        if let Some(S1UnverifiedKey(ref key)) = r.state {
            assert_eq!(key.0, masterkey.0);
        } else {
            panic!();
        }

        e = r.process(GotMessage(
            t1.clone(),
            phase1.clone(),
            nonce_and_ciphertext1,
        ));
        assert_eq!(
            e,
            events![
                SendEvent::GotVerifiedKey(masterkey.clone()),
                BossEvent::Happy,
                BossEvent::GotVerifier(verifier.to_vec()),
                BossEvent::GotMessage(phase1.clone(), plaintext1.to_vec()),
            ]
        );

        // second message should only provoke GotMessage
        let phase2 = Phase(String::from("phase2"));
        let phasekey2 = derive_phase_key(
            &EitherSide::from(&side1[..]),
            &masterkey,
            &phase2,
        );
        let plaintext2 = b"plaintext2";
        let (_, nonce_and_ciphertext2) = encrypt_data(&phasekey2, plaintext2);

        e = r.process(GotMessage(
            t1.clone(),
            phase2.clone(),
            nonce_and_ciphertext2,
        ));
        assert_eq!(
            e,
            events![BossEvent::GotMessage(phase2.clone(), plaintext2.to_vec()),]
        );

        // bad message makes us Scared
        let phase3 = Phase(String::from("phase3"));
        let bad_phasekey3 = b"00112233445566778899aabbccddeeff".to_vec();
        let plaintext3 = b"plaintext3";
        let (_, nonce_and_ciphertext3) =
            encrypt_data(&bad_phasekey3, plaintext3);

        e = r.process(GotMessage(
            t1.clone(),
            phase3.clone(),
            nonce_and_ciphertext3,
        ));
        assert_eq!(e, events![BossEvent::Scared]);

        // all messages are ignored once we're Scared
        let phase4 = Phase(String::from("phase4"));
        let phasekey4 =
            derive_phase_key(&EitherSide::from(side1), &masterkey, &phase4);
        let plaintext4 = b"plaintext4";
        let (_, nonce_and_ciphertext4) = encrypt_data(&phasekey4, plaintext4);

        e = r.process(GotMessage(
            t1.clone(),
            phase4.clone(),
            nonce_and_ciphertext4,
        ));
        assert_eq!(e, events![]);
    }

    #[test]
    fn test_scared() {
        use super::State::*;

        let masterkey = Key(b"0123456789abcdef0123456789abcdef".to_vec());
        let side1 = String::from("side1");
        let t1 = TheirSide::from(side1.clone());

        let mut r = ReceiveMachine::new();

        let mut e = r.process(GotKey(masterkey.clone()));
        assert_eq!(e, events![]);

        if let Some(S1UnverifiedKey(ref key)) = r.state {
            assert_eq!(key.0, masterkey.0);
        } else {
            panic!();
        }

        // bad message makes us Scared
        let phase1 = Phase(String::from("phase1"));
        let bad_phasekey1 = b"00112233445566778899aabbccddeeff".to_vec();
        let plaintext1 = b"plaintext1";
        let (_, nonce_and_ciphertext1) =
            encrypt_data(&bad_phasekey1, plaintext1);

        e = r.process(GotMessage(
            t1.clone(),
            phase1.clone(),
            nonce_and_ciphertext1,
        ));
        assert_eq!(e, events![BossEvent::Scared]);

        // all messages are ignored once we're Scared
        let phase2 = Phase(String::from("phase2"));
        let phasekey2 =
            derive_phase_key(&EitherSide::from(side1), &masterkey, &phase2);
        let plaintext2 = b"plaintext2";
        let (_, nonce_and_ciphertext2) = encrypt_data(&phasekey2, plaintext2);

        e = r.process(GotMessage(
            t1.clone(),
            phase2.clone(),
            nonce_and_ciphertext2,
        ));
        assert_eq!(e, events![]);
    }
}
