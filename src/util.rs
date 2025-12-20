use std::sync::LazyLock;

use base64::Engine;

macro_rules! ensure {
    ($cond:expr_2021, $err:expr_2021 $(,)?) => {
        if !$cond {
            return std::result::Result::Err($err.into());
        }
    };
}

macro_rules! bail {
    ($err:expr_2021 $(,)?) => {{
        return std::result::Result::Err($err.into());
    }};
}

/**
 * Native reimplementation of [`sodiumoxide::utils::increment_le](https://docs.rs/sodiumoxide/0.2.6/sodiumoxide/utils/fn.increment_le.html).
 * TODO remove after https://github.com/quininer/memsec/issues/11 is resolved.
 * Original implementation: https://github.com/jedisct1/libsodium/blob/6d566070b48efd2fa099bbe9822914455150aba9/src/libsodium/sodium/utils.c#L262-L307
 */
#[expect(unused)]
pub fn sodium_increment_le(n: &mut [u8]) {
    let mut c = 1u16;
    for b in n {
        c += *b as u16;
        *b = c as u8;
        c >>= 8;
    }
}

pub fn sodium_increment_be(n: &mut [u8]) {
    let mut c = 1u16;
    for b in n.iter_mut().rev() {
        c += *b as u16;
        *b = c as u8;
        c >>= 8;
    }
}

/** Mint a new hashcash token with a given difficulty and resource string. */
pub fn hashcash(resource: String, bits: u32) -> String {
    use rand::{Rng, distributions::Standard};
    use sha1::{Digest, Sha1};

    if bits > 32 {
        tracing::warn!(
            "Minting a hashcash token with {} bits. If the application is frozen, you'll know why",
            bits
        );
    }

    /* This is the `[year][month][day]` format, but without activating the parser */
    use time::format_description::{Component, FormatItem};
    let format = [
        FormatItem::Component(Component::Year(
            time::format_description::modifier::Year::default(),
        )),
        FormatItem::Component(Component::Month(
            time::format_description::modifier::Month::default(),
        )),
        FormatItem::Component(Component::Day(
            time::format_description::modifier::Day::default(),
        )),
    ];

    let base64_engine = base64::engine::general_purpose::STANDARD;

    /* I'm pretty sure HashCash should work with any time zone */
    let date = base64_engine.encode(
        time::OffsetDateTime::now_utc()
            .date()
            .format(&format[..])
            .unwrap(),
    );

    let rand: String = base64_engine.encode(
        rand::thread_rng()
            .sample_iter(&Standard)
            .take(16)
            .collect::<Vec<u8>>(),
    );

    /* 64 bit counter should suffice */
    let mut counter = [0; 8];
    let mut hasher = Sha1::new();

    loop {
        sodium_increment_be(&mut counter);

        let stamp = format!(
            "1:{}:{}:{}::{}:{}",
            bits,
            date,
            resource,
            rand,
            base64_engine.encode(counter)
        );

        hasher.update(&stamp);
        let result = hasher.finalize_reset();

        let mut leading_zeros = 0;
        for byte in result {
            let front_zeros = byte.leading_zeros();
            leading_zeros += front_zeros;

            if front_zeros < 8 {
                break;
            }
        }

        if leading_zeros >= bits {
            return stamp;
        }
    }
}

/// The error type returned by [`timeout`]
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("Timed out")]
pub(crate) struct TimeoutError;

/// Utility function to add a timeout to a future
///
/// This behaves the same as async std timeout, but with async-io
#[cfg(not(target_family = "wasm"))]
pub(crate) fn timeout<'a, R, F: std::future::Future<Output = R> + 'a>(
    timeout: std::time::Duration,
    future: F,
) -> impl Future<Output = Result<R, TimeoutError>> + 'a {
    let timeout_future = async move {
        async_io::Timer::after(timeout).await;
        Err(TimeoutError)
    };

    futures_lite::future::or(async { Ok(future.await) }, timeout_future)
}

/// Utility function to spawn a future. We don't use crate::util::spawn, because not the entirety of smol compiles on WASM
pub(crate) fn spawn<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
) -> async_task::Task<T> {
    static EXECUTOR: LazyLock<async_executor::Executor> =
        LazyLock::new(async_executor::Executor::new);

    EXECUTOR.spawn(future)
}
