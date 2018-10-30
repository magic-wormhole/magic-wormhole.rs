extern crate hex;

use hkdf::Hkdf;
use serde_json::{self, Value};
use sha2::{Digest, Sha256};
use sodiumoxide;
use sodiumoxide::crypto::secretbox;
use spake2::{Ed25519Group, Identity, Password, SPAKE2};
use std::mem;

use super::events::{AppID, Code, Events, Key, MySide, Phase};
use super::util;
// we process these
use super::events::KeyEvent;
// we emit these
use super::events::BossEvent::GotKey as B_GotKey;
use super::events::MailboxEvent::AddMessage as M_AddMessage;
use super::events::ReceiveEvent::GotKey as R_GotKey;

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

impl KeyMachine {
    pub fn new(appid: &AppID, side: &MySide) -> KeyMachine {
        KeyMachine {
            appid: appid.clone(),
            state: Some(State::S0KnowNothing),
            side: side.clone(),
        }
    }

    pub fn process(&mut self, event: KeyEvent) -> Events {
        /*println!(
            "key: current state = {:?}, got event = {:?}",
            self.state, event
        );*/

        use self::KeyEvent::*;
        match event {
            GotCode(code) => self.got_code(&code),
            GotPake(pake) => self.got_pake(pake),
        }
    }

    fn got_code(&mut self, code: &Code) -> Events {
        use self::State::*;
        let oldstate = mem::replace(&mut self.state, None);
        match oldstate.unwrap() {
            S0KnowNothing => {
                let (pake_state, pake_msg_ser) = start_pake(&code, &self.appid);
                self.state = Some(S1KnowCode(pake_state));
                events![M_AddMessage(Phase("pake".to_string()), pake_msg_ser)]
            }
            S1KnowCode(_) => panic!("already got code"),
            S2KnowPake(ref their_pake_msg) => {
                let (pake_state, pake_msg_ser) = start_pake(&code, &self.appid);
                let key: Key = finish_pake(pake_state, &their_pake_msg);
                let versions = json!({"app_versions": {}}); // TODO: self.versions
                let (version_phase, version_msg) =
                    build_version_msg(&self.side, &key, &versions);
                self.state = Some(S3KnowBoth(key.clone()));
                events![
                    M_AddMessage(Phase("pake".to_string()), pake_msg_ser),
                    M_AddMessage(version_phase, version_msg),
                    B_GotKey(key.clone()),
                    R_GotKey(key.clone())
                ]
            }
            S3KnowBoth(_) => panic!("already got code"),
            S4Scared => panic!("already scared"),
        }
    }

    fn got_pake(&mut self, pake: Vec<u8>) -> Events {
        use self::State::*;
        let oldstate = mem::replace(&mut self.state, None);
        match oldstate.unwrap() {
            S0KnowNothing => {
                self.state = Some(S2KnowPake(pake));
                events![]
            }
            S1KnowCode(pake_state) => {
                let key: Key = finish_pake(pake_state, &pake);
                let versions = json!({"app_versions": {}}); // TODO: self.versions
                let (version_phase, version_msg) =
                    build_version_msg(&self.side, &key, &versions);
                self.state = Some(S3KnowBoth(key.clone()));
                events![
                    M_AddMessage(version_phase, version_msg),
                    B_GotKey(key.clone()),
                    R_GotKey(key.clone())
                ]
            }
            S2KnowPake(_) => panic!("already got pake"),
            S3KnowBoth(_) => panic!("already got pake"),
            S4Scared => panic!("already scared"),
        }
    }
}

fn start_pake(code: &Code, appid: &AppID) -> (SPAKE2<Ed25519Group>, Vec<u8>) {
    let (pake_state, msg1) = SPAKE2::<Ed25519Group>::start_symmetric(
        &Password::new(code.as_bytes()),
        &Identity::new(appid.as_bytes()),
    );
    let payload = util::bytes_to_hexstr(&msg1);
    let pake_msg = PhaseMessage { pake_v1: payload };
    let pake_msg_ser = serde_json::to_vec(&pake_msg).unwrap();
    (pake_state, pake_msg_ser)
}

fn finish_pake(pake_state: SPAKE2<Ed25519Group>, peer_msg: &[u8]) -> Key {
    let msg2 = extract_pake_msg(&peer_msg).unwrap();
    Key(pake_state.finish(&hex::decode(msg2).unwrap()).unwrap())
}

fn build_version_msg(
    side: &MySide,
    key: &Key,
    versions: &Value,
) -> (Phase, Vec<u8>) {
    let phase = "version";
    let data_key = derive_phase_key(&side.to_string(), &key, &phase);
    let plaintext = versions.to_string();
    let (_nonce, encrypted) = encrypt_data(&data_key, &plaintext.as_bytes());
    (Phase(phase.to_string()), encrypted)
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
    let nonce = secretbox::Nonce::from_slice(&noncebuf).unwrap();
    let sodium_key = secretbox::Key::from_slice(&key).unwrap();
    let ciphertext = secretbox::seal(&plaintext, &nonce, &sodium_key);
    let mut nonce_and_ciphertext = Vec::new();
    nonce_and_ciphertext.extend(nonce.as_ref().to_vec());
    nonce_and_ciphertext.extend(ciphertext);
    nonce_and_ciphertext
}

