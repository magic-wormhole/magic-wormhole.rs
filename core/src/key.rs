extern crate hex;
extern crate serde_json;

use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use sodiumoxide;
use sodiumoxide::crypto::secretbox;
use spake2;
use spake2::{Ed25519Group, SPAKE2};

use events::Events;
use util;
// we process these
use events::KeyEvent;
// we emit these
use events::BossEvent::GotKey as B_GotKey;
use events::MailboxEvent::AddMessage as M_AddMessage;
use events::ReceiveEvent::GotKey as R_GotKey;

#[derive(Debug, PartialEq)]
enum State {
    S00,
    S10(String),          // code
    S01(Vec<u8>),         // pake
    S11(String, Vec<u8>), // code, pake
}

#[allow(dead_code)]
enum SKState {
    S0KnowNothing,
    S1KnowCode,
    S2KnowCode,
    S3Scared,
}

pub struct Key {
    appid: String,
    state: State,
    side: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct PhaseMessage {
    pake_v1: String,
}

impl Key {
    pub fn new(appid: &str, side: &str) -> Key {
        Key {
            appid: appid.to_string(),
            state: State::S00,
            side: side.to_string(),
        }
    }

    pub fn process(&mut self, event: KeyEvent) -> Events {
        use self::State::*;

        println!(
            "key: current state = {:?}, got event = {:?}",
            self.state, event
        );
        let (newstate, actions) = match self.state {
            S00 => self.do_s00(event),
            S01(ref body) => self.do_s01(body.to_vec(), event),
            S10(ref code) => self.do_s10(&code, event),
            S11(ref code, ref body) => self.do_s11(&code, body.to_vec(), event),
        };

        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        actions
    }

    fn extract_pake_msg(&self, body: Vec<u8>) -> Option<String> {
        let pake_msg = serde_json::from_slice(&body)
            .and_then(|res: PhaseMessage| Ok(res.pake_v1))
            .ok();

        pake_msg
    }

    fn build_pake(&self, code: &str) -> (Events, SPAKE2<Ed25519Group>) {
        let (s1, msg1) = spake2::SPAKE2::<Ed25519Group>::start_symmetric(
            code.as_bytes(),
            self.appid.as_bytes(),
        );
        let payload = util::bytes_to_hexstr(&msg1);
        let pake_msg = PhaseMessage {
            pake_v1: payload,
        };
        let pake_msg_ser = serde_json::to_vec(&pake_msg).unwrap();

        (
            events![M_AddMessage("pake".to_string(), pake_msg_ser)],
            s1,
        )
    }

    fn compute_key(&self, key: &[u8]) -> Events {
        let phase = "version";
        let data_key = Self::derive_phase_key(&self.side, &key, phase);
        let versions = r#"{"app_versions": {}}"#;
        let plaintext = versions.to_string();
        let (_nonce, encrypted) =
            Self::encrypt_data(data_key, &plaintext.as_bytes());
        events![
            B_GotKey(key.to_vec()),
            M_AddMessage(phase.to_string(), encrypted),
            R_GotKey(key.to_vec())
        ]
    }

    pub fn encrypt_data(key: Vec<u8>, plaintext: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let nonce = secretbox::gen_nonce();
        let sodium_key = secretbox::Key::from_slice(&key).unwrap();
        let ciphertext = secretbox::seal(&plaintext, &nonce, &sodium_key);
        let mut nonce_and_ciphertext = Vec::new();
        nonce_and_ciphertext.extend(nonce.as_ref().to_vec());
        nonce_and_ciphertext.extend(ciphertext);
        (nonce.as_ref().to_vec(), nonce_and_ciphertext)
    }

    // TODO: return an Result with a proper error type
    // secretbox::open() returns Result<Vec<u8>, ()> which is not helpful.
    pub fn decrypt_data(key: Vec<u8>, encrypted: &[u8]) -> Option<Vec<u8>> {
        let (nonce, ciphertext) =
            encrypted.split_at(sodiumoxide::crypto::secretbox::NONCEBYTES);
        assert_eq!(
            nonce.len(),
            sodiumoxide::crypto::secretbox::NONCEBYTES
        );
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
        let salt = vec![0; length];
        let hk = Hkdf::<Sha256>::extract(&salt, key);
        hk.expand(purpose, length)
    }

