//! Cryptographic backbone of the Transit protocol
//!
//! This handles the encrypted handshakes during connection setup, then provides
//! a simple "encrypt/decrypt" abstraction that will be used for all messages.

use super::{
    TransitError, TransitKey, TransitRxKey, TransitTransport, TransitTransportRx,
    TransitTransportTx, TransitTxKey,
};
use crate::Key;
use async_trait::async_trait;
use futures::{future::BoxFuture, io::AsyncWriteExt};
use std::sync::Arc;
use xsalsa20poly1305 as secretbox;
use xsalsa20poly1305::aead::{Aead, NewAead};

/// Private, because we try multiple handshakes and only
/// one needs to succeed
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(super) enum TransitHandshakeError {
    #[error("Handshake failed")]
    HandshakeFailed,
    #[error("Relay handshake failed")]
    RelayHandshakeFailed,
    #[error("Malformed peer address")]
    BadAddress(
        #[from]
        #[source]
        std::net::AddrParseError,
    ),
    #[error("Noise cryptography error")]
    NoiseCrypto(
        #[from]
        #[source]
        noise_protocol::Error,
    ),
    #[error("Decryption error")]
    Decryption,
    #[error("IO error")]
    IO(
        #[from]
        #[source]
        std::io::Error,
    ),
    #[cfg(target_family = "wasm")]
    #[error("WASM error")]
    WASM(
        #[from]
        #[source]
        ws_stream_wasm::WsErr,
    ),
}

impl From<()> for TransitHandshakeError {
    fn from(_: ()) -> Self {
        Self::Decryption
    }
}

/// The Transit protocol has the property that the last message of the handshake is from the leader
/// and confirms the usage of that specific connection. This trait represents that specific type state.
pub(super) trait TransitCryptoInitFinalizer: Send {
    fn handshake_finalize(
        self: Box<Self>,
        socket: &mut dyn TransitTransport,
    ) -> BoxFuture<Result<DynTransitCrypto, TransitHandshakeError>>;
}

/// Due to poorly chosen abstractions elsewhere, the [`TransitCryptoInitFinalizer`] trait is also
/// used by the follower side. Since it is a no-op there, simply implement the trait for the result.
impl TransitCryptoInitFinalizer for DynTransitCrypto {
    fn handshake_finalize(
        self: Box<Self>,
        _socket: &mut dyn TransitTransport,
    ) -> BoxFuture<Result<DynTransitCrypto, TransitHandshakeError>> {
        Box::pin(futures::future::ready(Ok(*self)))
    }
}

