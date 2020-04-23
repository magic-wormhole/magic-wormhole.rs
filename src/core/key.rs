use hkdf::Hkdf;
use serde_derive::{Deserialize, Serialize};
use serde_json::json;
use serde_json::{self, Value};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, SPAKE2};
use xsalsa20poly1305::{
    aead::{
        generic_array::{typenum::Unsigned, GenericArray},
        Aead, NewAead,
    },
    XSalsa20Poly1305,
};
use zeroize::Zeroizing;

use super::events::{AppID, Code, EitherSide, Events, Key, MySide, Phase};
use super::util;
// we process these
use super::events::KeyEvent;
// we emit these
use super::events::BossEvent::GotKey as B_GotKey;
use super::events::MailboxEvent::AddMessage as M_AddMessage;
use super::events::ReceiveEvent::GotKey as R_GotKey;
use super::timing::new_timelog;

#[derive(Debug, PartialEq, Eq)]
enum State {
    S0KnowNothing,
    S1KnowCode(SPAKE2<Ed25519Group>), // pake_state
    S2KnowPake(Vec<u8>),              // their_pake
    S3KnowBoth(Key),                  // key
    #[allow(dead_code)] // TODO: if PAKE is somehow bad, land here
    S4Scared,
}

pub struct KeyMachine {
    appid: AppID,
    side: MySide,
    state: Option<State>,
}

#[derive(Serialize, Deserialize, Debug)]
struct PhaseMessage {
    pake_v1: String,
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

impl KeyMachine {
    pub fn new(appid: &AppID, side: &MySide) -> KeyMachine {
        KeyMachine {
            appid: appid.clone(),
            state: Some(State::S0KnowNothing),
            side: side.clone(),
        }
    }

    fn start(&self, code: Code, actions: &mut Events) -> SPAKE2<Ed25519Group> {
        let mut t1 = new_timelog("pake1", None);
        t1.detail("waiting", "crypto");
        let (pake_state, pake_msg_ser) = make_pake(&code, &self.appid);
        t1.finish(None);
        actions.push(t1);
        actions.push(M_AddMessage(Phase(String::from("pake")), pake_msg_ser));
        pake_state
    }

    fn finish(
        &self,
        pake_state: SPAKE2<Ed25519Group>,
        their_pake_msg: &[u8],
        actions: &mut Events,
    ) -> Key {
        let mut t2 = new_timelog("pake2", None);
        t2.detail("waiting", "crypto");
        let msg2 = extract_pake_msg(&their_pake_msg).unwrap();
        let key = Key(pake_state.finish(&hex::decode(msg2).unwrap()).unwrap());
        t2.finish(None);
        actions.push(t2);
        let versions = json!({"app_versions": {}}); // TODO: self.versions
        let (version_phase, version_msg) =
            build_version_msg(&self.side, &key, &versions);
        actions.push(M_AddMessage(version_phase, version_msg));
        actions.push(B_GotKey(key.clone()));
        actions.push(R_GotKey(key.clone()));
        key
    }

    pub fn process(&mut self, event: KeyEvent) -> Events {
        /*trace!(
            "key: current state = {:?}, got event = {:?}",
            self.state, event
        );*/

        use self::KeyEvent::*;
        use self::State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0KnowNothing => match event {
                GotCode(code) => {
                    let pake_state = self.start(code, &mut actions);
                    S1KnowCode(pake_state)
                }
                GotPake(pake) => S2KnowPake(pake),
            },
            S1KnowCode(pake_state) => match event {
                GotCode(_) => panic!("already got code"),
                GotPake(their_pake_msg) => {
                    let key =
                        self.finish(pake_state, &their_pake_msg, &mut actions);
                    S3KnowBoth(key)
                }
            },
            S2KnowPake(ref their_pake_msg) => match event {
                GotCode(code) => {
                    let pake_state = self.start(code, &mut actions);
                    let key =
                        self.finish(pake_state, &their_pake_msg, &mut actions);
                    S3KnowBoth(key)
                }
                GotPake(_) => panic!("already got pake"),
            },
            S3KnowBoth(_) => match event {
                GotCode(_) => panic!("already got code"),
                GotPake(_) => panic!("already got pake"),
            },
            S4Scared => panic!("already scared"),
        });

