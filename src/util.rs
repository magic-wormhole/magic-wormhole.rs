use async_std::{io, io::prelude::*};

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
