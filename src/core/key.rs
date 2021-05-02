use crate::{
    core::{server_messages::OutboundMessage, EncryptedMessage, Event, Mailbox, Mood, Nameplate},
    APIEvent,
};
use hkdf::Hkdf;
use serde_derive::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use sha2::{digest::FixedOutput, Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, SPAKE2};
use std::collections::VecDeque;
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::{
    aead::{generic_array::GenericArray, Aead, AeadCore, NewAead},
    XSalsa20Poly1305,
};

use super::{
    events::{AppID, Code, EitherSide, MySide, Phase},
    mailbox, util,
};

#[derive(Debug, PartialEq, derive_more::Display)]
enum State {
    #[display(
        fmt = "S1NoPake {{ pake: <censored>, message_queue: <{} items> }}",
        "_1.len()"
    )]
    S1NoPake(SPAKE2<Ed25519Group>, Vec<EncryptedMessage>), // pake_state, message queue
    #[display(
        fmt = "S2Unverified {{ key: <censored>, message_queue: <{} items> }}",
        "_1.len()"
    )]
    S2Unverified(xsalsa20poly1305::Key, Vec<EncryptedMessage>), // key, another message queue
}

#[derive(Debug, derive_more::Display)]
#[display(
    fmt = "KeyMachine {{ side: {}, versions: {}, nameplate: {:?}, mailbox_machine: {}, state: {} }}",
    side,
    versions,
    nameplate,
    mailbox_machine,
    state
)]
pub(super) struct KeyMachine {
    side: MySide,
    versions: serde_json::Value,
    pub nameplate: Option<Nameplate>,
    mailbox_machine: mailbox::MailboxMachine,
    state: State,
}

#[derive(Serialize, Deserialize, Debug)]
struct PhaseMessage {
    pake_v1: String,
}

impl KeyMachine {
    pub fn start(
        actions: &mut VecDeque<Event>,
        appid: &AppID,
        side: MySide,
        versions: serde_json::Value,
        nameplate: Nameplate,
        mailbox: Mailbox,
        code: &Code,
    ) -> KeyMachine {
        let (pake_state, pake_msg_ser) = make_pake(code, &appid);
        let mut mailbox_machine = mailbox::MailboxMachine::new(&side, mailbox);
        mailbox_machine.send_message(actions, Phase(String::from("pake")), pake_msg_ser);

        KeyMachine {
            versions,
            state: State::S1NoPake(pake_state, Vec::new()),
            side,
            mailbox_machine,
            nameplate: Some(nameplate),
        }
    }

    pub(super) fn receive_message(
        mut self: Box<Self>,
        actions: &mut VecDeque<Event>,
        message: EncryptedMessage,
    ) -> super::State {
        if !self.mailbox_machine.receive_message(&message) {
            return super::State::Keying(self);
        }

        match self.state {
            State::S1NoPake(pake_state, mut queue) => {
                if message.phase.is_pake() {
                    // got a pake message, derive key
                    // TODO error handling
                    let pake_message = extract_pake_msg(&message.body).unwrap();
                    let key = *secretbox::Key::from_slice(
                        &pake_state
                            .finish(&hex::decode(pake_message).unwrap())
                            .unwrap(),
                    );

                    // Send versions message
                    let versions = json!({"app_versions": self.versions});
                    let (version_phase, version_msg) =
                        build_version_msg(&self.side, &key, &versions);
                    self.mailbox_machine
                        .send_message(actions, version_phase, version_msg);

                    // Release all queued messages
                    for message in queue {
                        actions.push_back(Event::BounceMessage(message));
                    }

                    self.state = State::S2Unverified(key, Vec::new());
                } else {
                    // not a  pake message, queue it.
                    queue.push(message);
                    self.state = State::S1NoPake(pake_state, queue);
                }
            },
            State::S2Unverified(key, mut queue) => {
                if message.phase.is_version() {
                    match message.decrypt(&key) {
                        Ok(plaintext) => {
                            // Handle received message
                            // TODO handle error conditions
                            let version_str = String::from_utf8(plaintext).unwrap();
                            let v: Value = serde_json::from_str(&version_str).unwrap();
                            let app_versions = match v.get("app_versions") {
                                Some(versions) => versions.clone(),
                                None => serde_json::json!({}),
                            };

                            // Release old things
                            if let Some(nameplate) = self.nameplate {
                                actions.push_back(OutboundMessage::release(nameplate).into());
                            }
                            for message in queue {
                                actions.push_back(Event::BounceMessage(message));
                            }

                            // We are now fully initialized! Up and running! :tada:
                            actions.push_back(
                                APIEvent::ConnectedToClient {
                                    verifier: Box::new(derive_verifier(&key)),
                                    key: Box::new(key),
                                    versions: app_versions,
                                }
                                .into(),
                            );
                            return super::State::Running(Box::new(
                                super::running::RunningMachine {
                                    phase: 0,
                                    key,
                                    side: self.side,
                                    mailbox_machine: self.mailbox_machine,
                                    await_nameplate_release: true,
                                },
                            ));
                        },
                        Err(error) => {
                            actions.push_back(Event::ShutDown(Err(error)));
                            self.state = State::S2Unverified(key, queue);
                        },
                    }
                } else {
                    queue.push(message);
                    self.state = State::S2Unverified(key, queue);
                }
            },
        }

        super::State::Keying(self)
    }

