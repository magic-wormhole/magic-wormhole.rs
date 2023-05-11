use super::{Mood, Phase};
use rand::Rng;
use std::{borrow::Cow, time::Duration};

use crate::{
    self as magic_wormhole,
    core::{MailboxConnection, Nameplate},
    AppConfig, AppID, Code, Wormhole, WormholeError,
};
#[cfg(feature = "transfer")]
use crate::{transfer, transit};

pub const TEST_APPID: AppID = AppID(std::borrow::Cow::Borrowed(
    "lothar.com/wormhole/rusty-wormhole-test",
));

pub const APP_CONFIG: AppConfig<()> = AppConfig::<()> {
    id: TEST_APPID,
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: (),
    with_dilation: false,
};

const TIMEOUT: Duration = Duration::from_secs(60);

pub fn init_logger() {
    /* Ignore errors from succeedent initialization tries */
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("magic_wormhole", log::LevelFilter::Trace)
        .filter_module("magic_wormhole::transfer", log::LevelFilter::Trace)
        .filter_module("magic_wormhole::transit", log::LevelFilter::Trace)
        .filter_module("mio", log::LevelFilter::Debug)
        .try_init();
}

fn default_relay_hints() -> Vec<transit::RelayHint> {
    vec![
        transit::RelayHint::from_urls(None, [transit::DEFAULT_RELAY_SERVER.parse().unwrap()])
            .unwrap(),
    ]
}

#[async_std::test]
pub async fn test_connect_with_unknown_code_and_allocate_passes() -> eyre::Result<(), WormholeError>
{
    init_logger();

    let code = generate_random_code();

    let mailbox_connection =
        MailboxConnection::connect(transfer::APP_CONFIG.id(TEST_APPID).clone(), code, true).await;

    assert!(mailbox_connection.is_ok());

    mailbox_connection.unwrap().shutdown(Mood::Happy).await
}

#[async_std::test]
pub async fn test_connect_with_unknown_code_and_no_allocate_fails() {
    init_logger();

    let code = generate_random_code();

    let mailbox_connection = MailboxConnection::connect(
        transfer::APP_CONFIG.id(TEST_APPID).clone(),
        code.clone(),
        false,
    )
    .await;

    assert!(mailbox_connection.is_err());
    let error = mailbox_connection.err().unwrap();
    match error {
        WormholeError::UnclaimedNameplate(nameplate) => {
            assert_eq!(nameplate, code.nameplate());
        },
        _ => {
            assert!(false);
        },
    }
}

/** Send a file using the Rust implementation. This does not guarantee compatibility with Python! ;) */
#[cfg(feature = "transfer")]
#[async_std::test]
#[allow(deprecated)]
pub async fn test_file_rust2rust_deprecated() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let (welcome, wormhole_future) =
                Wormhole::connect_without_code(transfer::APP_CONFIG.id(TEST_APPID).clone(), 2)
                    .await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code.clone()).unwrap();
            let wormhole = wormhole_future.await?;
            eyre::Result::<_>::Ok(
                transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open("tests/example-file.bin").await?,
                    "example-file.bin",
                    std::fs::metadata("tests/example-file.bin").unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_sent, _total| {},
                    futures::future::pending(),
                )
                .await?,
            )
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let code = code_rx.await?;
            log::info!("Got code over local: {}", &code);
            let config = transfer::APP_CONFIG.id(TEST_APPID);
            let (welcome, wormhole) =
                Wormhole::connect_with_code(config, code.clone(), true).await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);

            let req = transfer::request_file(
                wormhole,
                default_relay_hints(),
                magic_wormhole::transit::Abilities::ALL_ABILITIES,
                futures::future::pending(),
            )
            .await?
            .unwrap();

            let mut buffer = Vec::<u8>::new();
            req.accept(
                &transit::log_transit_connection,
                |_received, _total| {},
                &mut buffer,
                futures::future::pending(),
            )
            .await?;
            Ok(buffer)
        })?;

    sender_task.await?;
    let original = std::fs::read("tests/example-file.bin")?;
    let received: Vec<u8> = (receiver_task.await as eyre::Result<Vec<u8>>)?;

    assert_eq!(original, received, "Files differ");
    Ok(())
}

