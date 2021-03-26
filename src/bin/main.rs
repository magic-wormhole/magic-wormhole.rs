use std::{
    ops::Deref,
    str,
    time::{Duration, Instant},
};

use anyhow::anyhow;
use async_std::{sync::Arc, task};
use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, AppSettings, Arg, ArgMatches,
    SubCommand,
};
use log::*;
use pbr::{ProgressBar, Units};

use magic_wormhole::{
    transfer, transit::RelayUrl, util, CodeProvider, Wormhole, WormholeConnector,
};
use std::str::FromStr;

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
        )
        .arg(
            Arg::with_name("timeout")
                .long("timeout")
                .takes_value(true)
                .help("Suppress progress-bar display"),
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
        .await?;
        info!("Got welcome: {}", &welcome.welcome);
        info!("This wormhole's code is: {}", &welcome.code);
        info!("On the other computer, please run:\n");
        info!("wormhole receive {}\n", &welcome.code);
        let mut wormhole = connector.connect_to_client().await?;
        info!("Got key: {}", wormhole.key);

        let mut pb = ProgressBar::new(0);
        pb.format("╢▌▌░╟");
        pb.set_units(Units::Bytes);

        let file = matches.value_of("file").unwrap();
        transfer::send_file(
            &mut wormhole,
            file,
            &relay_server.parse().unwrap(),
            move |sent, total| match sent {
                0 => {
                    pb.total = total;
                },
                progress => {
                    pb.set(progress);
                },
            },
        )
        .await
        .unwrap();
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        on_send_many_command(matches).await?;
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
        .await?;
        let w = connector.connect_to_client().await?;

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

async fn on_send_many_command(matches: &ArgMatches<'_>) -> anyhow::Result<()> {
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
    .await?;
    /* Explicitely close connection */
    connector.cancel().await;
    let file = matches.value_of("file").unwrap();
    let timeout_secs = u64::from_str(matches.value_of("timeout").unwrap_or("3600"))?;
    let timeout = Duration::from_secs(timeout_secs);

    info!("Got welcome: {}", &welcome.welcome);
    info!("This wormhole's code is: {}", &welcome.code);
    info!("On the other computer, please run:\n");
    info!("wormhole receive {}\n", &welcome.code);
    send_many(relay_server, &welcome.code, file, timeout).await
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

async fn send_many(
    relay_server: &str,
    code: &str,
    filename: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let time = Instant::now();

    let filename = Arc::new(filename.to_owned());
    let url = Arc::new(relay_server.parse().map_err(|e| anyhow!("{}", e))?);

    while time.elapsed() < timeout {
        let (_welcome, connector) = magic_wormhole::connect_to_server(
            magic_wormhole::transfer::APPID,
            magic_wormhole::transfer::AppVersion::default(),
            magic_wormhole::DEFAULT_MAILBOX_SERVER,
            CodeProvider::SetCode(code.to_owned()),
        )
        .await?;
        send_in_background(Arc::clone(&url), Arc::clone(&filename), connector).await?;
    }
    Ok(())
}

async fn send_in_background(
    url: Arc<RelayUrl>,
    filename: Arc<String>,
    connector: WormholeConnector,
) -> anyhow::Result<()> {
    let mut wormhole = connector.connect_to_client().await?;
    task::spawn(async move {
        let result = transfer::send_file(&mut wormhole, filename.deref(), &url, |sent, total| {
            // @TODO: Not sure what kind of experience is best here.
            info!("Sent {} of {} bytes", sent, total);
        })
        .await;
        match result {
            Ok(_) => info!("TODO success message"),
            Err(e) => warn!("Send failed, {}", e),
        };
    });
    Ok(())
}

async fn receive(mut w: Wormhole, relay_server: &str) -> anyhow::Result<()> {
    let req = transfer::request_file(&mut w, &relay_server.parse().unwrap()).await?;

    let answer = util::ask_user(
        format!(
            "Receive file '{}' (size: {} bytes)?",
            req.filename.display(),
            req.filesize
        ),
        false,
    )
    .await;

    /*
     * Control flow is a bit tricky here:
     * - First of all, we ask if we want to receive the file at all
     * - Then, we check if the file already exists
     * - If it exists, ask whether to overwrite and act accordingly
     * - If it doesn't, directly accept, but DON'T overwrite any files
     */
    if answer {
        let mut pb = ProgressBar::new(req.filesize);
        pb.format("╢▌▌░╟");
        pb.set_units(Units::Bytes);

        let on_progress = move |received, _total| {
            pb.set(received);
        };

        if req.filename.exists() {
            let overwrite = util::ask_user(
                format!("Override existing file {}?", req.filename.display()),
                false,
            )
            .await;
            if overwrite {
                req.accept(true, on_progress).await
            } else {
                req.reject().await
            }
        } else {
            req.accept(false, on_progress).await
        }
    } else {
        req.reject().await
    }
}