    pub(super) fn shutdown(
        self,
        actions: &mut VecDeque<Event>,
        result: anyhow::Result<()>,
    ) -> super::State {
        self.mailbox_machine.close(
            actions,
            if result.is_ok() {
                Mood::Lonely
            } else {
                Mood::Errory
            },
        );
        let await_nameplate_release = if let Some(nameplate) = self.nameplate {
            actions.push_back(OutboundMessage::release(nameplate).into());
            true
        } else {
            false
        };
        super::State::Closing {
            await_nameplate_release,
            await_mailbox_close: true,
            result,
        }
    }
}

fn make_pake(code: &Code, appid: &AppID) -> (SPAKE2<Ed25519Group>, Vec<u8>) {
    let (pake_state, msg1) = SPAKE2::<Ed25519Group>::start_symmetric(
        &Password::new(code.as_bytes()),
        &Identity::new(appid.0.as_bytes()),
    );
    let payload = util::bytes_to_hexstr(&msg1);
    let pake_msg = PhaseMessage { pake_v1: payload };
    let pake_msg_ser = serde_json::to_vec(&pake_msg).unwrap();
    (pake_state, pake_msg_ser)
}

fn build_version_msg(
    side: &MySide,
    key: &xsalsa20poly1305::Key,
    versions: &Value,
) -> (Phase, Vec<u8>) {
    let phase = Phase(String::from("version"));
    let data_key = derive_phase_key(&side, &key, &phase);
    let plaintext = versions.to_string();
    let (_nonce, encrypted) = encrypt_data(&data_key, &plaintext.as_bytes());
    (phase, encrypted)
}

fn extract_pake_msg(body: &[u8]) -> Option<String> {
    serde_json::from_slice(&body)
        .map(|res: PhaseMessage| res.pake_v1)
        .ok()
}

fn encrypt_data_with_nonce(
    key: &xsalsa20poly1305::Key,
    plaintext: &[u8],
    nonce: &xsalsa20poly1305::Nonce,
) -> Vec<u8> {
    let cipher = XSalsa20Poly1305::new(GenericArray::from_slice(&key));
    let mut ciphertext = cipher.encrypt(nonce, plaintext).unwrap();
    let mut nonce_and_ciphertext = vec![];
    nonce_and_ciphertext.extend_from_slice(&nonce);
    nonce_and_ciphertext.append(&mut ciphertext);
    nonce_and_ciphertext
}

pub fn encrypt_data(
    key: &xsalsa20poly1305::Key,
    plaintext: &[u8],
) -> (xsalsa20poly1305::Nonce, Vec<u8>) {
    let nonce = xsalsa20poly1305::generate_nonce(&mut rand::thread_rng());
    let nonce_and_ciphertext = encrypt_data_with_nonce(key, plaintext, &nonce);
    (nonce, nonce_and_ciphertext)
}