/** Send a file using the Rust implementation. This does not guarantee compatibility with Python! ;) */
#[cfg(feature = "transfer")]
#[async_std::test]
pub async fn test_file_rust2rust() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let mailbox_connection =
                MailboxConnection::create(transfer::APP_CONFIG.id(TEST_APPID).clone(), 2).await?;
            if let Some(welcome) = &mailbox_connection.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &mailbox_connection.code);
            code_tx.send(mailbox_connection.code.clone()).unwrap();
            let wormhole = Wormhole::connect(mailbox_connection).await?;
            eyre::Result::<_>::Ok(
                transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open("tests/example-file.bin").await?,
                    "example-file.bin",
                    std::fs::metadata("tests/example-file.bin").unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_sent, _total| {},
                    futures::future::pending(),
                )
                .await?,
            )
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let code = code_rx.await?;
            log::info!("Got code over local: {}", &code);
            let config = transfer::APP_CONFIG.id(TEST_APPID);
            let mailbox = MailboxConnection::connect(config, code.clone(), false).await?;
            if let Some(welcome) = mailbox.welcome.clone() {
                log::info!("Got welcome: {}", welcome);
            }
            let wormhole = Wormhole::connect(mailbox).await?;

            let req = transfer::request_file(
                wormhole,
                default_relay_hints(),
                magic_wormhole::transit::Abilities::ALL_ABILITIES,
                futures::future::pending(),
            )
            .await?
            .unwrap();

            let mut buffer = Vec::<u8>::new();
            req.accept(
                &transit::log_transit_connection,
                |_received, _total| {},
                &mut buffer,
                futures::future::pending(),
            )
            .await?;
            Ok(buffer)
        })?;

    sender_task.await?;
    let original = std::fs::read("tests/example-file.bin")?;
    let received: Vec<u8> = (receiver_task.await as eyre::Result<Vec<u8>>)?;

    assert_eq!(original, received, "Files differ");
    Ok(())
}

/** Send a file using the Rust implementation that has exactly 4096 bytes (our chunk size) */
#[cfg(feature = "transfer")]
#[async_std::test]
pub async fn test_4096_file_rust2rust() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    const FILENAME: &str = "tests/example-file-4096.bin";

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let config = transfer::APP_CONFIG.id(TEST_APPID);
            let mailbox = MailboxConnection::create(config, 2).await?;
            if let Some(welcome) = &mailbox.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &mailbox.code);
            code_tx.send(mailbox.code.clone()).unwrap();
            let wormhole = Wormhole::connect(mailbox).await?;
            eyre::Result::<_>::Ok(
                transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open(FILENAME).await?,
                    "example-file.bin",
                    std::fs::metadata(FILENAME).unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_sent, _total| {},
                    futures::future::pending(),
                )
                .await?,
            )
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let code = code_rx.await?;
            log::info!("Got code over local: {}", &code);
            let config = transfer::APP_CONFIG.id(TEST_APPID);
            let mailbox = MailboxConnection::connect(config, code, false).await?;
            if let Some(welcome) = &mailbox.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            let wormhole = Wormhole::connect(mailbox).await?;

            let req = transfer::request_file(
                wormhole,
                default_relay_hints(),
                magic_wormhole::transit::Abilities::ALL_ABILITIES,
                futures::future::pending(),
            )
            .await?
            .unwrap();

            let mut buffer = Vec::<u8>::new();
            req.accept(
                &transit::log_transit_connection,
                |_received, _total| {},
                &mut buffer,
                futures::future::pending(),
            )
            .await?;
            Ok(buffer)
        })?;

    sender_task.await?;
    let original = std::fs::read(FILENAME)?;
    let received: Vec<u8> = (receiver_task.await as eyre::Result<Vec<u8>>)?;

    assert_eq!(original, received, "Files differ");
    Ok(())
}

