/// Various helpers to deal with closing connections and cancellation
use super::*;
use crate::util;
use futures::Future;

/// A weird mixture of [`futures::future::Abortable`], [`async_std::sync::Condvar`] and [`futures::future::Select`] tailored to our Ctrl+C handling.
///
/// At it's core, it is an `Abortable` but instead of having an `AbortHandle`, we use a future that resolves as trigger.
/// Under the hood, it is implementing the same functionality as a `select`, but mapping one of the outcomes to an error type.
pub async fn cancellable<T>(
    future: impl Future<Output = T> + Unpin,
    cancel: impl Future<Output = ()>,
) -> Result<T, Cancelled> {
    use futures::future::Either;
    futures::pin_mut!(cancel);
    match futures::future::select(cancel, future).await {
        Either::Left(((), _)) => Err(Cancelled),
        Either::Right((val, _)) => Ok(val),
    }
}

/** Like `cancellable`, but you'll get back the cancellation future in case the code terminates for future use */
pub async fn cancellable_2<T, C: Future<Output = ()> + Unpin>(
    future: impl Future<Output = T> + Unpin,
    cancel: C,
) -> Result<(T, C), Cancelled> {
    use futures::future::Either;
    match futures::future::select(cancel, future).await {
        Either::Left(((), _)) => Err(Cancelled),
        Either::Right((val, cancel)) => Ok((val, cancel)),
    }
}

/// Indicator that the [`Cancellable`] task was cancelled.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Task has been cancelled")
    }
}

/// Maximum duration that we are willing to wait for cleanup tasks to finish
const SHUTDOWN_TIME: std::time::Duration = std::time::Duration::from_secs(5);

// TODO make function once possible (Rust language limitations etc.)
macro_rules! with_cancel_wormhole {
    ($wormhole:ident, run = $run:expr, $cancel:expr, ret_cancel = $ret_cancel:expr $(,)?) => {{
        let run = Box::pin($run);
        let result = cancel::cancellable_2(run, $cancel).await;
        let Some((transit, wormhole, cancel)) =
            cancel::handle_run_result_noclose($wormhole, result).await?
        else {
            return Ok($ret_cancel);
        };
        (transit, wormhole, cancel)
    }};
}

// Make macro public
pub(super) use with_cancel_wormhole;

// Rustfmt has a bug where it will indent a few lines again and again and again and again and again anda
#[rustfmt::skip]
macro_rules! with_cancel_transit {
    ($transit:ident, run = $run:expr, $cancel:expr, $make_error_message:expr, $parse_message:expr, ret_cancel = $ret_cancel:expr $(,)?) => {{
        let run = Box::pin($run);
        let result = cancel::cancellable_2(run, $cancel).await;
        let Some((value, transit)) = cancel::handle_run_result_transit(
            $transit,
            result,
            $make_error_message,
            $parse_message,
        ).await? else { return Ok($ret_cancel); };
        (value, transit)
    }};
}

// Make macro public
pub(super) use with_cancel_transit;

/// Run a future with timeout and cancellation, ignore errors
async fn wrap_timeout(run: impl Future<Output = ()>, cancel: impl Future<Output = ()>) {
    let run = util::timeout(SHUTDOWN_TIME, run);
    futures::pin_mut!(run);
    match cancellable(run, cancel).await {
        Ok(Ok(())) => {},
        Ok(Err(_timeout)) => log::debug!("Post-transfer timed out"),
        Err(_cancelled) => log::debug!("Post-transfer got cancelled by user"),
    };
}

/// Ignore an error but at least debug print it
fn debug_err(result: Result<(), impl std::fmt::Display>, operation: &str) {
    if let Err(error) = result {
        log::debug!("Failed to {} after transfer: {}", operation, error);
    }
}

/** Handle the post-{transfer, failure, cancellation} logic, then close the Wormhole */
pub async fn handle_run_result(
    wormhole: Wormhole,
    result: Result<(Result<(), TransferError>, impl Future<Output = ()>), Cancelled>,
) -> Result<(), TransferError> {
    match handle_run_result_noclose(wormhole, result).await {
        Ok(Some(((), mut wormhole, cancel))) => {
            /* Happy case: everything went okay. Now close the wormholhe */
            log::debug!("Transfer done, doing cleanup logic");
            wrap_timeout(
                async {
                    debug_err(wormhole.close().await, "close Wormhole");
                },
                cancel,
            )
            .await;
            Ok(())
        },
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    }
}