// TODO: return a Result with a proper error type
pub fn decrypt_data(key: &xsalsa20poly1305::Key, encrypted: &[u8]) -> Option<Vec<u8>> {
    use xsalsa20poly1305::aead::generic_array::typenum::marker_traits::Unsigned;
    let nonce_size = <XSalsa20Poly1305 as AeadCore>::NonceSize::to_usize();
    let (nonce, ciphertext) = encrypted.split_at(nonce_size);
    assert_eq!(nonce.len(), nonce_size);
    let cipher = XSalsa20Poly1305::new(GenericArray::from_slice(key));
    cipher
        .decrypt(GenericArray::from_slice(nonce), ciphertext)
        .ok()
}

fn sha256_digest(input: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::default();
    hasher.update(input);
    hasher.finalize_fixed().to_vec()
}

pub fn derive_key(key: &xsalsa20poly1305::Key, purpose: &[u8]) -> xsalsa20poly1305::Key {
    let hk = Hkdf::<Sha256>::new(None, key);
    let mut key = xsalsa20poly1305::Key::default();
    hk.expand(purpose, &mut key).unwrap();
    key
}

pub fn derive_phase_key(
    side: &EitherSide,
    key: &xsalsa20poly1305::Key,
    phase: &Phase,
) -> xsalsa20poly1305::Key {
    let side_digest: Vec<u8> = sha256_digest(side.0.as_bytes());
    let phase_digest: Vec<u8> = sha256_digest(phase.0.as_bytes());
    let mut purpose_vec: Vec<u8> = b"wormhole:phase:".to_vec();
    purpose_vec.extend(side_digest);
    purpose_vec.extend(phase_digest);

    derive_key(&key, &purpose_vec)
}