        actions
    }
}

fn build_version_msg(
    side: &MySide,
    key: &Key,
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
        .and_then(|res: PhaseMessage| Ok(res.pake_v1))
        .ok()
}

fn encrypt_data_with_nonce(
    key: &[u8],
    plaintext: &[u8],
    noncebuf: &[u8],
) -> Vec<u8> {
    let cipher = XSalsa20Poly1305::new(*GenericArray::from_slice(&key));
    let mut ciphertext = cipher
        .encrypt(GenericArray::from_slice(&noncebuf), plaintext)
        .unwrap();
    let mut nonce_and_ciphertext = vec![];
    nonce_and_ciphertext.append(&mut Vec::from(noncebuf));
    nonce_and_ciphertext.append(&mut ciphertext);
    nonce_and_ciphertext
}

pub fn encrypt_data(key: &[u8], plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut noncebuf: GenericArray<u8, <XSalsa20Poly1305 as Aead>::NonceSize> =
        GenericArray::default();
    util::random_bytes(&mut noncebuf);
    let nonce_and_ciphertext =
        encrypt_data_with_nonce(key, plaintext, &noncebuf);
    (noncebuf.to_vec(), nonce_and_ciphertext)
}

// TODO: return a Result with a proper error type
pub fn decrypt_data(key: &[u8], encrypted: &[u8]) -> Option<Vec<u8>> {
    let nonce_size = <XSalsa20Poly1305 as Aead>::NonceSize::to_usize();
    let (nonce, ciphertext) = encrypted.split_at(nonce_size);
    assert_eq!(nonce.len(), nonce_size);
    let cipher = XSalsa20Poly1305::new(*GenericArray::from_slice(key));
    cipher
        .decrypt(GenericArray::from_slice(nonce), ciphertext)
        .ok()
}

fn sha256_digest(input: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::default();
    hasher.input(input);
    hasher.result().to_vec()
}

fn derive_key(key: &[u8], purpose: &[u8], length: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(None, key);
    let mut v = vec![0; length];
    hk.expand(purpose, &mut v).unwrap();
    v
}

pub fn derive_phase_key(
    side: &EitherSide,
    key: &Key,
    phase: &Phase,
) -> Zeroizing<Vec<u8>> {
    let side_bytes = side.0.as_bytes();
    let phase_bytes = phase.0.as_bytes();
    let side_digest: Vec<u8> = sha256_digest(side_bytes);
    let phase_digest: Vec<u8> = sha256_digest(phase_bytes);
    let mut purpose_vec: Vec<u8> = b"wormhole:phase:".to_vec();
    purpose_vec.extend(side_digest);
    purpose_vec.extend(phase_digest);

    let length = <XSalsa20Poly1305 as NewAead>::KeySize::to_usize();
    Zeroizing::new(derive_key(&key.to_vec(), &purpose_vec, length))
}

