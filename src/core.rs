use std::borrow::Cow;

#[cfg(feature = "dilation")]
use crate::dilation::DilatedWormhole;
use log::*;
#[cfg(test)]
use mockall::automock;
use serde;
use serde_derive::{Deserialize, Serialize};
use xsalsa20poly1305 as secretbox;

use self::rendezvous::*;
pub(self) use self::server_messages::EncryptedMessage;

pub(super) mod key;
pub mod rendezvous;
mod server_messages;
#[cfg(test)]
mod test;
mod wordlist;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WormholeError {
    /// Some deserialization went wrong, we probably got some garbage
    #[error("Corrupt message received from peer")]
    ProtocolJson(
        #[from]
        #[source]
        serde_json::Error,
    ),
    #[error("Error with the rendezvous server connection")]
    ServerError(
        #[from]
        #[source]
        rendezvous::RendezvousError,
    ),
    /// A generic string message for "something went wrong", i.e.
    /// the server sent some bullshit message order
    #[error("Protocol error: {}", _0)]
    Protocol(Box<str>),
    #[error(
        "Key confirmation failed. If you didn't mistype the code, \
        this is a sign of an attacker guessing passwords. Please try \
        again some time later."
    )]
    PakeFailed,
    #[error("Cannot decrypt a received message")]
    Crypto,
    #[error("Nameplate is unclaimed: {}", _0)]
    UnclaimedNameplate(Nameplate),
    #[error("Dilation version mismatch")]
    DilationVersion,
}

impl WormholeError {
    /** Should we tell the server that we are "errory" or "scared"? */
    pub fn is_scared(&self) -> bool {
        matches!(self, Self::PakeFailed)
    }
}

impl From<std::convert::Infallible> for WormholeError {
    fn from(_: std::convert::Infallible) -> Self {
        unreachable!()
    }
}

/**
 * The result of the client-server handshake
 */
#[derive(Clone, Debug, PartialEq, Eq)]
#[deprecated(
    since = "0.7.0",
    note = "part of the response of `Wormhole::connect_without_code(...)` and `Wormhole::connect_with_code(...) please use 'MailboxConnection::create(...)`/`MailboxConnection::connect(..)` and `Wormhole::connect(mailbox_connection)' instead"
)]
pub struct WormholeWelcome {
    /** A welcome message from the server (think of "message of the day"). Should be displayed to the user if present. */
    pub welcome: Option<String>,
    pub code: Code,
}

/**
 * Establishing Wormhole connection
 *
 * You can send and receive arbitrary messages in form of byte slices over it, using [`Wormhole::send`] and [`Wormhole::receive`].
 * Everything else (including encryption) will be handled for you.
 *
 * To create a wormhole, use the mailbox connection created via [`MailboxConnection::create`] or [`MailboxConnection::connect*`] with the [`Wormhole::connect`] method.
 * Typically, the sender side connects without a code (which will create one), and the receiver side has one (the user entered it, who got it from the sender).
 *
 * # Clean shutdown
 *
 * TODO
 */
/* TODO
 * Maybe a better way to handle application level protocols is to create a trait for them and then
 * to paramterize over them.
 */

/// A `MailboxConnection` contains a `RendezvousServer` which is connected to the mailbox
pub struct MailboxConnection<V: serde::Serialize + Send + Sync + 'static> {
    /// A copy of `AppConfig`,
    config: AppConfig<V>,
    /// The `RendezvousServer` with an open mailbox connection
    server: RendezvousServer,
    /// The welcome message received from the mailbox server
    pub welcome: Option<String>,
    /// The mailbox id of the created mailbox
    pub mailbox: Mailbox,
    /// The Code which is required to connect to the mailbox.
    pub code: Code,
}

