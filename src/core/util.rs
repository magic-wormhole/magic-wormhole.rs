use rand::{rngs::OsRng, RngCore};

#[deprecated]
pub fn random_bytes(bytes: &mut [u8]) {
    OsRng.fill_bytes(bytes);
}
