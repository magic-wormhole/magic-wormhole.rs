use std::fmt::Arguments;

use async_std::{io, io::prelude::*};

pub async fn ask_user(message: Arguments<'_>) -> bool {
    let mut stdout = io::stdout();
    let stdin = io::stdin();

    loop {
        stdout.write_fmt(message).await.unwrap();

        stdout.flush().await.unwrap();

        let mut answer = String::new();
        stdin.read_line(&mut answer).await.unwrap();

        match answer.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('y') => break true,
            Some('n') => break false,
            _ => {
                stdout
                    .write_fmt(format_args!("Please type y or n!\n"))
                    .await
                    .unwrap();
                stdout.flush().await.unwrap();
                continue;
            },
        };
    }
}
