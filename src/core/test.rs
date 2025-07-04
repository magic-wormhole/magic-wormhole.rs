#![allow(irrefutable_let_patterns)]

use super::{Mood, Phase};
use rand::Rng;
use std::{borrow::Cow, str::FromStr, time::Duration};

#[cfg(feature = "transfer")]
use crate::transfer;
use crate::{
    self as magic_wormhole, core::MailboxConnection, transit, AppConfig, AppID, Code, WormholeError,
};
use test_log::test;

pub const TEST_APPID: AppID = AppID(std::borrow::Cow::Borrowed(
    "magic-wormhole.github.io/magic-wormhole.rs/test",
));

pub const APP_CONFIG: AppConfig<()> = AppConfig::<()> {
    id: TEST_APPID,
    rendezvous_url: Cow::Borrowed(crate::rendezvous::DEFAULT_RENDEZVOUS_SERVER),
    app_version: (),
};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Utility method that logs information of the transit result
///
/// Example usage:
///
/// ```no_run
/// use magic_wormhole as mw;
/// # #[async_std::main] async fn main() -> Result<(), mw::transit::TransitConnectError> {
/// # let derived_key = unimplemented!();
/// # let their_abilities = unimplemented!();
/// # let their_hints = unimplemented!();
/// let connector: mw::transit::TransitConnector = unimplemented!("transit::init(…).await?");
/// let (mut transit, info) = connector
///     .leader_connect(derived_key, their_abilities, their_hints)
///     .await?;
/// mw::log_transit_connection(info);
/// # Ok(())
/// # }
/// ```
pub(crate) fn log_transit_connection(info: crate::transit::TransitInfo) {
    tracing::info!("{info}")
}

fn default_relay_hints() -> Vec<transit::RelayHint> {
    vec![
        transit::RelayHint::from_urls(None, [transit::DEFAULT_RELAY_SERVER.parse().unwrap()])
            .unwrap(),
    ]
}

#[test(async_std::test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
pub async fn test_connect_with_unknown_code_and_allocate_passes() {
    let code = generate_random_code();

    let mailbox_connection =
        MailboxConnection::connect(transfer::APP_CONFIG.id(TEST_APPID).clone(), code, true).await;

    assert!(mailbox_connection.is_ok());

    mailbox_connection
        .unwrap()
        .shutdown(Mood::Happy)
        .await
        .unwrap()
}