pub fn encrypt_data(key: &[u8], plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let noncebuf = secretbox::gen_nonce().as_ref().to_vec();
    let nonce_and_ciphertext =
        encrypt_data_with_nonce(key, plaintext, &noncebuf);
    (noncebuf, nonce_and_ciphertext)
}

// TODO: return an Result with a proper error type
// secretbox::open() returns Result<Vec<u8>, ()> which is not helpful.
pub fn decrypt_data(key: &[u8], encrypted: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ciphertext) =
        encrypted.split_at(sodiumoxide::crypto::secretbox::NONCEBYTES);
    assert_eq!(nonce.len(), sodiumoxide::crypto::secretbox::NONCEBYTES);
    secretbox::open(
        &ciphertext,
        &secretbox::Nonce::from_slice(nonce).unwrap(),
        &secretbox::Key::from_slice(&key).unwrap(),
    ).ok()
}

fn sha256_digest(input: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::default();
    hasher.input(input);
    hasher.result().to_vec()
}

pub fn derive_key(key: &[u8], purpose: &[u8], length: usize) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::extract(None, key);
    let mut v = vec![0; length];
    hk.expand(purpose, &mut v).unwrap();
    v
}

pub fn derive_phase_key(side: &str, key: &Key, phase: &str) -> Vec<u8> {
    let side_bytes = side.as_bytes();
    let phase_bytes = phase.as_bytes();
    let side_digest: Vec<u8> = sha256_digest(side_bytes);
    let phase_digest: Vec<u8> = sha256_digest(phase_bytes);
    let mut purpose_vec: Vec<u8> = b"wormhole:phase:".to_vec();
    purpose_vec.extend(side_digest);
    purpose_vec.extend(phase_digest);

    let length = sodiumoxide::crypto::secretbox::KEYBYTES;
    derive_key(&key.to_vec(), &purpose_vec, length)
}

#[cfg(test)]
mod test {
    use core::events::{AppID, MySide};
    extern crate hex;
    use super::*;

    #[test]
    fn test_extract_pake_msg() {
        let _key = super::KeyMachine::new(
            &AppID("appid".to_string()),
            &MySide("side1".to_string()),
        );

        let s1 = "7b2270616b655f7631223a22353337363331646366643064336164386130346234663531643935336131343563386538626663373830646461393834373934656634666136656536306339663665227d";
        let pake_msg = super::extract_pake_msg(&hex::decode(s1).unwrap());
        assert_eq!(pake_msg, Some("537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e".to_string()));
    }

    #[test]
    fn test_derive_key() {
        let main = hex::decode(
            "588ba9eef353778b074413a0140205d90d7479e36e0dd4ee35bb729d26131ef1",
        ).unwrap();
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
        ).unwrap());
        let dk11 = derive_phase_key("side1", &main, "phase1");
        assert_eq!(
            hex::encode(dk11),
            "3af6a61d1a111225cc8968c6ca6265efe892065c3ab46de79dda21306b062990"
        );
        let dk12 = derive_phase_key("side1", &main, "phase2");
        assert_eq!(
            hex::encode(dk12),
            "88a1dd12182d989ff498022a9656d1e2806f17328d8bf5d8d0c9753e4381a752"
        );
        let dk21 = derive_phase_key("side2", &main, "phase1");
        assert_eq!(
            hex::encode(dk21),
            "a306627b436ec23bdae3af8fa90c9ac927780d86be1831003e7f617c518ea689"
        );
        let dk22 = derive_phase_key("side2", &main, "phase2");
        assert_eq!(
            hex::encode(dk22),
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
            &AppID("appid1".to_string()),
            &MySide("side".to_string()),
        );

        let key = Key("key".as_bytes().to_vec());
        let side = "side";
        let phase = "phase1";
        let phase1_key = derive_phase_key(side, &key, phase);

        assert_eq!(
            hex::encode(phase1_key),
            "fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96"
        );
    }

    #[test]
    fn test_encrypt_data() {
        let k = hex::decode(
            "ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679",
        ).unwrap();
        let plaintext =
            hex::decode("edc089a518219ec1cee184e89d2d37af").unwrap();
        assert_eq!(plaintext.len(), 16);
        let nonce = hex::decode(
            "2d5e43eb465aa42e750f991e425bee485f06abad7e04af80",
        ).unwrap();
        assert_eq!(nonce.len(), 24);
        let msg = encrypt_data_with_nonce(&k, &plaintext, &nonce);
        assert_eq!(hex::encode(msg), "2d5e43eb465aa42e750f991e425bee485f06abad7e04af80fe318e39d0e4ce932d2b54b300c56d2cda55ee5f0488d63eb1d5f76f7919a49a");
    }

    #[test]
    fn test_decrypt_data() {
        let k = hex::decode(
            "ddc543ef8e4629a603d39dd0307a51bb1e7adb9cb259f6b085c91d0842a18679",
        ).unwrap();
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
        let phase = "phase";
        let data_key = derive_phase_key(side, &key, phase);
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
}