impl<V: serde::Serialize + Send + Sync + 'static> MailboxConnection<V> {
    /// Create a connection to a mailbox which is configured with a `Code` starting with the nameplate and by a given number of wordlist based random words.
    ///
    /// # Arguments
    ///
    /// * `config`: Application configuration
    /// * `code_length`: number of words used for the password. The words are taken from the default wordlist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> eyre::Result<()> { async_std::task::block_on(async {
    /// use magic_wormhole::{transfer::APP_CONFIG, AppConfig, MailboxConnection};
    /// let config = APP_CONFIG;
    /// let mailbox_connection = MailboxConnection::create(config, 2).await?;
    /// # Ok(()) })}
    /// ```
    pub async fn create(config: AppConfig<V>, code_length: usize) -> Result<Self, WormholeError> {
        Self::create_with_password(
            config,
            &wordlist::default_wordlist(code_length).choose_words(),
        )
        .await
    }

    /// Create a connection to a mailbox which is configured with a `Code` containing the nameplate and the given password.
    ///
    /// # Arguments
    ///
    /// * `config`: Application configuration
    /// * `password`: Free text password which will be appended to the nameplate number to form the `Code`
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> eyre::Result<()> { async_std::task::block_on(async {
    /// use magic_wormhole::{transfer::APP_CONFIG, MailboxConnection};
    /// let config = APP_CONFIG;
    /// let mailbox_connection = MailboxConnection::create_with_password(config, "secret").await?;
    /// # Ok(()) })}
    /// ```
    pub async fn create_with_password(
        config: AppConfig<V>,
        password: &str,
    ) -> Result<Self, WormholeError> {
        let (mut server, welcome) =
            RendezvousServer::connect(&config.id, &config.rendezvous_url).await?;
        let (nameplate, mailbox) = server.allocate_claim_open().await?;
        let code = Code::new(&nameplate, &password);

        Ok(MailboxConnection {
            config,
            server,
            mailbox,
            code,
            welcome,
        })
    }

    /// Create a connection to a mailbox defined by a `Code` which contains the `Nameplate` and the password to authorize the access.
    ///
    /// # Arguments
    ///
    /// * `config`: Application configuration
    /// * `code`: The `Code` required to authorize to connect to an existing mailbox.
    /// * `allocate`: `true`: Allocates a `Nameplate` if it does not exist.
    ///               `false`: The call fails with a `WormholeError::UnclaimedNameplate` when the `Nameplate` does not exist.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # fn main() -> eyre::Result<()> { async_std::task::block_on(async {
    /// use magic_wormhole::{transfer::APP_CONFIG, Code, MailboxConnection, Nameplate};
    /// let config = APP_CONFIG;
    /// let code = Code::new(&Nameplate::new("5"), "password");
    /// let mailbox_connection = MailboxConnection::connect(config, code, false).await?;
    /// # Ok(()) })}
    /// ```
    pub async fn connect(
        config: AppConfig<V>,
        code: Code,
        allocate: bool,
    ) -> Result<Self, WormholeError> {
        let (mut server, welcome) =
            RendezvousServer::connect(&config.id, &config.rendezvous_url).await?;
        let nameplate = code.nameplate();
        if !allocate {
            let nameplates = server.list_nameplates().await?;
            if !nameplates.contains(&nameplate) {
                server.shutdown(Mood::Errory).await?;
                return Err(WormholeError::UnclaimedNameplate(nameplate));
            }
        }
        let mailbox = server.claim_open(nameplate).await?;

        Ok(MailboxConnection {
            config,
            server,
            mailbox,
            code,
            welcome,
        })
    }

    /// Shut down the connection to the mailbox
    ///
    /// # Arguments
    ///
    /// * `mood`: `Mood` should give a hint of the reason of the shutdown
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> eyre::Result<()> { use magic_wormhole::WormholeError;
    /// async_std::task::block_on(async {
    /// use magic_wormhole::{transfer::APP_CONFIG, MailboxConnection, Mood};
    /// let config = APP_CONFIG;
    /// let mailbox_connection = MailboxConnection::create_with_password(config, "secret")
    ///     .await?;
    /// mailbox_connection.shutdown(Mood::Happy).await?;
    /// # Ok(())})}
    /// ```
    pub async fn shutdown(self, mood: Mood) -> Result<(), WormholeError> {
        self.server
            .shutdown(mood)
            .await
            .map_err(WormholeError::ServerError)
    }
}

#[derive(Debug)]
pub struct Wormhole {
    server: RendezvousServer,
    phase: u64,
    key: key::Key<key::WormholeKey>,
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
    pub verifier: Box<secretbox::Key>,
    /**
     * Our "app version" information that we sent. See the [`peer_version`] for more information.
     */
    pub our_version: Box<dyn std::any::Any + Send + Sync>,
    /**
     * Protocol version information from the other side.
     * This is bound by the [`AppID`]'s protocol and thus shall be handled on a higher level
     * (e.g. by the file transfer API).
     */
    pub peer_version: serde_json::Value,
    /**
     * Enable dilatation. This is off by default. Any application that
     * would want to use a dilated wormhole would turn it on, which
     * results in app specific exchange in the "version" message.
     */
    pub enable_dilation: bool,
}

impl Wormhole {
    /**
     * Generate a code and connect to the rendezvous server.
     *
     * # Returns
     *
     * A tuple with a [`WormholeWelcome`] and a [`std::future::Future`] that will
     * do the rest of the client-client handshake and yield the [`Wormhole`] object
     * on success.
     */
    #[deprecated(
        since = "0.7.0",
        note = "please use 'MailboxConnection::create(...) and Wormhole::connect(mailbox_connection)' instead"
    )]
    #[allow(deprecated)]
    pub async fn connect_without_code(
        config: AppConfig<impl serde::Serialize + Send + Sync + 'static>,
        code_length: usize,
        enable_dilation: bool,
    ) -> Result<
        (
            WormholeWelcome,
            impl std::future::Future<Output = Result<Self, WormholeError>>,
        ),
        WormholeError,
    > {
        let mailbox_connection = MailboxConnection::create(config, code_length).await?;
        Ok((
            WormholeWelcome {
                welcome: mailbox_connection.welcome.clone(),
                code: mailbox_connection.code.clone(),
            },
            Self::connect(mailbox_connection),
        ))
    }

    /**
     * Connect to a peer with a code.
     */
    #[deprecated(
        since = "0.7.0",
        note = "please use 'MailboxConnection::connect(...) and Wormhole::connect(mailbox_connection)' instead"
    )]
    #[allow(deprecated)]
    pub async fn connect_with_code(
        config: AppConfig<impl serde::Serialize + Send + Sync + 'static>,
        code: Code,
        expect_claimed_nameplate: bool,
        enable_dilation: bool,
    ) -> Result<(WormholeWelcome, Self), WormholeError> {
        let mailbox_connection =
            MailboxConnection::connect(config, code.clone(), !expect_claimed_nameplate).await?;
        return Ok((
            WormholeWelcome {
                welcome: mailbox_connection.welcome.clone(),
                code: code,
            },
            Self::connect(mailbox_connection).await?,
        ));
    }

    /// Set up a Wormhole which is the client-client part of the connection setup
    ///
    /// The MailboxConnection already contains a rendezvous server with an opened mailbox.
    pub async fn connect(
        mailbox_connection: MailboxConnection<impl serde::Serialize + Send + Sync + 'static>,
    ) -> Result<Self, WormholeError> {
        let MailboxConnection {
            config,
            mut server,
            mailbox: _mailbox,
            code,
            welcome: _welcome,
        } = mailbox_connection;

        /* Send PAKE */
        let (pake_state, pake_msg_ser) = key::make_pake(&code.0, &config.id);
        server.send_peer_message(Phase::PAKE, pake_msg_ser).await?;

        /* Receive PAKE */
        let peer_pake = key::extract_pake_msg(&server.next_peer_message_some().await?.body)?;
        let key = pake_state
            .finish(&peer_pake)
            .map_err(|_| WormholeError::PakeFailed)
            .map(|key| *secretbox::Key::from_slice(&key))?;

        /* Send versions message */
        let mut versions = key::VersionsMessage::new(enable_dilation);
        versions.set_app_versions(serde_json::to_value(&config.app_version).unwrap());
        println!(
            "versions msg: {}",
            serde_json::to_value(&app_versions).unwrap()
        );
        let (version_phase, version_msg) = key::build_version_msg(server.side(), &key, &versions);
        server.send_peer_message(version_phase, version_msg).await?;
        let peer_version = server.next_peer_message_some().await?;

        /* Handle received message */
        let versions: key::VersionsMessage = peer_version
            .decrypt(&key)
            .ok_or(WormholeError::PakeFailed)
            .and_then(|plaintext| {
                serde_json::from_slice(&plaintext).map_err(WormholeError::ProtocolJson)
            })?;

        let peer_version = versions.app_versions;

        if server.needs_nameplate_release() {
            server.release_nameplate().await?;
        }

        log::info!("Found peer on the rendezvous server.");

        /* We are now fully initialized! Up and running! :tada: */
        Ok(Self {
            server,
            appid: config.id,
            phase: 0,
            key: key::Key::new(key.into()),
            verifier: Box::new(key::derive_verifier(&key)),
            our_version: Box::new(config.app_version),
            peer_version,
            enable_dilation,
        })
    }

    /** TODO */
    pub async fn connect_with_seed() {
        todo!()
    }

    /** Send an encrypted message to peer */
    pub async fn send(&mut self, plaintext: Vec<u8>) -> Result<(), WormholeError> {
        let phase_string = Phase::numeric(self.phase);
        self.phase += 1;
        let data_key = key::derive_phase_key(self.server.side(), &self.key, &phase_string);
        let (_nonce, encrypted) = key::encrypt_data(&data_key, &plaintext);
        self.server
            .send_peer_message(phase_string, encrypted)
            .await?;
        Ok(())
    }

    /** Send an encrypted dilation phase message to peer */
    pub async fn send_dilation_message(&mut self, plaintext: Vec<u8>) -> Result<(), WormholeError> {
        let phase_string = Phase::dilation(self.phase);
        self.phase += 1;
        let data_key = key::derive_phase_key(self.server.side(), &self.key, &phase_string);
        let (_nonce, encrypted) = key::encrypt_data(&data_key, &plaintext);
        self.server
            .send_peer_message(phase_string, encrypted)
            .await?;
        Ok(())
    }

    /**
     * Serialize and send an encrypted message to peer
     *
     * This will serialize the message as `json` string, which is most commonly
     * used by upper layer protocols. The serialization may not fail
     *
     * ## Panics
     *
     * If the serialization fails
     */
    pub async fn send_json<T: serde::Serialize + Send + Sync + 'static>(
        &mut self,
        message: &T,
    ) -> Result<(), WormholeError> {
        self.send(serde_json::to_vec(message).unwrap()).await
    }

    // XXX this function's name could be better than this..
    pub async fn send_json_dilation_message<T: serde::Serialize + Send + Sync + 'static>(
        &mut self,
        message: &T,
    ) -> Result<(), WormholeError> {
        self.send_dilation_message(serde_json::to_vec(message).unwrap())
            .await
    }

    /** Receive an encrypted message from peer */
    pub async fn receive(&mut self) -> Result<Vec<u8>, WormholeError> {
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

    /**
     * Receive an encrypted message from peer
     *
     * This will deserialize the message as `json` string, which is most commonly
     * used by upper layer protocols. We distinguish between the different layers
     * on which a serialization error happened, hence the double `Result`.
     */
    pub async fn receive_json<T>(&mut self) -> Result<T, WormholeError>
    where
        T: for<'a> serde::Deserialize<'a>,
    {
        self.receive().await.and_then(|data: Vec<u8>| {
            serde_json::from_slice(&data).map_err(|e| {
                log::error!(
                    "Received invalid data from peer: '{}'",
                    String::from_utf8_lossy(&data)
                );
                WormholeError::ProtocolJson(e)
            })
        })
    }

    pub async fn close(self) -> Result<(), WormholeError> {
        log::debug!("Closing Wormhole…");
        self.server.shutdown(Mood::Happy).await.map_err(Into::into)
    }

    /**
     * The `AppID` this wormhole is bound to.
     * This determines the upper-layer protocol. Only wormholes with the same value can talk to each other.
     */
    pub fn appid(&self) -> &AppID {
        &self.appid
    }

    /**
     * The symmetric encryption key used by this connection.
     * Can be used to derive sub-keys for different purposes.
     */
    pub fn key(&self) -> &key::Key<key::WormholeKey> {
        &self.key
    }
}

