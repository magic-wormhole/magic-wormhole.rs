macro_rules! ensure {
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return std::result::Result::Err($err.into());
        }
    };
}

macro_rules! bail {
    ($err:expr $(,)?) => {{
        return std::result::Result::Err($err.into());
    }};
}

/// A warpper around `&[u8]` that implements [`std::fmt::Display`] in a more intelligent+ way.
pub struct DisplayBytes<'a>(pub &'a [u8]);

impl std::fmt::Display for DisplayBytes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_decode = hex::decode(self.0);
        let (string, hex_param) = match hex_decode.as_ref().map(Vec::as_slice) {
            Ok(decoded_hex) => (decoded_hex, "hex-encoded "),
            Err(_) => (self.0, ""),
        };

        let string = match std::str::from_utf8(string) {
            Ok(string) => string,
            Err(_) => {
                return f.write_fmt(format_args!("<{} {}bytes>", string.len(), hex_param));
            },
        };

        match string.parse::<serde_json::Value>() {
            Ok(serde_json::Value::Object(map)) => {
                if map.len() == 1 {
                    return f.write_fmt(format_args!(
                        "<{}JSON dict with key '{}'>",
                        hex_param,
                        map.keys().next().unwrap()
                    ));
                } else if map.contains_key("type") {
                    return f.write_fmt(format_args!(
                        "<{}JSON dict of type '{}'>",
                        hex_param,
                        map.get("type").unwrap()
                    ));
                } else {
                    return f.write_fmt(format_args!(
                        "<{}JSON dict with {} keys>",
                        hex_param,
                        map.len()
                    ));
                }
            },
            Ok(serde_json::Value::Array(list)) => {
                return f.write_fmt(format_args!(
                    "<{}JSON array with {} entry/ies>",
                    hex_param,
                    list.len()
                ));
            },
            _ => (),
        }

        if string.len() > 20 {
            f.write_fmt(format_args!("\"{:.15}…\"", string.replace('"', "\\\"")))?;
        } else {
            f.write_fmt(format_args!("\"{}\"", string.replace('"', "\\\"")))?;
        }

        Ok(())
    }
}

/**
 * Native reimplementation of [`sodiumoxide::utils::increment_le](https://docs.rs/sodiumoxide/0.2.6/sodiumoxide/utils/fn.increment_le.html).
 * TODO remove after https://github.com/quininer/memsec/issues/11 is resolved.
 * Original implementation: https://github.com/jedisct1/libsodium/blob/6d566070b48efd2fa099bbe9822914455150aba9/src/libsodium/sodium/utils.c#L262-L307
 */
#[allow(unused)]
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
    use rand::{distributions::Standard, Rng};
    use sha1::{Digest, Sha1};

    if bits > 32 {
        log::warn!(
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
    /* I'm pretty sure HashCash should work with any time zone */
    let date = base64::encode(
        time::OffsetDateTime::now_utc()
            .date()
            .format(&format[..])
            .unwrap(),
    );

    let rand: String = base64::encode(
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
            base64::encode(counter)
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
#[cfg(not(target_family = "wasm"))]
pub async fn sleep(duration: std::time::Duration) {
    async_std::task::sleep(duration).await
}

#[cfg(target_family = "wasm")]
pub async fn sleep(duration: std::time::Duration) {
    /* Skip error handling. Waiting is best effort anyways */
    let _ = wasm_timer::Delay::new(duration).await;
}

#[cfg(not(target_family = "wasm"))]
pub async fn timeout<F, T>(
    duration: std::time::Duration,
    future: F,
) -> Result<T, async_std::future::TimeoutError>
where
    F: futures::Future<Output = T>,
{
    async_std::future::timeout(duration, future).await
}

#[cfg(target_family = "wasm")]
pub async fn timeout<F, T>(duration: std::time::Duration, future: F) -> Result<T, std::io::Error>
where
    F: futures::Future<Output = T>,
{
    use futures::FutureExt;
    use wasm_timer::TryFutureExt;
    future.map(Result::Ok).timeout(duration).await
}
