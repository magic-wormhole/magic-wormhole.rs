use async_trait::async_trait;
use std::{any::Any, fmt::Debug};

#[cfg(test)]
use mockall::automock;

use crate::{
    core::{
        key::{derive_phase_key, derive_verifier, encrypt_data},
        PhaseProvider,
    },
    rendezvous::RendezvousServer,
    AppConfig, AppID, Key, Mood, WormholeError, WormholeKey,
};

#[derive(Debug)]
pub struct WormholeProtocolDefault {
    server: RendezvousServer,
    phase: u64,
    key: Key<WormholeKey>,
    appid: AppID,
    /**
     * If you're paranoid, let both sides check that they calculated the same verifier.
     *
     * PAKE hardens a standard key exchange with a password ("password authenticated") in order
     * to mitigate potential man in the middle attacks that would otherwise be possible. Since
     * the passwords usually are not of hight entropy, there is a low-probability possible of
     * an attacker guessing the password correctly, enabling them to MitM the connection.
     *
     * Not only is that probability low, but they also have only one try per connection and a failed
     * attempts will be noticed by both sides. Nevertheless, comparing the verifier mitigates that
     * attack vector.
     */
    pub verifier: Box<super::secretbox::Key>,
    /**
     * Our "app version" information that we sent. See the [`peer_version`] for more information.
     */
    pub our_version: Box<dyn Any + Send + Sync>,
    /**
     * Protocol version information from the other side.
     * This is bound by the [`AppID`]'s protocol and thus shall be handled on a higher level
     * (e.g. by the file transfer API).
     */
    pub peer_version: serde_json::Value,
}

impl WormholeProtocolDefault {
    pub fn new<T>(
        server: RendezvousServer,
        config: AppConfig<T>,
        key: Key<WormholeKey>,
        peer_version: serde_json::Value,
    ) -> Self
    where
        T: serde::Serialize + Send + Sync + Sized + 'static,
    {
        let verifier = Box::new(derive_verifier(&key));
        Self {
            server,
            appid: config.id,
            phase: 0,
            key,
            verifier,
            our_version: Box::new(config.app_version),
            peer_version,
        }
    }
}

#[async_trait]
impl WormholeProtocol for WormholeProtocolDefault {
    /** Send an encrypted message to peer */
    async fn send_with_phase(
        &mut self,
        plaintext: Vec<u8>,
        phase_provider: PhaseProvider,
    ) -> Result<(), WormholeError> {
        let current_phase = phase_provider(self.phase);
        self.phase += 1;
        let data_key = derive_phase_key(self.server.side(), &self.key, &current_phase);
        let (_nonce, encrypted) = encrypt_data(&data_key, &plaintext);
        self.server
            .send_peer_message(current_phase, encrypted)
            .await?;
        Ok(())
    }

    /** Receive an encrypted message from peer */
    async fn receive(&mut self) -> Result<Vec<u8>, WormholeError> {
        loop {
            let peer_message = match self.server.next_peer_message().await? {
                Some(peer_message) => peer_message,
                None => continue,
            };

            // TODO maybe reorder incoming messages by phase numeral?
            let decrypted_message = peer_message
                .decrypt(&self.key)
                .ok_or(WormholeError::Crypto)?;

            // Send to client
            return Ok(decrypted_message);
        }
    }

    async fn close(&mut self) -> Result<(), WormholeError> {
        log::debug!("Closing Wormhole…");
        self.server.shutdown(Mood::Happy).await.map_err(Into::into)
    }

    /**
     * The `AppID` this wormhole is bound to.
     * This determines the upper-layer protocol. Only wormholes with the same value can talk to each other.
     */
    fn appid(&self) -> &AppID {
        &self.appid
    }

    /**
     * The symmetric encryption key used by this connection.
     * Can be used to derive sub-keys for different purposes.
     */
    fn key(&self) -> &Key<WormholeKey> {
        &self.key
    }

    fn peer_version(&self) -> &serde_json::Value {
        &self.peer_version
    }

    fn our_version(&self) -> &Box<dyn Any + Send + Sync> {
        &self.our_version
    }
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait WormholeProtocol: Debug + Send + Sync {
    async fn send_with_phase(
        &mut self,
        plaintext: Vec<u8>,
        phase_provider: PhaseProvider,
    ) -> Result<(), WormholeError>;
    async fn receive(&mut self) -> Result<Vec<u8>, WormholeError>;
    async fn close(&mut self) -> Result<(), WormholeError>;
    fn appid(&self) -> &AppID;
    fn key(&self) -> &Key<WormholeKey>;
    fn peer_version(&self) -> &serde_json::Value;
    fn our_version(&self) -> &Box<dyn Any + Send + Sync>;
}