/**
 * create a dilated wormhole
 */
#[cfg(feature = "dilation")]
pub fn dilate(wormhole: Wormhole) -> Result<DilatedWormhole, WormholeError> {
    // XXX: create endpoints?
    // get versions from the other side and check if they support dilation.
    let can_they_dilate = &wormhole.peer_version["can-dilate"];
    if !can_they_dilate.is_null() && can_they_dilate[0] != "1" {
        return Err(WormholeError::DilationVersion);
    }

    Ok(DilatedWormhole::new(wormhole, MySide::generate(8)))
}

// the serialized forms of these variants are part of the wire protocol, so
// they must be spelled exactly as shown
#[derive(
    Debug,
    PartialEq,
    Copy,
    Clone,
    serde_derive::Deserialize,
    serde_derive::Serialize,
    derive_more::Display,
)]
pub enum Mood {
    #[serde(rename = "happy")]
    Happy,
    #[serde(rename = "lonely")]
    Lonely,
    #[serde(rename = "errory")]
    Errory,
    #[serde(rename = "scary")]
    Scared,
    #[serde(rename = "unwelcome")]
    Unwelcome,
}

/**
 * Wormhole configuration corresponding to an upper layer protocol
 *
 * There are multiple different protocols built on top of the core
 * Wormhole protocol. They are identified by a unique URI-like ID string
 * (`AppID`), an URL to find the rendezvous server (might be shared among
 * multiple protocols), and client implementations also have a "version"
 * data to do protocol negotiation.
 *
 * See [`crate::transfer::APP_CONFIG`], which entails
 */
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct AppConfig<V> {
    pub id: AppID,
    pub rendezvous_url: Cow<'static, str>,
    pub app_version: V,
}

