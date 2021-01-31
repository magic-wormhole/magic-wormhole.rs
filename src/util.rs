use async_std::{io, io::prelude::*};

pub async fn ask_user(message: String, default_yes: bool) -> bool {
    let message = format!(
        "{} ({}/{}) ",
        message,
        if default_yes { "Y" } else { "y" },
        if default_yes { "n" } else { "N" }
    );

    let mut stdout = io::stdout();
    let stdin = io::stdin();

    loop {
        stdout.write(message.as_bytes()).await.unwrap();

        stdout.flush().await.unwrap();

        let mut answer = String::new();
        stdin.read_line(&mut answer).await.unwrap();

        match answer.chars().next().map(|c| c.to_ascii_lowercase()) {
            Some('y') => break true,
            Some('n') => break false,
            None => break default_yes,
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