/** Handle the post-{transfer, failure, cancellation} logic */
pub async fn handle_run_result_noclose<T, C: Future<Output = ()>>(
    mut wormhole: Wormhole,
    result: Result<(Result<T, TransferError>, C), Cancelled>,
) -> Result<Option<(T, Wormhole, C)>, TransferError> {
    match result {
        /* Happy case: everything went okay */
        Ok((Ok(val), cancel)) => Ok(Some((val, wormhole, cancel))),
        /* Got peer error: stop everything immediately */
        Ok((Err(error @ TransferError::PeerError(_)), cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            wrap_timeout(
                async {
                    debug_err(wormhole.close().await, "close Wormhole");
                },
                cancel,
            )
            .await;
            Err(error)
        },
        /* Got transit error: try to receive peer error for better error message */
        Ok((Err(mut error @ TransferError::Transit(_)), cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            wrap_timeout(async {
                /* If transit failed, ask for a proper error and potentially use that instead */
                // TODO this should be replaced with some try_receive that only polls already available messages,
                // and we should not only look for the next one but all have been received
                // and we should not interrupt a receive operation without making sure it leaves the connection
                // in a consistent state, otherwise the shutdown may cause protocol errors
                if let Ok(Ok(Ok(PeerMessage::Error(e)))) = util::timeout(SHUTDOWN_TIME / 3, wormhole.receive_json()).await {
                    error = TransferError::PeerError(e);
                } else {
                    log::debug!("Failed to retrieve more specific error message from peer. Maybe it crashed?");
                }
                debug_err(wormhole.close().await, "close Wormhole");
            }, cancel).await;
            Err(error)
        },
        /* Other error: try to notify peer */
        Ok((Err(error), cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            wrap_timeout(
                async {
                    debug_err(
                        wormhole
                            .send_json(&PeerMessage::Error(format!("{}", error)))
                            .await,
                        "notify peer about the error",
                    );
                    debug_err(wormhole.close().await, "close Wormhole");
                },
                cancel,
            )
            .await;
            Err(error)
        },
        /* Cancelled: try to notify peer */
        Err(cancelled) => {
            log::debug!("Transfer got cancelled, doing cleanup logic");
            /* Replace cancel with ever-pending future, as we have already been cancelled */
            wrap_timeout(
                async {
                    debug_err(
                        wormhole
                            .send_json(&PeerMessage::Error(format!("{}", cancelled)))
                            .await,
                        "notify peer about our cancellation",
                    );
                    debug_err(wormhole.close().await, "close Wormhole");
                },
                futures::future::pending(),
            )
            .await;
            Ok(None)
        },
    }
}

/**
 * Handle the post-{transfer, failure, cancellation} logic where the error signaling is done over the transit channel
 */
pub async fn handle_run_result_transit<T>(
    mut transit: transit::Transit,
    result: Result<(Result<T, TransferError>, impl Future<Output = ()>), Cancelled>,
    make_error_message: impl FnOnce(&(dyn std::string::ToString + Sync)) -> Vec<u8>,
    parse_message: impl Fn(&[u8]) -> Result<Option<String>, TransferError>,
) -> Result<Option<(T, transit::Transit)>, TransferError> {
    match result {
        /* Happy case: everything went okay */
        Ok((Ok(val), _cancel)) => Ok(Some((val, transit))),
        /* Got peer error: stop everything immediately */
        Ok((Err(error @ TransferError::PeerError(_)), _cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            Err(error)
        },
        /* Got transit error: try to receive peer error for better error message */
        Ok((Err(mut error @ TransferError::Transit(_)), cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            wrap_timeout(
                async {
                    /* Receive one peer message to see if they sent some error prior to closing
                     * (Note that this will only happen if we noticed the closed connection while trying to send,
                     * otherwise receiving will already yield the error message).
                     */
                    loop {
                        let Ok(msg) = transit.receive_record().await else {
                            break;
                        };
                        match parse_message(&msg) {
                            Ok(None) => continue,
                            Ok(Some(err)) => {
                                error = TransferError::PeerError(err);
                                break;
                            },
                            Err(_) => break,
                        }
                    }
                },
                cancel,
            )
            .await;
            Err(error)
        },
        /* Other error: try to notify peer */
        Ok((Err(error), cancel)) => {
            log::debug!(
                "Transfer encountered an error ({}), doing cleanup logic",
                error
            );
            wrap_timeout(
                async {
                    debug_err(
                        transit.send_record(&make_error_message(&error)).await,
                        "notify peer about the error",
                    );
                },
                cancel,
            )
            .await;
            Err(error)
        },
        /* Cancelled: try to notify peer */
        Err(cancelled) => {
            log::debug!("Transfer got cancelled, doing cleanup logic");
            /* Replace cancel with ever-pending future, as we have already been cancelled */
            wrap_timeout(
                async {
                    debug_err(
                        transit.send_record(&make_error_message(&cancelled)).await,
                        "notify peer about our cancellation",
                    );
                },
                futures::future::pending(),
            )
            .await;
            Ok(None)
        },
    }
}