impl<V> AppConfig<V> {
    pub fn id(mut self, id: AppID) -> Self {
        self.id = id;
        self
    }

    pub fn rendezvous_url(mut self, rendezvous_url: Cow<'static, str>) -> Self {
        self.rendezvous_url = rendezvous_url;
        self
    }
}

impl<V: serde::Serialize> AppConfig<V> {
    pub fn app_version(mut self, app_version: V) -> Self {
        self.app_version = app_version;
        self
    }
}

/// Newtype wrapper for application IDs
///
/// The application ID is a string that scopes all commands
/// to that name, effectively separating different protocols
/// on the same rendezvous server.
#[derive(
    PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display, derive_more::Deref,
)]
#[deref(forward)]
pub struct AppID(#[deref] pub Cow<'static, str>);

impl AppID {
    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        AppID(id.into())
    }
}

impl From<String> for AppID {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// MySide is used for the String that we send in all our outbound messages
#[derive(
    PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display, derive_more::Deref,
)]
#[serde(transparent)]
#[display(fmt = "MySide({})", "&*_0")]
pub struct MySide(EitherSide);

impl MySide {
    pub fn generate(n: usize) -> MySide {
        use rand::{rngs::OsRng, RngCore};

        let mut bytes = vec![0; n];
        OsRng.fill_bytes(&mut bytes);

        MySide(EitherSide(hex::encode(bytes)))
    }