/// Do a handshake. Multiple handshakes can be started from one instance on multiple streams.
#[async_trait]
pub(super) trait TransitCryptoInit: Send + Sync {
    // Yes, this method returns a nested future. TODO explain
    async fn handshake_leader(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError>;
    async fn handshake_follower(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError>;
}

/// The classic Transit cryptography backend, powered by libsodium's "Secretbox" API.
///
/// The handshake looks like this (leader perspective):
/// ```text
/// -> transit sender ${transit_key.derive("transit_sender)")} ready\n\n
/// <- transit receiver ${transit_key.derive("transit_receiver")} ready\n\n
/// -> go\n
/// ```
pub struct SecretboxInit {
    pub key: Arc<Key<TransitKey>>,
}

#[async_trait]
impl TransitCryptoInit for SecretboxInit {
    async fn handshake_leader(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError> {
        // 9. create record keys
        let rkey = self
            .key
            .derive_subkey_from_purpose("transit_record_receiver_key");
        let skey = self
            .key
            .derive_subkey_from_purpose("transit_record_sender_key");

        // for transmit mode, send send_handshake_msg and compare.
        // the received message with send_handshake_msg
        socket
            .write_all(
                format!(
                    "transit sender {} ready\n\n",
                    self.key
                        .derive_subkey_from_purpose::<crate::GenericKey>("transit_sender")
                        .to_hex()
                )
                .as_bytes(),
            )
            .await?;

        let expected_rx_handshake = format!(
            "transit receiver {} ready\n\n",
            self.key
                .derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver")
                .to_hex()
        );
        assert_eq!(expected_rx_handshake.len(), 89);
        socket.read_expect(expected_rx_handshake.as_bytes()).await?;

        struct Finalizer {
            skey: Key<TransitTxKey>,
            rkey: Key<TransitRxKey>,
        }

        impl TransitCryptoInitFinalizer for Finalizer {
            fn handshake_finalize(
                self: Box<Self>,
                socket: &mut dyn TransitTransport,
            ) -> BoxFuture<Result<DynTransitCrypto, TransitHandshakeError>> {
                Box::pin(async move {
                    socket.write_all(b"go\n").await?;

                    Ok::<_, TransitHandshakeError>((
                        Box::new(SecretboxCryptoEncrypt {
                            skey: self.skey,
                            snonce: Default::default(),
                        }) as Box<dyn TransitCryptoEncrypt>,
                        Box::new(SecretboxCryptoDecrypt {
                            rkey: self.rkey,
                            rnonce: Default::default(),
                        }) as Box<dyn TransitCryptoDecrypt>,
                    ))
                })
            }
        }

        Ok(Box::new(Finalizer { skey, rkey }))
    }

    async fn handshake_follower(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError> {
        // 9. create record keys
        /* The order here is correct. The "sender" and "receiver" side are a misnomer and should be called
         * "leader" and "follower" instead. As a follower, we use the leader key for receiving and our
         * key for sending.
         */
        let rkey = self
            .key
            .derive_subkey_from_purpose("transit_record_sender_key");
        let skey = self
            .key
            .derive_subkey_from_purpose("transit_record_receiver_key");

        // for receive mode, send receive_handshake_msg and compare.
        // the received message with send_handshake_msg
        socket
            .write_all(
                format!(
                    "transit receiver {} ready\n\n",
                    self.key
                        .derive_subkey_from_purpose::<crate::GenericKey>("transit_receiver")
                        .to_hex(),
                )
                .as_bytes(),
            )
            .await?;

        let expected_tx_handshake = format!(
            "transit sender {} ready\n\ngo\n",
            self.key
                .derive_subkey_from_purpose::<crate::GenericKey>("transit_sender")
                .to_hex(),
        );
        assert_eq!(expected_tx_handshake.len(), 90);
        socket.read_expect(expected_tx_handshake.as_bytes()).await?;

        Ok(Box::new((
            Box::new(SecretboxCryptoEncrypt {
                skey,
                snonce: Default::default(),
            }) as Box<dyn TransitCryptoEncrypt>,
            Box::new(SecretboxCryptoDecrypt {
                rkey,
                rnonce: Default::default(),
            }) as Box<dyn TransitCryptoDecrypt>,
        )) as Box<dyn TransitCryptoInitFinalizer>)
    }
}

type NoiseHandshakeState = noise_protocol::HandshakeState<
    noise_rust_crypto::X25519,
    noise_rust_crypto::ChaCha20Poly1305,
    noise_rust_crypto::Blake2s,
>;
type NoiseCipherState = noise_protocol::CipherState<noise_rust_crypto::ChaCha20Poly1305>;

/// Cryptography based on the [noise protocol](noiseprotocol.org).
/// → "Magic-Wormhole Dilation Handshake v1 Leader\n\n"
/// ← "Magic-Wormhole Dilation Handshake v1 Follower\n\n"
/// → psk, e // Handshake
/// ← e, ee
/// ← "" // First real message
/// → "" // Not in this method, to confirm the connection
///
/// The noise protocol pattern used is "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s"
pub struct NoiseInit {
    pub key: Arc<Key<TransitKey>>,
}

#[async_trait]
impl TransitCryptoInit for NoiseInit {
    async fn handshake_leader(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError> {
        socket
            .write_all(b"Magic-Wormhole Dilation Handshake v1 Leader\n\n")
            .await?;
        socket
            .read_expect(b"Magic-Wormhole Dilation Handshake v1 Follower\n\n")
            .await?;

        let mut handshake: NoiseHandshakeState = {
            let mut builder = noise_protocol::HandshakeStateBuilder::new();
            builder.set_pattern(noise_protocol::patterns::noise_nn_psk0());
            builder.set_prologue(&[]);
            builder.set_is_initiator(true);
            builder.build_handshake_state()
        };
        handshake.push_psk(&self.key);

        // → psk, e
        socket
            .write_transit_message(&handshake.write_message_vec(&[])?)
            .await?;

        // ← e, ee
        handshake.read_message(&socket.read_transit_message().await?, &mut [])?;

        assert!(handshake.completed());
        let (tx, mut rx) = handshake.get_ciphers();

        // ← ""
        let peer_confirmation_message = rx.decrypt_vec(&socket.read_transit_message().await?)?;
        ensure!(
            peer_confirmation_message.is_empty(),
            TransitHandshakeError::HandshakeFailed
        );

        struct Finalizer {
            tx: NoiseCipherState,
            rx: NoiseCipherState,
        }

        impl TransitCryptoInitFinalizer for Finalizer {
            fn handshake_finalize(
                mut self: Box<Self>,
                socket: &mut dyn TransitTransport,
            ) -> BoxFuture<Result<DynTransitCrypto, TransitHandshakeError>> {
                Box::pin(async move {
                    // → ""
                    socket
                        .write_transit_message(&self.tx.encrypt_vec(&[]))
                        .await?;

                    Ok::<_, TransitHandshakeError>((
                        Box::new(NoiseCryptoEncrypt { tx: self.tx })
                            as Box<dyn TransitCryptoEncrypt>,
                        Box::new(NoiseCryptoDecrypt { rx: self.rx })
                            as Box<dyn TransitCryptoDecrypt>,
                    ))
                })
            }
        }

        Ok(Box::new(Finalizer { tx, rx }))
    }

    async fn handshake_follower(
        &self,
        socket: &mut dyn TransitTransport,
    ) -> Result<Box<dyn TransitCryptoInitFinalizer>, TransitHandshakeError> {
        socket
            .write_all(b"Magic-Wormhole Dilation Handshake v1 Follower\n\n")
            .await?;
        socket
            .read_expect(b"Magic-Wormhole Dilation Handshake v1 Leader\n\n")
            .await?;

        let mut handshake: NoiseHandshakeState = {
            let mut builder = noise_protocol::HandshakeStateBuilder::new();
            builder.set_pattern(noise_protocol::patterns::noise_nn_psk0());
            builder.set_prologue(&[]);
            builder.set_is_initiator(false);
            builder.build_handshake_state()
        };
        handshake.push_psk(&self.key);

        // ← psk, e
        handshake.read_message(&socket.read_transit_message().await?, &mut [])?;

        // → e, ee
        socket
            .write_transit_message(&handshake.write_message_vec(&[])?)
            .await?;

        assert!(handshake.completed());
        // Warning: rx and tx are swapped here (read the `get_ciphers` doc carefully)
        let (mut rx, mut tx) = handshake.get_ciphers();

        // → ""
        socket.write_transit_message(&tx.encrypt_vec(&[])).await?;

        // ← ""
        let peer_confirmation_message = rx.decrypt_vec(&socket.read_transit_message().await?)?;
        ensure!(
            peer_confirmation_message.is_empty(),
            TransitHandshakeError::HandshakeFailed
        );

        Ok(Box::new((
            Box::new(NoiseCryptoEncrypt { tx }) as Box<dyn TransitCryptoEncrypt>,
            Box::new(NoiseCryptoDecrypt { rx }) as Box<dyn TransitCryptoDecrypt>,
        )) as Box<dyn TransitCryptoInitFinalizer>)
    }
}

type DynTransitCrypto = (Box<dyn TransitCryptoEncrypt>, Box<dyn TransitCryptoDecrypt>);

#[async_trait]
pub(super) trait TransitCryptoEncrypt: Send {
    async fn encrypt(
        &mut self,
        socket: &mut dyn TransitTransportTx,
        plaintext: &[u8],
    ) -> Result<(), TransitError>;
}

#[async_trait]
pub(super) trait TransitCryptoDecrypt: Send {
    async fn decrypt(
        &mut self,
        socket: &mut dyn TransitTransportRx,
    ) -> Result<Box<[u8]>, TransitError>;
}

struct SecretboxCryptoEncrypt {
    /** Our key, used for sending */
    pub skey: Key<TransitTxKey>,
    /** Nonce for sending */
    pub snonce: secretbox::Nonce,
}

struct SecretboxCryptoDecrypt {
    /** Their key, used for receiving */
    pub rkey: Key<TransitRxKey>,
    /**
     * Nonce for receiving
     *
     * We'll count as receiver and track if messages come in in order
     */
    pub rnonce: secretbox::Nonce,
}

#[async_trait]
impl TransitCryptoEncrypt for SecretboxCryptoEncrypt {
    async fn encrypt(
        &mut self,
        socket: &mut dyn TransitTransportTx,
        plaintext: &[u8],
    ) -> Result<(), TransitError> {
        let nonce = &mut self.snonce;
        let sodium_key = secretbox::Key::from_slice(&self.skey);

        let ciphertext = {
            let nonce_le = secretbox::Nonce::from_slice(nonce);

            let cipher = secretbox::XSalsa20Poly1305::new(sodium_key);
            cipher
                .encrypt(nonce_le, plaintext)
                /* TODO replace with (TransitError::Crypto) after the next xsalsa20poly1305 update */
                .map_err(|_| TransitError::Crypto)?
        };

        // send the encrypted record
        socket
            .write_all(&((ciphertext.len() + nonce.len()) as u32).to_be_bytes())
            .await?;
        socket.write_all(nonce).await?;
        socket.write_all(&ciphertext).await?;

        crate::util::sodium_increment_be(nonce);

        Ok(())
    }
}

#[async_trait]
impl TransitCryptoDecrypt for SecretboxCryptoDecrypt {
    async fn decrypt(
        &mut self,
        socket: &mut dyn TransitTransportRx,
    ) -> Result<Box<[u8]>, TransitError> {
        let nonce = &mut self.rnonce;

        let enc_packet = socket.read_transit_message().await?;

        use std::io::{Error, ErrorKind};
        ensure!(
            enc_packet.len() >= secretbox::NONCE_SIZE,
            Error::new(
                ErrorKind::InvalidData,
                "Message must be long enough to contain at least the nonce"
            )
        );

        // 3. decrypt the vector 'enc_packet' with the key.
        let plaintext = {
            let (received_nonce, ciphertext) = enc_packet.split_at(secretbox::NONCE_SIZE);
            {
                // Nonce check
                ensure!(
                    nonce.as_slice() == received_nonce,
                    TransitError::Nonce(received_nonce.into(), nonce.as_slice().into()),
                );

                crate::util::sodium_increment_be(nonce);
            }

            let cipher = secretbox::XSalsa20Poly1305::new(secretbox::Key::from_slice(&self.rkey));
            cipher
                .decrypt(secretbox::Nonce::from_slice(received_nonce), ciphertext)
                /* TODO replace with (TransitError::Crypto) after the next xsalsa20poly1305 update */
                .map_err(|_| TransitError::Crypto)?
        };

        Ok(plaintext.into_boxed_slice())
    }
}

struct NoiseCryptoEncrypt {
    tx: NoiseCipherState,
}

struct NoiseCryptoDecrypt {
    rx: NoiseCipherState,
}

#[async_trait]
impl TransitCryptoEncrypt for NoiseCryptoEncrypt {
    async fn encrypt(
        &mut self,
        socket: &mut dyn TransitTransportTx,
        plaintext: &[u8],
    ) -> Result<(), TransitError> {
        socket
            .write_transit_message(&self.tx.encrypt_vec(plaintext))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl TransitCryptoDecrypt for NoiseCryptoDecrypt {
    async fn decrypt(
        &mut self,
        socket: &mut dyn TransitTransportRx,
    ) -> Result<Box<[u8]>, TransitError> {
        let plaintext = self.rx.decrypt_vec(&socket.read_transit_message().await?)?;
        Ok(plaintext.into_boxed_slice())
    }
}
