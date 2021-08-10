use async_std::{io, io::prelude::*};

#[macro_export]
macro_rules! ensure {
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return std::result::Result::Err($err.into());
        }
    };
}

#[macro_export]
macro_rules! bail {
    ($err:expr $(,)?) => {
        return std::result::Result::Err($err.into());
    };
}

/// A warpper around `&[u8]` that implements [`std::fmt::Display`] in a more intelligent+ way.
pub struct DisplayBytes<'a>(pub &'a [u8]);

impl std::fmt::Display for DisplayBytes<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex_decode = hex::decode(&self.0);
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

pub async fn ask_user(message: String, default_answer: bool) -> bool {
    let message = format!(
        "{} ({}/{}) ",
        message,
        if default_answer { "Y" } else { "y" },
        if default_answer { "n" } else { "N" }
    );

    let mut stdout = io::stdout();
    let stdin = io::stdin();

    loop {
        stdout.write(message.as_bytes()).await.unwrap();

        stdout.flush().await.unwrap();

        let mut answer = String::new();
        stdin.read_line(&mut answer).await.unwrap();

        match &*answer.to_lowercase().trim() {
            "y" | "yes" => break true,
            "n" | "no" => break false,
            "" => break default_answer,
            _ => {
                stdout
                    .write("Please type y or n!\n".as_bytes())
                    .await
                    .unwrap();
                stdout.flush().await.unwrap();
                continue;
            },
        };
    }
}
