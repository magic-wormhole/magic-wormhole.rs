use super::{Mood, Phase};
use std::{borrow::Cow, time::Duration};

use crate::{self as magic_wormhole, AppConfig, AppID, Code, Wormhole};
#[cfg(feature = "transfer")]
use crate::{transfer, transit};

pub const TEST_APPID: AppID = AppID(std::borrow::Cow::Borrowed(
    "lothar.com/wormhole/rusty-wormhole-test",
));

pub const APP_CONFIG: AppConfig<()> = AppConfig::<()> {
    id: TEST_APPID,
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: (),
};

const TIMEOUT: Duration = Duration::from_secs(60);

fn init_logger() {
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

/** Send a file using the Rust implementation. This does not guarantee compatibility with Python! ;) */
#[cfg(feature = "transfer")]
#[async_std::test]
pub async fn test_file_rust2rust() -> eyre::Result<()> {
    init_logger();

    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .spawn(async {
            let (welcome, connector) =
                Wormhole::connect_without_code(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code).unwrap();
            let wormhole = connector.await?;
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
            let (welcome, wormhole) =
                Wormhole::connect_with_code(transfer::APP_CONFIG.id(TEST_APPID), code, true)
                    .await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }

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
            let (welcome, connector) =
                Wormhole::connect_without_code(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code).unwrap();
            let wormhole = connector.await?;
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
            let (welcome, wormhole) =
                Wormhole::connect_with_code(transfer::APP_CONFIG.id(TEST_APPID), code, true)
                    .await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }

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
            let (welcome, connector) =
                Wormhole::connect_without_code(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code).unwrap();
            let wormhole = connector.await?;
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
            let (welcome, wormhole) =
                Wormhole::connect_with_code(transfer::APP_CONFIG.id(TEST_APPID), code, true)
                    .await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }

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

    let (welcome, connector) =
        Wormhole::connect_without_code(transfer::APP_CONFIG.id(TEST_APPID), 2).await?;

    let code = welcome.code;
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
            let wormhole = connector.await?;
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
            let (_welcome, wormhole) = Wormhole::connect_with_code(
                transfer::APP_CONFIG.id(TEST_APPID),
                sender_code.clone(),
                false,
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
        let (_welcome, wormhole) =
            Wormhole::connect_with_code(transfer::APP_CONFIG.id(TEST_APPID), code.clone(), true)
                .await?;
        log::info!("Got key: {}", &wormhole.key);
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
            let (welcome, connector) = Wormhole::connect_without_code(APP_CONFIG, 2).await?;
            if let Some(welcome) = &welcome.welcome {
                log::info!("Got welcome: {}", welcome);
            }
            log::info!("This wormhole's code is: {}", &welcome.code);
            code_tx.send(welcome.code.nameplate()).unwrap();

            let result = connector.await;
            /* This should have failed, due to the wrong code */
            assert!(result.is_err());
            eyre::Result::<_>::Ok(())
        })?;
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .spawn(async {
            let nameplate = code_rx.await?;
            log::info!("Got nameplate over local: {}", &nameplate);
            let result = Wormhole::connect_with_code(
                APP_CONFIG,
                /* Making a wrong code here by appending bullshit */
                Code::new(&nameplate, "foo-bar"),
                true,
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

    let (welcome, connector1) = Wormhole::connect_without_code(APP_CONFIG, 2).await?;
    log::info!("This test's code is: {}", &welcome.code);

    let connector2 = Wormhole::connect_with_code(APP_CONFIG, welcome.code.clone(), true);

    let connector3 = Wormhole::connect_with_code(APP_CONFIG, welcome.code.clone(), true);

    match futures::try_join!(connector1, connector2, connector3).unwrap_err() {
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
    // the max nameplate number is 999, so this will not impact a real nameplate
    let code = Code("1000-guitarist-revenge".to_owned());
    let connector = Wormhole::connect_with_code(APP_CONFIG, code, true)
        .await
        .unwrap_err();
    match connector {
        magic_wormhole::WormholeError::UnclaimedNameplate(x) => {
            assert_eq!(x, magic_wormhole::core::Nameplate("1000".to_owned()));
        },
        other => panic!(
            "Got wrong error type {:?}. Expected `NameplateNotFound`",
            other
        ),
    }

    Ok(())
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