    // It's a minor type system feature that converting an arbitrary string into MySide is hard.
    // This prevents it from getting swapped around with TheirSide.
    #[cfg(test)]
    pub fn unchecked_from_string(s: String) -> MySide {
        MySide(EitherSide(s))
    }
}

// TheirSide is used for the string that arrives inside inbound messages
#[derive(
    PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display, derive_more::Deref,
)]
#[serde(transparent)]
#[display(fmt = "TheirSide({})", "&*_0")]
pub struct TheirSide(EitherSide);

impl<S: Into<String>> From<S> for TheirSide {
    fn from(s: S) -> TheirSide {
        TheirSide(EitherSide(s.into()))
    }
}

#[derive(
    PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display, derive_more::Deref,
)]
#[serde(transparent)]
#[deref(forward)]
#[display(fmt = "{}", "&*_0")]
pub struct EitherSide(pub String);

impl<S: Into<String>> From<S> for EitherSide {
    fn from(s: S) -> EitherSide {
        EitherSide(s.into())
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Hash, Deserialize, Serialize, derive_more::Display)]
#[serde(transparent)]
pub struct Phase(pub Cow<'static, str>);

impl Phase {
    pub const VERSION: Self = Phase(Cow::Borrowed("version"));
    pub const PAKE: Self = Phase(Cow::Borrowed("pake"));

    pub fn numeric(phase: u64) -> Self {
        Phase(phase.to_string().into())
    }

    pub fn dilation(phase: u64) -> Self {
        Phase(format!("dilate-{}", phase.to_string()).to_string().into())
    }

    pub fn is_version(&self) -> bool {
        self == &Self::VERSION
    }
    pub fn is_pake(&self) -> bool {
        self == &Self::PAKE
    }
    pub fn to_num(&self) -> Option<u64> {
        self.0.parse().ok()
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display)]
#[serde(transparent)]
pub struct Mailbox(pub String);

#[derive(
    PartialEq, Eq, Clone, Debug, Deserialize, Serialize, derive_more::Display, derive_more::Deref,
)]
#[serde(transparent)]
#[deref(forward)]
#[display(fmt = "{}", _0)]
pub struct Nameplate(pub String);

impl Nameplate {
    pub fn new(n: &str) -> Self {
        Nameplate(String::from(n))
    }
}

impl Into<String> for Nameplate {
    fn into(self) -> String {
        self.0
    }
}

/** A wormhole code à la 15-foo-bar
 *
 * The part until the first dash is called the "nameplate" and is purely numeric.
 * The rest is the password and may be arbitrary, although dash-joining words from
 * a wordlist is a common convention.
 */
#[derive(PartialEq, Eq, Clone, Debug, derive_more::Display, derive_more::Deref)]
#[display(fmt = "{}", _0)]
pub struct Code(pub String);

impl Code {
    pub fn new(nameplate: &Nameplate, password: &str) -> Self {
        Code(format!("{}-{}", nameplate, password))
    }

    pub fn split(&self) -> (Nameplate, String) {
        let mut iter = self.0.splitn(2, '-');
        let nameplate = Nameplate::new(iter.next().unwrap());
        let password = iter.next().unwrap();
        (nameplate, password.to_string())
    }

    pub fn nameplate(&self) -> Nameplate {
        Nameplate::new(self.0.splitn(2, '-').next().unwrap())
    }
}