#[test(async_std::test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
pub async fn test_connect_with_unknown_code_and_no_allocate_fails() {
    tracing::info!("hola!");
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

/** Generate common offers for testing, together with a pre-made answer that checks the received content */
async fn file_offers(
) -> eyre::Result<Vec<(transfer::offer::OfferSend, transfer::offer::OfferAccept)>> {
    async fn offer(
        name: &str,
    ) -> eyre::Result<(transfer::offer::OfferSend, transfer::offer::OfferAccept)> {
        #[cfg(target_family = "wasm")]
        let (data, offer) = {
            let data = match name {
                "example-file-4096.bin" => {
                    include_bytes!("../../tests/example-file-4096.bin").to_vec()
                },
                "example-file-empty" => include_bytes!("../../tests/example-file-empty").to_vec(),
                "example-file.bin" => include_bytes!("../../tests/example-file.bin").to_vec(),
                _ => panic!("file {name} not included in test binary"),
            };
            let offer = transfer::offer::OfferSend::new_file_custom(
                name.into(),
                data.len() as u64,
                Box::new({
                    let data = data.clone();
                    move || {
                        let data = data.clone();
                        Box::pin(async move {
                            Ok(Box::new(async_std::io::Cursor::new(data))
                                as Box<
                                    dyn crate::transfer::offer::AsyncReadSeek
                                        + std::marker::Send
                                        + Unpin
                                        + 'static,
                                >)
                        })
                    }
                }),
            );

            (data, offer)
        };
        #[cfg(not(target_family = "wasm"))]
        let (data, offer) = {
            let path = format!("tests/{name}");
            let data = async_std::fs::read(&path).await.unwrap();
            let offer = transfer::offer::OfferSend::new_file_or_folder(name.into(), &path)
                .await
                .unwrap();

            (data, offer)
        };

        let answer = offer.set_content(|_path| {
            use std::{
                io,
                pin::Pin,
                task::{Context, Poll},
            };

            let content = transfer::offer::new_accept_content({
                let data = data.clone();
                move |_append| {
                    struct Writer {
                        closed: bool,
                        send_bytes: Vec<u8>,
                        receive_bytes: Vec<u8>,
                    }

                    impl futures::io::AsyncWrite for Writer {
                        fn poll_write(
                            mut self: Pin<&mut Self>,
                            _: &mut Context<'_>,
                            buf: &[u8],
                        ) -> Poll<io::Result<usize>> {
                            self.receive_bytes.extend_from_slice(buf);
                            Poll::Ready(Ok(buf.len()))
                        }

                        fn poll_close(
                            mut self: Pin<&mut Self>,
                            _: &mut Context<'_>,
                        ) -> Poll<io::Result<()>> {
                            self.closed = true;
                            if self.send_bytes == self.receive_bytes {
                                Poll::Ready(Ok(()))
                            } else {
                                Poll::Ready(Err(io::Error::other(
                                    "Send and receive are not the same",
                                )))
                            }
                        }

                        fn poll_flush(
                            self: Pin<&mut Self>,
                            _: &mut Context<'_>,
                        ) -> Poll<io::Result<()>> {
                            Poll::Ready(Ok(()))
                        }
                    }

                    impl Drop for Writer {
                        fn drop(&mut self) {
                            assert!(self.closed, "Implementation forgot to close Writer");
                        }
                    }

                    let send_bytes = data.clone();
                    async move {
                        Ok(Writer {
                            closed: false,
                            send_bytes,
                            receive_bytes: Vec::new(),
                        })
                    }
                }
            });
            transfer::offer::AcceptInner {
                content,
                offset: 0,
                sha256: None,
            }
        });

        Ok((offer, answer))
    }

    Ok(vec![
        offer("example-file.bin").await?,
        /* Empty file: https://github.com/magic-wormhole/magic-wormhole.rs/issues/160 */
        offer("example-file-empty").await?,
        /* 4k file: https://github.com/magic-wormhole/magic-wormhole.rs/issues/152 */
        offer("example-file-4096.bin").await?,
    ])
}

/** Send a file using the Rust implementation. This does not guarantee compatibility with Python! ;) */
#[cfg(feature = "transfer")]
#[test(async_std::test)]
// TODO Wasm test disabled, it crashes
// #[cfg_attr(target_arch = "wasm32", test(wasm_bindgen_test::wasm_bindgen_test))]
pub async fn test_file_rust2rust() {
    for (offer, answer) in file_offers().await.unwrap() {
        let (code_tx, code_rx) = futures::channel::oneshot::channel();

        let sender_task = async_std::task::Builder::new()
            .name("sender".to_owned())
            .local(async {
                let mailbox_connection =
                    MailboxConnection::create(transfer::APP_CONFIG.id(TEST_APPID).clone(), 2)
                        .await?;
                if let Some(welcome) = &mailbox_connection.welcome {
                    tracing::info!("Got welcome: {}", welcome);
                }
                tracing::info!("This wormhole's code is: {}", &mailbox_connection.code);
                code_tx.send(mailbox_connection.code.clone()).unwrap();
                let wormhole = crate::Wormhole::connect(mailbox_connection).await?;
                eyre::Result::<_>::Ok(
                    transfer::send(
                        wormhole,
                        default_relay_hints(),
                        magic_wormhole::transit::Abilities::ALL,
                        offer,
                        &log_transit_connection,
                        |_sent, _total| {},
                        futures::future::pending(),
                    )
                    .await?,
                )
            })
            .unwrap();
        let receiver_task = async_std::task::Builder::new()
            .name("receiver".to_owned())
            .local(async {
                let code = code_rx.await?;
                let config = transfer::APP_CONFIG.id(TEST_APPID);
                let mailbox = MailboxConnection::connect(config, code.clone(), false).await?;
                if let Some(welcome) = mailbox.welcome.clone() {
                    tracing::info!("Got welcome: {}", welcome);
                }
                let wormhole = crate::Wormhole::connect(mailbox).await?;

                // Hacky v1-compat conversion for now
                let mut answer =
                    (answer.into_iter_files().next().unwrap().1.content)(false).await?;

                /*let transfer::ReceiveRequest::V1(req) = transfer::request(
                    wormhole,
                    default_relay_hints(),
                    magic_wormhole::transit::Abilities::ALL,
                    futures::future::pending(),
                )
                .await?
                .unwrap() else {
                    panic!("v2 should be disabled for now")
                };*/

                let req = transfer::request_file(
                    wormhole,
                    default_relay_hints(),
                    magic_wormhole::transit::Abilities::ALL,
                    futures::future::pending(),
                )
                .await?
                .unwrap();

                req.accept(
                    &log_transit_connection,
                    |_received, _total| {},
                    &mut answer,
                    futures::future::pending(),
                )
                .await?;
                eyre::Result::<_>::Ok(())
            })
            .unwrap();

        sender_task.await.unwrap();
        receiver_task.await.unwrap();
    }
}

/** Test the functionality used by the `send-many` subcommand.
 */
#[cfg(feature = "transfer")]
#[test(async_std::test)]
// TODO Wasm test disabled, it crashes
// #[cfg_attr(target_arch = "wasm32", test(wasm_bindgen_test::wasm_bindgen_test))]
pub async fn test_send_many() {
    let mailbox = MailboxConnection::create(transfer::APP_CONFIG.id(TEST_APPID), 2)
        .await
        .unwrap();
    let code = mailbox.code.clone();
    tracing::info!("The code is {:?}", code);

    async fn gen_offer() -> eyre::Result<transfer::offer::OfferSend> {
        file_offers().await.map(|mut vec| vec.remove(0).0)
    }

    async fn gen_accept() -> eyre::Result<transfer::offer::OfferAccept> {
        file_offers().await.map(|mut vec| vec.remove(0).1)
    }

    /* Send many */
    let sender_code = code.clone();
    let senders = async_std::task::spawn_local(async move {
        // let mut senders = Vec::<async_std::task::JoinHandle<std::result::Result<std::vec::Vec<u8>, eyre::Error>>>::new();
        let mut senders: Vec<async_std::task::JoinHandle<eyre::Result<()>>> = Vec::new();

        /* The first time, we reuse the current session for sending */
        {
            tracing::info!("Sending file #{}", 0);
            let wormhole = crate::Wormhole::connect(mailbox).await?;
            senders.push(async_std::task::spawn_local(async move {
                eyre::Result::Ok(
                    crate::transfer::send(
                        wormhole,
                        default_relay_hints(),
                        magic_wormhole::transit::Abilities::ALL,
                        gen_offer().await?,
                        &log_transit_connection,
                        |_, _| {},
                        futures::future::pending(),
                    )
                    .await?,
                )
            }));
        }

        for i in 1..5usize {
            tracing::info!("Sending file #{}", i);
            let wormhole = crate::Wormhole::connect(
                MailboxConnection::connect(
                    transfer::APP_CONFIG.id(TEST_APPID),
                    sender_code.clone(),
                    true,
                )
                .await?,
            )
            .await?;
            let gen_offer = gen_offer.clone();
            senders.push(async_std::task::spawn_local(async move {
                eyre::Result::Ok(
                    crate::transfer::send(
                        wormhole,
                        default_relay_hints(),
                        magic_wormhole::transit::Abilities::ALL,
                        gen_offer().await?,
                        &log_transit_connection,
                        |_, _| {},
                        futures::future::pending(),
                    )
                    .await?,
                )
            }));
        }
        eyre::Result::<_>::Ok(senders)
    });

    async_std::task::sleep(std::time::Duration::from_secs(1)).await;

    /* Receive many */
    for i in 0..5usize {
        tracing::info!("Receiving file #{}", i);
        let wormhole = crate::Wormhole::connect(
            MailboxConnection::connect(transfer::APP_CONFIG.id(TEST_APPID), code.clone(), true)
                .await
                .unwrap(),
        )
        .await
        .unwrap();
        tracing::info!("Got key: {}", &wormhole.key);
        /*let transfer::ReceiveRequest::V1(req) = crate::transfer::request(
            wormhole,
            default_relay_hints(),
            magic_wormhole::transit::Abilities::ALL,
            futures::future::pending(),
        )
        .await?
        .unwrap() else {
            panic!("v2 should be disabled for now")
        };*/

        let req = transfer::request_file(
            wormhole,
            default_relay_hints(),
            magic_wormhole::transit::Abilities::ALL,
            futures::future::pending(),
        )
        .await
        .unwrap()
        .unwrap();

        // Hacky v1-compat conversion for now
        let mut answer = (gen_accept()
            .await
            .unwrap()
            .into_iter_files()
            .next()
            .unwrap()
            .1
            .content)(false)
        .await
        .unwrap();

        req.accept(
            &log_transit_connection,
            |_, _| {},
            &mut answer,
            futures::future::pending(),
        )
        .await
        .unwrap();
    }

    for sender in senders.await.unwrap() {
        sender.await.unwrap();
    }
}

/// Try to send a file, but use a bad code, and see how it's handled
#[test(async_std::test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
pub async fn test_wrong_code() {
    let (code_tx, code_rx) = futures::channel::oneshot::channel();

    let sender_task = async_std::task::Builder::new()
        .name("sender".to_owned())
        .local(async {
            let mailbox = MailboxConnection::create(APP_CONFIG, 2).await.unwrap();
            if let Some(welcome) = &mailbox.welcome {
                tracing::info!("Got welcome: {}", welcome);
            }
            let code = mailbox.code.clone();
            tracing::info!("This wormhole's code is: {}", &code);
            code_tx.send(code.nameplate()).unwrap();

            let result = crate::Wormhole::connect(mailbox).await;
            /* This should have failed, due to the wrong code */
            assert!(result.is_err());
            eyre::Result::<_>::Ok(())
        })
        .unwrap();
    let receiver_task = async_std::task::Builder::new()
        .name("receiver".to_owned())
        .local(async {
            let nameplate = code_rx.await?;
            tracing::info!("Got nameplate over local: {}", &nameplate);
            let result = crate::Wormhole::connect(
                MailboxConnection::connect(
                    APP_CONFIG,
                    /* Making a wrong code here by appending nonsense */
                    Code::from_components(nameplate, "foo-bar".parse().unwrap()),
                    true,
                )
                .await
                .unwrap(),
            )
            .await;

            /* This should have failed, due to the wrong code */
            assert!(result.is_err());
            eyre::Result::<_>::Ok(())
        })
        .unwrap();

    async_std::future::timeout(TIMEOUT, sender_task)
        .await
        .unwrap()
        .unwrap();
    async_std::future::timeout(TIMEOUT, receiver_task)
        .await
        .unwrap()
        .unwrap();
}

/** Connect three people to the party and watch it explode … gracefully */
#[test(async_std::test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
pub async fn test_crowded() {
    let initial_mailbox_connection = MailboxConnection::create(APP_CONFIG, 2).await.unwrap();
    tracing::info!("This test's code is: {}", &initial_mailbox_connection.code);
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
        other => panic!("Got wrong error message: {other}, wanted 'crowded'"),
    }
}

#[async_std::test]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
pub async fn test_connect_with_code_expecting_nameplate() {
    let code = generate_random_code();
    let result = MailboxConnection::connect(APP_CONFIG, code.clone(), false).await;
    let error = result.err().unwrap();
    match error {
        magic_wormhole::WormholeError::UnclaimedNameplate(x) => {
            assert_eq!(x, code.nameplate());
        },
        other => panic!("Got wrong error type {other:?}. Expected `NameplateNotFound`"),
    }
}

fn generate_random_code() -> Code {
    let mut rng = rand::thread_rng();
    let nameplate_string = format!("{}-guitarist-revenge", rng.gen_range(1000..10000));
    Code::from_str(&nameplate_string).unwrap()
}

#[test]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn test_phase() {
    let p = Phase::PAKE;
    assert!(p.is_pake());
    assert!(!p.is_version());
}

#[test]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
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
