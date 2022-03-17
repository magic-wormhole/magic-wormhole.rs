use color_eyre::eyre;
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{Duration, SystemTime},
};

fn current_version_number() -> u32 {
    0
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Database {
    // A special seed to recognize when we are sending something to ourselves
    #[serde(with = "hex::serde")]
    pub myself: [u8; xsalsa20poly1305::KEY_SIZE],
    // We assign each peer a unique opaque string that we use as identifier,
    // next to the name.
    #[serde(default)]
    pub peers: HashMap<String, Peer>,
    // Tell our peers who we are
    #[serde(default)]
    pub our_names: Vec<String>,

    // Backwards compatibility
    #[serde(default = "current_version_number")]
    version: u32,
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

impl Database {
    pub fn load(path: &std::path::Path) -> eyre::Result<Self> {
        Ok(serde_json::from_reader(std::fs::File::open(path)?)?)
    }

    pub fn save(&self, path: &std::path::Path) -> eyre::Result<()> {
        serde_json::to_writer_pretty(std::fs::File::create(path)?, self)?;
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&Peer> {
        self.peers
            .iter()
            .find(|(key, value)| {
                key.as_str() == name || value.contact_name.as_deref() == Some(name)
            })
            .map(|(_key, value)| value)
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut Peer> {
        self.peers
            .iter_mut()
            .find(|(key, value)| {
                key.as_str() == name || value.contact_name.as_deref() == Some(name)
            })
            .map(|(_key, value)| value)
    }

    pub fn insert_peer(&mut self, peer: magic_wormhole::WormholeSeed) -> String {
        // Three bytes = Six hex chars length, 2^24 possible values
        // If this repeatedly fails to generate unique strings, slowly increase entropy
        let id = (0..)
            .map(|i| match i {
                // https://github.com/rust-lang/rust/issues/37854
                0..=99 => hex::encode(rand::random::<[u8; 3]>()),
                100..=999 => hex::encode(rand::random::<[u8; 4]>()),
                _ => hex::encode(rand::random::<[u8; 32]>()),
            })
            .find(|id| !self.peers.contains_key(id))
            .unwrap();
        self.peers.insert(
            id.clone(),
            Peer {
                contact_name: None,
                names: peer.display_names,
                seed: peer.seed.into(),
                last_seen: std::time::SystemTime::now(),
                other: Default::default(),
            },
        );
        id
    }

    pub fn iter_known_peers(&self) -> impl Iterator<Item = xsalsa20poly1305::Key> + '_ {
        self.peers
            .values()
            .map(|peer| peer.seed.into())
            .chain(std::iter::once(self.myself.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// Then name under which we stored the seed.
    pub contact_name: Option<String>,
    /// The peer's chosen display names, in decreasing order of preference
    pub names: Vec<String>,
    #[serde(with = "hex::serde")]
    pub seed: [u8; xsalsa20poly1305::KEY_SIZE],
    pub last_seen: SystemTime,

    // Backwards compatibility
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

impl Peer {
    pub fn seen(&mut self) {
        self.last_seen = SystemTime::now();
    }

    pub fn expires(&self) -> SystemTime {
        if self.contact_name.is_some() {
            /* One year */
            self.last_seen + Duration::from_secs(3600 * 24 * 365)
        } else {
            /* One day */
            self.last_seen + Duration::from_secs(3600 * 24)
        }
    }
}

#[allow(dead_code)]
fn main() {
    panic!("This ought to be a helper module, no idea why Rust thinks it's a crate");
}
