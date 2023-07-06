use crate::core::*;
use hkdf::Hkdf;
use serde_derive::{Deserialize, Serialize};
use sha2::{digest::FixedOutput, Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::{
    aead::{generic_array::GenericArray, Aead, AeadCore, NewAead},
    XSalsa20Poly1305,
};

/// Marker trait to give encryption keys a "purpose", to not confuse them
///
/// See [`Key`].
// TODO Once const generics are stabilized, try out if a const string generic may replace this.
pub trait KeyPurpose: std::fmt::Debug {}

/// The type of main key of the Wormhole
#[derive(Debug)]
pub struct WormholeKey;
impl KeyPurpose for WormholeKey {}

/// A generic key purpose for ad-hoc subkeys or if you don't care.
#[derive(Debug)]
pub struct GenericKey;
impl KeyPurpose for GenericKey {}

/**
 * The symmetric encryption key used to communicate with the other side.
 *
 * You don't need to do any crypto, but you might need it to derive subkeys for sub-protocols.
 */
#[derive(Debug, Clone, derive_more::Display, derive_more::Deref)]
#[display(fmt = "{:?}", _0)]
#[deref(forward)]
pub struct Key<P: KeyPurpose>(
    #[deref] pub Box<secretbox::Key>,
    #[deref(ignore)] std::marker::PhantomData<P>,
);

impl Key<WormholeKey> {
    /**
     * Derive the sub-key used for transit
     *
     * This one's a bit special, since the Wormhole's AppID is included in the purpose. Different kinds of applications
     * can't talk to each other, not even accidentally, by design.
     *
     * The new key is derived with the `"{appid}/transit-key"` purpose.
     */
    #[cfg(feature = "transit")]
    pub fn derive_transit_key(&self, appid: &AppID) -> Key<crate::transit::TransitKey> {
        let transit_purpose = format!("{}/transit-key", appid);

        let derived_key = self.derive_subkey_from_purpose(&transit_purpose);
        trace!(
            "Input key: {}, Transit key: {}, Transit purpose: '{}'",
            self.to_hex(),
            derived_key.to_hex(),
            &transit_purpose
        );
        derived_key
    }
}

impl<P: KeyPurpose> Key<P> {
    pub fn new(key: Box<secretbox::Key>) -> Self {
        Self(key, std::marker::PhantomData)
    }

    pub fn to_hex(&self) -> String {
        hex::encode(**self)
    }

    /**
     * Derive a new sub-key from this one
     */
    pub fn derive_subkey_from_purpose<NewP: KeyPurpose>(&self, purpose: &str) -> Key<NewP> {
        Key(
            Box::new(derive_key(self, purpose.as_bytes())),
            std::marker::PhantomData,
        )
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct PhaseMessage {
    #[serde(with = "hex::serde")]
    pake_v1: Vec<u8>,
}

/// TODO doc
///
/// The "password" usually is the code, but it needs not to. The only requirement
/// is that both sides use the same value, and agree on that.
pub fn make_pake(password: &str, appid: &AppID) -> (Spake2<Ed25519Group>, Vec<u8>) {
    let (pake_state, msg1) = Spake2::<Ed25519Group>::start_symmetric(
        &Password::new(password.as_bytes()),
        &Identity::new(appid.0.as_bytes()),
    );
    let pake_msg = PhaseMessage { pake_v1: msg1 };
    let pake_msg_ser = serde_json::to_vec(&pake_msg).unwrap();
    (pake_state, pake_msg_ser)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct VersionsMessage {
    #[serde(default)]
    pub abilities: Vec<String>,
    //#[serde(default)]
    pub can_dilate: Option<[Cow<'static, str>; 1]>,
    //#[serde(default)]
    pub dilation_abilities: Option<Cow<'static, [Ability; 2]>>,
    //#[serde(default)]
    #[serde(rename = "app_versions")]
    pub app_versions: serde_json::Value,
    // resume: Option<WormholeResume>,
}

impl VersionsMessage {
    pub fn new() -> Self {
        // Default::default()
        Self {
            abilities: vec![],
            can_dilate: None,
            dilation_abilities: Some(std::borrow::Cow::Borrowed(&[
                Ability::DirectTcpV1,
                Ability::RelayV1,
            ])),
            app_versions: serde_json::Value::Null,
        }
    }

    pub fn set_app_versions(&mut self, versions: serde_json::Value) {
        self.app_versions = versions;
    }

    pub fn enable_dilation(&mut self) {
        self.can_dilate = Some([std::borrow::Cow::Borrowed("1")])
    }

    // pub fn add_resume_ability(&mut self, _resume: ()) {
    //     self.abilities.push("resume-v1".into())
    // }
}

pub fn build_version_msg(
    side: &MySide,
    key: &xsalsa20poly1305::Key,
    versions: &VersionsMessage,
) -> (Phase, Vec<u8>) {
    let phase = Phase::VERSION;
    let data_key = derive_phase_key(side, key, &phase);
    let plaintext = serde_json::to_vec(versions).unwrap();
    let (_nonce, encrypted) = encrypt_data(&data_key, &plaintext);
    (phase, encrypted)
}

pub fn extract_pake_msg(body: &[u8]) -> Result<Vec<u8>, WormholeError> {
    serde_json::from_slice(body)
        .map(|res: PhaseMessage| res.pake_v1)
        .map_err(WormholeError::ProtocolJson)
}

fn encrypt_data_with_nonce(
    key: &xsalsa20poly1305::Key,
    plaintext: &[u8],
    nonce: &xsalsa20poly1305::Nonce,
) -> Vec<u8> {
    let cipher = XSalsa20Poly1305::new(GenericArray::from_slice(key));
    let mut ciphertext = cipher.encrypt(nonce, plaintext).unwrap();
    let mut nonce_and_ciphertext = vec![];
    nonce_and_ciphertext.extend_from_slice(nonce);
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

    derive_key(key, &purpose_vec)
}

pub fn derive_verifier(key: &xsalsa20poly1305::Key) -> xsalsa20poly1305::Key {
    derive_key(key, b"wormhole:verifier")
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::core::EitherSide;

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
            pake_msg.ok(),
            Some(
                hex::decode("537631dcfd0d3ad8a04b4f51d953a145c8e8bfc780dda984794ef4fa6ee60c9f6e")
                    .unwrap()
            )
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
        let dk11 = derive_phase_key(&EitherSide::from("side1"), &main, &Phase("phase1".into()));
        assert_eq!(
            hex::encode(&*dk11),
            "3af6a61d1a111225cc8968c6ca6265efe892065c3ab46de79dda21306b062990"
        );
        let dk12 = derive_phase_key(&EitherSide::from("side1"), &main, &Phase("phase2".into()));
        assert_eq!(
            hex::encode(&*dk12),
            "88a1dd12182d989ff498022a9656d1e2806f17328d8bf5d8d0c9753e4381a752"
        );
        let dk21 = derive_phase_key(&EitherSide::from("side2"), &main, &Phase("phase1".into()));
        assert_eq!(
            hex::encode(&*dk21),
            "a306627b436ec23bdae3af8fa90c9ac927780d86be1831003e7f617c518ea689"
        );
        let dk22 = derive_phase_key(&EitherSide::from("side2"), &main, &Phase("phase2".into()));
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

    #[test]
    fn test_versions_message_can_dilate() {
        let mut message = VersionsMessage::new();

        assert_eq!(message.can_dilate, None);

        message.enable_dilation();

        assert_eq!(message.can_dilate, Some([std::borrow::Cow::Borrowed("1")]));
    }
}
