use smol::{
    Unblock,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
};

pub async fn ask_user(message: impl std::fmt::Display, default_answer: bool) -> bool {
    let message = format!(
        "{} ({}/{}) ",
        message,
        if default_answer { "Y" } else { "y" },
        if default_answer { "n" } else { "N" }
    );

    let mut stdout = Unblock::new(std::io::stdout());
    let mut stdin = BufReader::new(Unblock::new(std::io::stdin()));

    loop {
        stdout.write(message.as_bytes()).await.unwrap();
        stdout.flush().await.unwrap();

        let mut answer = String::new();
        stdin.read_line(&mut answer).await.unwrap();

        match answer.to_lowercase().trim() {
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

/// At it's core, it is an `Abortable` but instead of having an `AbortHandle`, we use a future that resolves as trigger.
/// Under the hood, it is implementing the same functionality as a `select`, but mapping one of the outcomes to an error type.
pub async fn cancellable<T>(
    future: impl Future<Output = T> + Unpin,
    cancel: impl Future<Output = ()>,
) -> Result<T, Cancelled> {
    use futures::future::Either;
    futures::pin_mut!(cancel);
    match futures::future::select(future, cancel).await {
        Either::Left((val, _)) => Ok(val),
        Either::Right(((), _)) => Err(Cancelled),
    }
}

/// Indicator that the cancellable task was cancelled.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Task has been cancelled")
    }
}