pub fn derive_verifier(key: &Key) -> Vec<u8> {
    // TODO: replace 32 with KEY_SIZE const
    derive_key(key, b"wormhole:verifier", 32)
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::events::{AppID, EitherSide, Event, KeyEvent};
    use hex;

    #[test]
    fn test_extract_pake_msg() {
        let _key = super::KeyMachine::new(
            &AppID(String::from("appid")),
            &MySide::unchecked_from_string(String::from("side1")),
        );

        let s1 = "7b2270616b655f7631223a22353337363331646366643064336164386130346234663531643935336131343563386538626663373830646461393834373934656634666136656536306339663665227d";
        let pake_msg = super::extract_pake_msg(&hex::decode(s1).unwrap());
        assert_eq!(pake_msg, Some(String::from("537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e")));
    }

    #[test]
    fn test_derive_key() {
        let main = hex::decode(
            "588ba9eef353778b074413a0140205d90d7479e36e0dd4ee35bb729d26131ef1",
        )
        .unwrap();
        let dk1 = derive_key(&main, b"purpose1", 32);
        assert_eq!(
            hex::encode(dk1),
            "835b5df80ce9ca46908e8524fb308649122cfbcefbeaa7e65061c6ef08ee1b2a"
        );

        let dk2 = derive_key(&main, b"purpose2", 10);
        assert_eq!(hex::encode(dk2), "f2238e84315b47eb6279");
    }

    #[test]
    fn test_derive_phase_key() {
        let main = Key(hex::decode(
            "588ba9eef353778b074413a0140205d90d7479e36e0dd4ee35bb729d26131ef1",
        )
        .unwrap());
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
        let _k = KeyMachine::new(
            &AppID(String::from("appid1")),
            &MySide::unchecked_from_string(String::from("side")),
        );

        let key = Key("key".as_bytes().to_vec());
        let side = "side";
        let phase = Phase(String::from("phase1"));
        let phase1_key =
            derive_phase_key(&EitherSide::from(side), &key, &phase);

        assert_eq!(
            hex::encode(&*phase1_key),
            "fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96"
        );
    }

    #[test]
    fn test_encrypt_data() {
        let k = hex::decode(
            "ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679",
        )
        .unwrap();
        let plaintext =
            hex::decode("edc089a518219ec1cee184e89d2d37af").unwrap();
        assert_eq!(plaintext.len(), 16);
        let nonce =
            hex::decode("2d5e43eb465aa42e750f991e425bee485f06abad7e04af80")
                .unwrap();
        assert_eq!(nonce.len(), 24);
        let msg = encrypt_data_with_nonce(&k, &plaintext, &nonce);
        assert_eq!(hex::encode(msg), "2d5e43eb465aa42e750f991e425bee485f06abad7e04af80fe318e39d0e4ce932d2b54b300c56d2cda55ee5f0488d63eb1d5f76f7919a49a");
    }

    #[test]
    fn test_decrypt_data() {
        let k = hex::decode(
            "ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679",
        )
        .unwrap();
        let encrypted = hex::decode("2d5e43eb465aa42e750f991e425bee485f06abad7e04af80fe318e39d0e4ce932d2b54b300c56d2cda55ee5f0488d63eb1d5f76f7919a49a").unwrap();
        match decrypt_data(&k, &encrypted) {
            Some(plaintext) => {
                assert_eq!(
                    hex::encode(plaintext),
                    "edc089a518219ec1cee184e89d2d37af"
                );
            }
            None => {
                panic!("failed to decrypt");
            }
        };
    }

    #[test]
    fn test_encrypt_data_decrypt_data_roundtrip() {
        let key = Key("key".as_bytes().to_vec());
        let side = "side";
        let phase = Phase(String::from("phase"));
        let data_key = derive_phase_key(&EitherSide::from(side), &key, &phase);
        let plaintext = "hello world";

        let (_nonce, encrypted) =
            encrypt_data(&data_key, &plaintext.as_bytes());
        let maybe_plaintext = decrypt_data(&data_key, &encrypted);
        match maybe_plaintext {
            Some(plaintext_decrypted) => {
                assert_eq!(plaintext.as_bytes().to_vec(), plaintext_decrypted);
            }
            None => panic!(),
        }
    }

    fn strip_timing(events: Events) -> Vec<Event> {
        events
            .into_iter()
            .filter(|e| match e {
                Event::Timing(_) => false,
                _ => true,
            })
            .collect()
    }

    #[test]
    fn test_code_first() {
        use super::super::events::{
            BossEvent, MailboxEvent, Phase, ReceiveEvent,
        };

        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);

        // we set our own code first, which generates+sends a PAKE message
        let mut e = strip_timing(k.process(KeyEvent::GotCode(code.clone())));

        match e.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(phase, body)) => {
                assert_eq!(phase, Phase(String::from("pake")));
                assert!(String::from_utf8(body)
                    .unwrap()
                    .contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
        assert_eq!(e.len(), 0);

        // build a PAKE message to simulate our peer
        let (_pake_state, pake_msg_ser) = make_pake(&code, &appid);

        // deliver it, which should finish the key-agreement process
        let mut e = strip_timing(k.process(KeyEvent::GotPake(pake_msg_ser)));
        match e.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(phase, _body)) => {
                assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
        let shared_key = match e.remove(0) {
            Event::Boss(BossEvent::GotKey(key)) => {
                //assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
                key
            }
            _ => panic!(),
        };
        match e.remove(0) {
            Event::Receive(ReceiveEvent::GotKey(rkey)) => {
                assert_eq!(shared_key, rkey);
                //assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_pake_first() {
        use super::super::events::{
            BossEvent, MailboxEvent, Phase, ReceiveEvent,
        };

        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);

        // build a PAKE message to simulate our peer
        let (_pake_state, pake_msg_ser) = make_pake(&code, &appid);

        // we receive the PAKE from our peer before the user finishes
        // providing our code, so we emit no messages
        let e = strip_timing(k.process(KeyEvent::GotPake(pake_msg_ser)));
        assert_eq!(e.len(), 0);

        // setting our own code should both start and finish the process
        let mut e = strip_timing(k.process(KeyEvent::GotCode(code.clone())));

        match e.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(phase, body)) => {
                assert_eq!(phase, Phase(String::from("pake")));
                assert!(String::from_utf8(body)
                    .unwrap()
                    .contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
        match e.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(phase, _body)) => {
                assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
        let shared_key = match e.remove(0) {
            Event::Boss(BossEvent::GotKey(key)) => {
                //assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
                key
            }
            _ => panic!(),
        };
        match e.remove(0) {
            Event::Receive(ReceiveEvent::GotKey(rkey)) => {
                assert_eq!(shared_key, rkey);
                //assert_eq!(phase, Phase(String::from("version")));
                //assert!(String::from_utf8(body).unwrap().contains("{\"pake_v1\":"));
            }
            _ => panic!(),
        }
    }

    #[test]
    #[should_panic]
    fn test_pake_pake() {
        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);

        let (_pake_state, pake_msg_ser) = make_pake(&code, &appid);
        k.process(KeyEvent::GotPake(pake_msg_ser.clone()));
        k.process(KeyEvent::GotPake(pake_msg_ser));
    }

    #[test]
    #[should_panic]
    fn test_code_code() {
        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);

        k.process(KeyEvent::GotCode(code.clone()));
        k.process(KeyEvent::GotCode(code));
    }

    #[test]
    #[should_panic]
    fn test_code_pake_code() {
        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);
        let (_pake_state, pake_msg_ser) = make_pake(&code, &appid);

        k.process(KeyEvent::GotCode(code.clone()));
        k.process(KeyEvent::GotPake(pake_msg_ser));
        k.process(KeyEvent::GotCode(code));
    }

    #[test]
    #[should_panic]
    fn test_pake_code_pake() {
        let code = Code(String::from("4-purple-sausages"));
        let appid = AppID(String::from("appid1"));
        let side = MySide::unchecked_from_string(String::from("side"));
        let mut k = KeyMachine::new(&appid, &side);
        let (_pake_state, pake_msg_ser) = make_pake(&code, &appid);

        k.process(KeyEvent::GotPake(pake_msg_ser.clone()));
        k.process(KeyEvent::GotCode(code));
        k.process(KeyEvent::GotPake(pake_msg_ser));
    }
}