/** https://github.com/magic-wormhole/magic-wormhole.rs/issues/160 */
#[cfg(feature = "transfer")]
#[async_std::test]
pub async fn test_empty_file_rust2rust() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let mailbox = MailboxConnection::create(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;
            if let Some(welcome) = &mailbox.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &mailbox.code);
            code_tx.send(mailbox.code.clone()).unwrap();
            let wormhole = Wormhole::connect(mailbox).await?;

            eyre::Result::<_>::Ok(
                transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open("tests/example-file-empty").await?,
                    "example-file-empty",
                    std::fs::metadata("tests/example-file-empty").unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_sent, _total| {},
                    futures::future::pending(),
                )
                .await?,
            )
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let code = code_rx.await?;
            log::info!("Got code over local: {}", &code);
            let mailbox =
                MailboxConnection::connect(transfer::APP_CONFIG.id(TEST_APPID), code, false)
                    .await?;
            if let Some(welcome) = &mailbox.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            let wormhole = Wormhole::connect(mailbox).await?;

            let req = transfer::request_file(
                wormhole,
                default_relay_hints(),
                magic_wormhole::transit::Abilities::ALL_ABILITIES,
                futures::future::pending(),
            )
            .await?
            .unwrap();

            let mut buffer = Vec::<u8>::new();
            req.accept(
                &transit::log_transit_connection,
                |_received, _total| {},
                &mut buffer,
                futures::future::pending(),
            )
            .await?;
            eyre::Result::<Vec<u8>>::Ok(buffer)
        })?;

    sender_task.await?;

    assert!(&receiver_task.await?.is_empty());
    Ok(())
}

/** Test the functionality used by the `send-many` subcommand. It logically builds upon the
 * `test_eventloop_exit` tests. We send us a file five times, and check if it arrived.
 */
#[cfg(feature = "transfer")]
#[async_std::test]
pub async fn test_send_many() -> eyre::Result<()> {
    init_logger();

    let mailbox = MailboxConnection::create(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;
    let code = mailbox.code.clone();
    log::info!("The code is {:?}", code);

    let correct_data = std::fs::read("tests/example-file.bin")?;

    /* Send many */
    let sender_code = code.clone();
    let senders = async_std::task::spawn(async move {
        // let mut senders = Vec::<async_std::task::JoinHandle<std::result::Result<std::vec::Vec<u8>, eyre::Error>>>::new();
        let mut senders = Vec::new();

        /* The first time, we reuse the current session for sending */
        {
            log::info!("Sending file #{}", 0);
            let wormhole = Wormhole::connect(mailbox).await?;
            senders.push(async_std::task::spawn(async move {
                default_relay_hints();
                crate::transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open("tests/example-file.bin").await?,
                    "example-file.bin",
                    std::fs::metadata("tests/example-file.bin").unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_, _| {},
                    futures::future::pending(),
                )
                .await
            }));
        }

        for i in 1..5usize {
            log::info!("Sending file #{}", i);
            let wormhole = Wormhole::connect(
                MailboxConnection::connect(
                    transfer::APP_CONFIG.id(TEST_APPID),
                    sender_code.clone(),
                    true,
                )
                .await?,
            )
            .await?;
            senders.push(async_std::task::spawn(async move {
                default_relay_hints();
                crate::transfer::send_file(
                    wormhole,
                    default_relay_hints(),
                    &mut async_std::fs::File::open("tests/example-file.bin").await?,
                    "example-file.bin",
                    std::fs::metadata("tests/example-file.bin").unwrap().len(),
                    magic_wormhole::transit::Abilities::ALL_ABILITIES,
                    &transit::log_transit_connection,
                    |_, _| {},
                    futures::future::pending(),
                )
                .await
            }));
        }
        eyre::Result::<_>::Ok(senders)
    });

    async_std::task::sleep(std::time::Duration::from_secs(1)).await;

    /* Receive many */
    for i in 0..5usize {
        log::info!("Receiving file #{}", i);
        let wormhole = Wormhole::connect(
            MailboxConnection::connect(transfer::APP_CONFIG.id(TEST_APPID), code.clone(), true)
                .await?,
        )
        .await?;
        log::info!("Got key: {}", &wormhole.key());
        let req = crate::transfer::request_file(
            wormhole,
            default_relay_hints(),
            magic_wormhole::transit::Abilities::ALL_ABILITIES,
            futures::future::pending(),
        )
        .await?
        .unwrap();

        let mut buffer = Vec::<u8>::new();
        req.accept(
            &transit::log_transit_connection,
            |_, _| {},
            &mut buffer,
            futures::future::pending(),
        )
        .await?;
        assert_eq!(correct_data, buffer, "Files #{} differ", i);
    }

    for sender in senders.await? {
        sender.await?;
    }

    Ok(())
}

