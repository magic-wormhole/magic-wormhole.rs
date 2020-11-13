use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, AppSettings, Arg, SubCommand,
};
use log::*;
use magic_wormhole::CodeProvider;
use magic_wormhole::{transfer, Wormhole};
use std::path::Path;
use std::str;

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        // .filter_module("magic_wormhole::core", LevelFilter::Trace)
        .filter_module("mio", LevelFilter::Debug)
        .filter_module("ws", LevelFilter::Error)
        .init();
    let relay_server_arg = Arg::with_name("relay-server")
        .long("relay-server")
        .visible_alias("relay")
        .takes_value(true)
        .value_name("tcp:HOSTNAME:PORT")
        .help("Use a custom relay server");
    let send_command = SubCommand::with_name("send")
        .visible_alias("tx")
        .arg(
            Arg::with_name("code-length")
                .short("c")
                .long("code-length")
                .takes_value(true)
                .value_name("NUMWORDS")
                .default_value("2")
                .help("Length of code (in bytes/words)"),
        )
        .arg(
            Arg::with_name("hide-progress")
                .long("hide-progress")
                .help("Suppress progress-bar display"),
        )
        .arg(
            Arg::with_name("code")
                .long("code")
                .takes_value(true)
                .value_name("CODE")
                .help("Enter a code instead of generating one automatically"),
        )
        .arg(relay_server_arg.clone())
        .arg(
            Arg::with_name("file")
                .index(1)
                .required(true)
                .value_name("FILENAME|DIRNAME")
                .help("The file or directory to send"),
        );
    let send_many_command = SubCommand::with_name("send-many")
        .help("Send a file to many people with the same code.")
        .arg(
            Arg::with_name("code-length")
                .short("c")
                .long("code-length")
                .takes_value(true)
                .value_name("NUMWORDS")
                .default_value("2")
                .help("Length of code (in bytes/words)"),
        )
        .arg(
            Arg::with_name("code")
                .long("code")
                .takes_value(true)
                .value_name("CODE")
                .help("Enter a code instead of generating one automatically"),
        )
        .arg(relay_server_arg.clone())
        .arg(
            Arg::with_name("file")
                .index(1)
                .required(true)
                .value_name("FILENAME|DIRNAME")
                .help("The file or directory to send"),
        );
    let receive_command = SubCommand::with_name("receive")
        .visible_alias("rx")
        .arg(
            Arg::with_name("verify")
                .long("verify")
                .help("display verification string (and wait for approval)"),
        )
        .arg(
            Arg::with_name("hide-progress")
                .long("hide-progress")
                .help("supress progress-bar display"),
        )
        .arg(
            Arg::with_name("noconfirm")
                .long("noconfirm")
                .visible_alias("yes")
                .help("Accept file transfer without asking for confirmation"),
        )
        .arg(
            Arg::with_name("output-file")
                .short("o")
                .long("output-file")
                .visible_alias("out")
                .takes_value(true)
                .value_name("FILENAME|DIRNAME")
                .help(
                    "The file or directory to create, overriding the name suggested by the sender",
                ),
        )
        .arg(
            Arg::with_name("code")
                .index(1)
                .value_name("CODE")
                .help("Provide the code now rather than typing it interactively"),
        )
        .arg(relay_server_arg);

    let clap = App::new(crate_name!())
        .version(crate_version!())
        .about(crate_description!())
        .author(crate_authors!())
        .setting(AppSettings::AllowExternalSubcommands)
        .setting(AppSettings::ArgRequiredElseHelp)
        .setting(AppSettings::VersionlessSubcommands)
        .subcommand(send_command)
        .subcommand(send_many_command)
        .subcommand(receive_command);
    let matches = clap.get_matches();

    if let Some(matches) = matches.subcommand_matches("send") {
        let relay_server = matches
            .value_of("relay-server")
            .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER);
        let (welcome, connector) = magic_wormhole::connect_to_server(
            magic_wormhole::transfer::APPID,
            magic_wormhole::transfer::AppVersion::default(),
            magic_wormhole::DEFAULT_MAILBOX_SERVER,
            match matches.value_of("code") {
                None => {
                    let numwords = matches
                        .value_of("code-length")
                        .unwrap()
                        .parse()
                        .expect("TODO error handling");
                    CodeProvider::AllocateCode(numwords)
                },
                Some(code) => CodeProvider::SetCode(code.to_string()),
            },
        )
        .await;
        info!("Got welcome: {}", &welcome.welcome);
        info!("This wormhole's code is: {}", &welcome.code);
        info!("On the other computer, please run:\n");
        info!("wormhole receive {}\n", &welcome.code);
        let mut wormhole = connector.connect_to_client().await;
        info!("Got key: {}", wormhole.key);
        let file = matches.value_of("file").unwrap();
        transfer::send_file(&mut wormhole, file, &relay_server.parse().unwrap())
            .await
            .unwrap();
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let relay_server = matches
            .value_of("relay-server")
            .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER);
        let (welcome, connector) = magic_wormhole::connect_to_server(
            magic_wormhole::transfer::APPID,
            magic_wormhole::transfer::AppVersion::default(),
            magic_wormhole::DEFAULT_MAILBOX_SERVER,
            match matches.value_of("code") {
                None => {
                    let numwords = matches
                        .value_of("code-length")
                        .unwrap()
                        .parse()
                        .expect("TODO error handling");
                    CodeProvider::AllocateCode(numwords)
                },
                Some(code) => CodeProvider::SetCode(code.to_string()),
            },
        )
        .await;
        /* Explicitely close connection */
        std::mem::drop(connector);
        let file = matches.value_of("file").unwrap();

        info!("Got welcome: {}", &welcome.welcome);
        info!("This wormhole's code is: {}", &welcome.code);
        info!("On the other computer, please run:\n");
        info!("wormhole receive {}\n", &welcome.code);
        send_many(relay_server, &welcome.code, file).await?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let relay_server = matches
            .value_of("relay-server")
            .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER);
        let code = matches
            .value_of("code")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| enter_code().expect("TODO handle this gracefully"));

        let (_welcome, connector) = magic_wormhole::connect_to_server(
            magic_wormhole::transfer::APPID,
            magic_wormhole::transfer::AppVersion::default(),
            magic_wormhole::DEFAULT_MAILBOX_SERVER,
            CodeProvider::SetCode(code.trim().to_owned()),
        )
        .await;
        let w = connector.connect_to_client().await;

        receive(w, relay_server).await?;
    } else {
        let _code = matches.subcommand_name();
        // TODO implement this properly once clap 3.0 is out
        // if might_be_code(code) {
        warn!("No command provided, assuming you simply want to receive a file.");
        warn!("To receive files, use `wormhole receive <CODE>`.");
        warn!("To list all available commands and options, type `wormhole --help`");
        warn!(
            "Please refer to `{} --help` for further usage instructions.",
            crate_name!()
        );
        // } else {
        // clap.print_long_help();
        // }
        unimplemented!();
    }

    Ok(())
}