pub fn derive_verifier(key: &xsalsa20poly1305::Key) -> xsalsa20poly1305::Key {
    derive_key(&key, b"wormhole:verifier")
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::core::events::EitherSide;

    #[test]
    fn test_extract_pake_msg() {
        // let _key = super::KeyMachine::new(
        //     &AppID::new("appid"),
        //     &MySide::unchecked_from_string(String::from("side1")),
        //     json!({}),
        // );

        let s1 = "7b2270616b655f7631223a22353337363331646366643064336164386130346234663531643935336131343563386538626663373830646461393834373934656634666136656536306339663665227d";
        let pake_msg = super::extract_pake_msg(&hex::decode(s1).unwrap());
        assert_eq!(
            pake_msg,
            Some(String::from(
                "537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e"
            ))
        );
    }

    #[test]
    fn test_derive_key() {
        let main = xsalsa20poly1305::Key::from_exact_iter(
            hex::decode("588ba9eef353778b074413a0140205d90d7479e36e0dd4ee35bb729d26131ef1")
                .unwrap(),
        )
        .unwrap();
        let dk1 = derive_key(&main, b"purpose1");
        assert_eq!(
            hex::encode(dk1),
            "835b5df80ce9ca46908e8524fb308649122cfbcefbeaa7e65061c6ef08ee1b2a"
        );

        /* The API doesn't support non-standard length keys anymore.
         * But we may want to add that back in in the future.
         */
        // let dk2 = derive_key(&main, b"purpose2", 10);
        // assert_eq!(hex::encode(dk2), "f2238e84315b47eb6279");
    }

    #[test]
    fn test_derive_phase_key() {
        let main = xsalsa20poly1305::Key::from_exact_iter(
            hex::decode("588ba9eef353778b074413a0140205d90d7479e36e0dd4ee35bb729d26131ef1")
                .unwrap(),
        )
        .unwrap();
        let dk11 = derive_phase_key(
            &EitherSide::from("side1"),
            &main,
            &Phase(String::from("phase1")),
        );
        assert_eq!(
            hex::encode(&*dk11),
            "3af6a61d1a111225cc8968c6ca6265efe892065c3ab46de79dda21306b062990"
        );
        let dk12 = derive_phase_key(
            &EitherSide::from("side1"),
            &main,
            &Phase(String::from("phase2")),
        );
        assert_eq!(
            hex::encode(&*dk12),
            "88a1dd12182d989ff498022a9656d1e2806f17328d8bf5d8d0c9753e4381a752"
        );
        let dk21 = derive_phase_key(
            &EitherSide::from("side2"),
            &main,
            &Phase(String::from("phase1")),
        );
        assert_eq!(
            hex::encode(&*dk21),
            "a306627b436ec23bdae3af8fa90c9ac927780d86be1831003e7f617c518ea689"
        );
        let dk22 = derive_phase_key(
            &EitherSide::from("side2"),
            &main,
            &Phase(String::from("phase2")),
        );
        assert_eq!(
            hex::encode(&*dk22),
            "bf99e3e16420f2dad33f9b1ccb0be1462b253d639dacdb50ed9496fa528d8758"
        );
    }

    #[test]
    fn test_derive_phase_key2() {
        // feed python's derive_phase_key with these inputs:
        // key = b"key"
        // side = u"side"
        // phase = u"phase1"
        // output of derive_phase_key is:
        // "\xfe\x93\x15r\x96h\xa6'\x8a\x97D\x9d\xc9\x9a_L!\x02\xa6h\xc6\x8538\x15)\x06\xbbuRj\x96"
        // hexlified output: fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96
        // let _k = KeyMachine::new(
        //     &AppID::new("appid1"),
        //     &MySide::unchecked_from_string(String::from("side")),
        //     json!({}),
        // );

        /* This test is disabled for now because the used key length is not compatible with our API */
        // let key = Key(b"key".to_vec());
        // let side = "side";
        // let phase = Phase(String::from("phase1"));
        // let phase1_key = derive_phase_key(&EitherSide::from(side), &key, &phase);

        // assert_eq!(
        //     hex::encode(&*phase1_key),
        //     "fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96"
        // );
    }

    #[test]
    fn test_encrypt_data() {
        let k = xsalsa20poly1305::Key::from_exact_iter(
            hex::decode("ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679")
                .unwrap(),
        )
        .unwrap();
        let plaintext = hex::decode("edc089a518219ec1cee184e89d2d37af").unwrap();
        assert_eq!(plaintext.len(), 16);
        let nonce = xsalsa20poly1305::Nonce::from_exact_iter(
            hex::decode("2d5e43eb465aa42e750f991e425bee485f06abad7e04af80").unwrap(),
        )
        .unwrap();
        assert_eq!(nonce.len(), 24);
        let msg = encrypt_data_with_nonce(&k, &plaintext, &nonce);
        assert_eq!(hex::encode(msg), "2d5e43eb465aa42e750f991e425bee485f06abad7e04af80fe318e39d0e4ce932d2b54b300c56d2cda55ee5f0488d63eb1d5f76f7919a49a");
    }

    #[test]
    fn test_decrypt_data() {
        let k = xsalsa20poly1305::Key::from_exact_iter(
            hex::decode("ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679")
                .unwrap(),
        )
        .unwrap();
        let encrypted = hex::decode("2d5e43eb465aa42e750f991e425bee485f06abad7e04af80fe318e39d0e4ce932d2b54b300c56d2cda55ee5f0488d63eb1d5f76f7919a49a").unwrap();
        match decrypt_data(&k, &encrypted) {
            Some(plaintext) => {
                assert_eq!(hex::encode(plaintext), "edc089a518219ec1cee184e89d2d37af");
            },
            None => {
                panic!("failed to decrypt");
            },
        };
    }

    /* This test is disabled for now because the used key length is not compatible with our API */
    // #[test]
    // fn test_encrypt_data_decrypt_data_roundtrip() {
    //     let key = Key(b"key".to_vec());
    //     let side = "side";
    //     let phase = Phase(String::from("phase"));
    //     let data_key = derive_phase_key(&EitherSide::from(side), &key, &phase);
    //     let plaintext = "hello world";

    //     let (_nonce, encrypted) = encrypt_data(&data_key, &plaintext.as_bytes());
    //     let maybe_plaintext = decrypt_data(&data_key, &encrypted);
    //     match maybe_plaintext {
    //         Some(plaintext_decrypted) => {
    //             assert_eq!(plaintext.as_bytes().to_vec(), plaintext_decrypted);
    //         },
    //         None => panic!(),
    //     }
    // }
}