/// Try to send a file, but use a bad code, and see how it's handled
#[async_std::test]
pub async fn test_wrong_code() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let mailbox = MailboxConnection::create(APP_CONFIG, 2).await?;
            if let Some(welcome) = &mailbox.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            let code = mailbox.code.clone();
            log::info!("This wormhole's code is: {}", &code);
            code_tx.send(code.nameplate()).unwrap();

            let result = Wormhole::connect(mailbox).await;
            /* This should have failed, due to the wrong code */
            assert!(result.is_err());
            eyre::Result::<_>::Ok(())
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let nameplate = code_rx.await?;
            log::info!("Got nameplate over local: {}", &nameplate);
            let result = Wormhole::connect(
                MailboxConnection::connect(
                    APP_CONFIG,
                    /* Making a wrong code here by appending bullshit */
                    Code::new(&nameplate, "foo-bar"),
                    true,
                )
                .await?,
            )
            .await;

            /* This should have failed, due to the wrong code */
            assert!(result.is_err());
            eyre::Result::<_>::Ok(())
        })?;

    async_std::future::timeout(TIMEOUT, sender_task).await??;
    async_std::future::timeout(TIMEOUT, receiver_task).await??;

    Ok(())
}

/** Connect three people to the party and watch it explode … gracefully */
#[async_std::test]
pub async fn test_crowded() -> eyre::Result<()> {
    init_logger();

    let initial_mailbox_connection = MailboxConnection::create(APP_CONFIG, 2).await?;
    log::info!("This test's code is: {}", &initial_mailbox_connection.code);
    let code = initial_mailbox_connection.code.clone();

    let mailbox_connection_1 = MailboxConnection::connect(APP_CONFIG.clone(), code.clone(), false);
    let mailbox_connection_2 = MailboxConnection::connect(APP_CONFIG.clone(), code.clone(), false);

    match futures::try_join!(mailbox_connection_1, mailbox_connection_2)
        .err()
        .unwrap()
    {
        magic_wormhole::WormholeError::ServerError(
            magic_wormhole::rendezvous::RendezvousError::Server(error),
        ) => {
            assert_eq!(&*error, "crowded")
        },
        other => panic!("Got wrong error message: {}, wanted 'crowded'", other),
    }

    Ok(())
}

#[async_std::test]
pub async fn test_connect_with_code_expecting_nameplate() -> eyre::Result<()> {
    let code = generate_random_code();
    let result = MailboxConnection::connect(APP_CONFIG, code.clone(), false).await;
    let error = result.err().unwrap();
    match error {
        magic_wormhole::WormholeError::UnclaimedNameplate(x) => {
            assert_eq!(x, code.nameplate());
        },
        other => panic!(
            "Got wrong error type {:?}. Expected `NameplateNotFound`",
            other
        ),
    }

    Ok(())
}

fn generate_random_code() -> Code {
    let mut rng = rand::thread_rng();
    let nameplate_string = format!("{}-guitarist-revenge", rng.gen_range(1000..10000));
    let nameplate = Nameplate::new(&nameplate_string);
    Code::new(&nameplate, "guitarist-revenge")
}

#[test]
fn test_phase() {
    let p = Phase::PAKE;
    assert!(p.is_pake());
    assert!(!p.is_version());
}

#[test]
fn test_mood() {
    // The serialized forms of these variants are part of the wire protocol,
    // so they must be spelled exactly as shown (they must match the strings
    // used in the Python version in src/wormhole/_boss.py , in calls to
    // self._T.close())
    assert_eq!(
        String::from(r#""happy""#),
        serde_json::to_string(&Mood::Happy).unwrap()
    );
    assert_eq!(
        String::from(r#""lonely""#),
        serde_json::to_string(&Mood::Lonely).unwrap()
    );
    assert_eq!(
        String::from(r#""errory""#),
        serde_json::to_string(&Mood::Errory).unwrap()
    );
    assert_eq!(
        String::from(r#""scary""#),
        serde_json::to_string(&Mood::Scared).unwrap()
    );
    assert_eq!(
        String::from(r#""unwelcome""#),
        serde_json::to_string(&Mood::Unwelcome).unwrap()
    );
}