fn _might_be_code(_code: Option<&str>) -> bool {
    // let re = Regex::new(r"\d+-\w+-\w+").unwrap();
    // if !re.is_match(&line) {
    //     panic!("Not a valid code format");
    // }
    unimplemented!()
}

fn enter_code() -> anyhow::Result<String> {
    info!("Enter code: ");
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    Ok(code)
}

async fn send(
    mut w: Wormhole,
    relay_server: &str,
    filename: impl AsRef<Path>,
) -> anyhow::Result<()> {
    let result = transfer::send_file(&mut w, filename, &relay_server.parse().unwrap()).await;
    result
}

async fn send_many(
    relay_server: &str,
    code: &str,
    filename: impl AsRef<Path>,
) -> anyhow::Result<()> {
    loop {
        match {
            let (_welcome, connector) = magic_wormhole::connect_to_server(
                magic_wormhole::transfer::APPID,
                magic_wormhole::transfer::AppVersion::default(),
                magic_wormhole::DEFAULT_MAILBOX_SERVER,
                CodeProvider::SetCode(code.to_owned()),
            )
            .await;
            let mut wormhole = connector.connect_to_client().await;
            let result =
                transfer::send_file(&mut wormhole, &filename, &relay_server.parse().unwrap()).await;
            result
        } {
            Ok(_) => {
                info!("TOOD success message");
            },
            Err(e) => {
                warn!("Send failed, {}", e);
            },
        }
    }
    // Ok(())
}

async fn receive(mut w: Wormhole, relay_server: &str) -> anyhow::Result<()> {
    use async_std::io;
    use async_std::prelude::*;

    let req = transfer::request_file(&mut w, &relay_server.parse().unwrap()).await?;

    let mut stdout = io::stdout();
    let stdin = io::stdin();

    let answer = loop {
        stdout
            .write_fmt(format_args!(
                "Receive file '{}' (size: {} bytes)? (y/N) ",
                req.filename.display(),
                req.filesize
            ))
            .await
            .unwrap();

        stdout.flush().await.unwrap();

        let mut answer = String::new();
        stdin.read_line(&mut answer).await.unwrap();

        match answer.chars().next() {
            Some('y') | Some('Y') => break true,
            Some('n') | Some('N') => break false,
            _ => {
                stdout
                    .write_fmt(format_args!("Please type y or n!\n"))
                    .await
                    .unwrap();
                stdout.flush().await.unwrap();
                continue;
            },
        };
    };

    if answer {
        req.accept().await
    } else {
        req.reject().await
    }
}