    pub fn derive_phase_key(side: &str, key: &[u8], phase: &str) -> Vec<u8> {
        let side_bytes = side.as_bytes();
        let phase_bytes = phase.as_bytes();
        let side_digest: Vec<u8> = Self::sha256_digest(side_bytes)
            .iter()
            .cloned()
            .collect();
        let phase_digest: Vec<u8> = Self::sha256_digest(phase_bytes)
            .iter()
            .cloned()
            .collect();
        let mut purpose_vec: Vec<u8> = "wormhole:phase:"
            .as_bytes()
            .iter()
            .cloned()
            .collect();
        purpose_vec.extend(side_digest);
        purpose_vec.extend(phase_digest);

        let length = sodiumoxide::crypto::secretbox::KEYBYTES;
        Self::derive_key(key, &purpose_vec, length)
    }

    fn do_s00(&self, event: KeyEvent) -> (Option<State>, Events) {
        use events::KeyEvent::*;

        match event {
            GotCode(code) => {
                // defer building and sending pake.
                (Some(State::S10(code.clone())), events![])
            }
            GotPake(body) => {
                // early, we haven't got the code yet.
                (Some(State::S01(body)), events![])
            }
            GotMessage => panic!(),
        }
    }

    fn send_pake_compute_key(&self, code: &str, body: Vec<u8>) -> Events {
        let (buildpake_events, sp) = self.build_pake(&code);
        let msg2 = self.extract_pake_msg(body).unwrap();
        let key = sp.finish(&hex::decode(msg2).unwrap()).unwrap();
        let mut key_events = self.compute_key(&key);

        let mut es = buildpake_events;
        es.append(&mut key_events);
        es
    }

    fn do_s01(
        &self,
        body: Vec<u8>,
        event: KeyEvent,
    ) -> (Option<State>, Events) {
        use events::KeyEvent::*;

        match event {
            GotCode(code) => {
                let es = self.send_pake_compute_key(&code, body.clone());
                (Some(State::S11(code, body)), es)
            }
            GotPake(_) => panic!(),
            GotMessage => panic!(),
        }
    }

    fn do_s10(&self, code: &str, event: KeyEvent) -> (Option<State>, Events) {
        use events::KeyEvent::*;

        match event {
            GotCode(_) => panic!(), // we already have the code
            GotPake(body) => {
                let es = self.send_pake_compute_key(&code, body.clone());
                (Some(State::S11(code.to_string(), body)), es)
            }
            GotMessage => panic!(),
        }
    }

    // no state transitions while in S11, we already have got code and pake
    fn do_s11(
        &self,
        _code: &str,
        _body: Vec<u8>,
        event: KeyEvent,
    ) -> (Option<State>, Events) {
        use events::KeyEvent::*;

        match event {
            GotCode(_) => panic!(),
            GotPake(_) => panic!(),
            GotMessage => panic!(),
        }
    }
}

#[cfg(test)]
mod test {

    #[test]
    fn test_extract_pake_msg() {
        extern crate hex;

        let key = super::Key::new("appid", "side1");

        let s1 = "7b2270616b655f7631223a22353337363331646366643064336164386130346234663531643935336131343563386538626663373830646461393834373934656634666136656536306339663665227d";
        let pake_msg = key.extract_pake_msg(hex::decode(s1).unwrap());
        assert_eq!(pake_msg, Some("537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e".to_string()));
    }

    #[test]
    fn test_derive_phase_key() {
        use super::*;

        // feed python's derive_phase_key with these inputs:
        // key = b"key"
        // side = u"side"
        // phase = u"phase1"
        // output of derive_phase_key is:
        // "\xfe\x93\x15r\x96h\xa6'\x8a\x97D\x9d\xc9\x9a_L!\x02\xa6h\xc6\x8538\x15)\x06\xbbuRj\x96"
        // hexlified output: fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96
        let _k = Key::new("appid1", "side");

        let key = "key".as_bytes();
        let side = "side";
        let phase = "phase1";
        let phase1_key = Key::derive_phase_key(side, key, phase);

        assert_eq!(
            hex::encode(phase1_key),
            "fe9315729668a6278a97449dc99a5f4c2102a668c6853338152906bb75526a96"
        );
    }

    #[test]
    fn test_encrypt_data_decrypt_data_roundtrip() {
        use super::*;

        let key = "key".as_bytes();
        let side = "side";
        let phase = "phase";
        let data_key = Key::derive_phase_key(side, key, phase);
        let plaintext = "hello world";

        let (_nonce, encrypted) =
            Key::encrypt_data(data_key.clone(), &plaintext.as_bytes());
        let maybe_plaintext = Key::decrypt_data(data_key, &encrypted);
        match maybe_plaintext {
            Some(plaintext_decrypted) => {
                assert_eq!(
                    plaintext.as_bytes().to_vec(),
                    plaintext_decrypted
                );
            }
            None => panic!(),
        }
    }
}
