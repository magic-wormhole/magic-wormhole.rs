use std::{
    ops::Deref,
    str,
    time::{Duration, Instant},
};

use anyhow::Context;
use async_std::{fs::OpenOptions, sync::Arc};
use clap::{
    crate_description, crate_name, crate_version, App, AppSettings, Arg, ArgMatches,
    SubCommand,
};
use console::{Term, style};
use indicatif::{ProgressBar, MultiProgress};
use std::io::Write;

use magic_wormhole::{
    transfer, transit::RelayUrl, util, CodeProvider, Wormhole, WormholeConnector, WormholeWelcome,
};
use std::str::FromStr;

#[async_std::main]
async fn main() -> anyhow::Result<()> {
    /* Define some common arguments first */

    let relay_server_arg = Arg::with_name("relay-server")
        .long("relay-server")
        .visible_alias("relay")
        .takes_value(true)
        .multiple(true)
        .value_name("tcp:HOSTNAME:PORT")
        .help("Use a custom relay server (specify multiple times for multiple relays)");
    let rendezvous_server_arg = Arg::with_name("rendezvous-server")
        .long("rendezvous-server")
        .takes_value(true)
        .value_name("ws:URL")
        .help("Use a custom rendezvous server. Both sides need to use the same value in order to find each other.");
    let log_arg = Arg::with_name("log")
        .long("log")
        .help("Enable logging to stdout, for debugging purposes");
    let code_length_arg = Arg::with_name("code-length")
        .short("c")
        .long("code-length")
        .takes_value(true)
        .value_name("NUMWORDS")
        .default_value("2")
        .help("Length of code (in bytes/words)");
    /* Use in send commands */
    let file_name = Arg::with_name("file-name")
        .long("rename")
        .visible_alias("name")
        .takes_value(true)
        .value_name("FILE_NAME")
        .help("Suggest a different name to the receiver. They won't know the file's actual name on your disk.");
    /* Use in receive commands */
    let file_rename = Arg::with_name("file-name")
        .long("rename")
        .visible_alias("name")
        .takes_value(true)
        .value_name("FILE_NAME")
        .help("Rename the received file or folder, overriding the name suggested by the sender.");
    let file_path = Arg::with_name("file-path")
        .long("out-dir")
        .takes_value(true)
        .value_name("PATH")
        .required(true)
        .default_value(".")
        .help("Store transferred file or folder in the specified directory. Defaults to $PWD.");

    /* The subcommands here */

    let send_command = SubCommand::with_name("send")
        .visible_alias("tx")
        .about("Send a file or a folder")
        .arg(code_length_arg.clone())
        .arg(
            Arg::with_name("code")
                .long("code")
                .takes_value(true)
                .value_name("CODE")
                .help("Enter a code instead of generating one automatically"),
        )
        .arg(relay_server_arg.clone())
        .arg(rendezvous_server_arg.clone())
        .arg(file_name.clone())
        .arg(
            Arg::with_name("file")
                .index(1)
                .required(true)
                .value_name("FILENAME|DIRNAME")
                .help("The file or directory to send"),
        );
    let send_many_command = SubCommand::with_name("send-many")
        .about("Send a file to many recipients. READ HELP PAGE FIRST!")
        .after_help(
            "This works by sending the file in a loop with the same code over \
            and over again. Note that this also gives an attacker multiple tries \
            to guess the code, whereas normally they have only one. Only use this \
            for non critical files. Alternatively, you can increase the code length \
            to counter the attack.",
        )
        .arg(code_length_arg)
        .arg(
            Arg::with_name("code")
                .long("code")
                .takes_value(true)
                .value_name("CODE")
                .help("Enter a code instead of generating one automatically"),
        )
        .arg(relay_server_arg.clone())
        .arg(rendezvous_server_arg.clone())
        .arg(file_name)
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
        .about("Receive a file or a folder")
        .arg(
            Arg::with_name("noconfirm")
                .long("noconfirm")
                .visible_alias("yes")
                .help("Accept file transfer without asking for confirmation"),
        )
        .arg(file_rename)
        .arg(file_path)
        .arg(
            Arg::with_name("code")
                .index(1)
                .value_name("CODE")
                .help("Provide the code now rather than typing it interactively"),
        )
        .arg(relay_server_arg)
        .arg(rendezvous_server_arg);

    /* The Clap application */
    let clap = App::new(crate_name!())
        .version(crate_version!())
        .about(crate_description!())
        .setting(AppSettings::AllowExternalSubcommands)
        .setting(AppSettings::ArgRequiredElseHelp)
        .setting(AppSettings::VersionlessSubcommands)
        .setting(AppSettings::DisableHelpSubcommand)
        .global_setting(AppSettings::ColoredHelp)
        .global_setting(AppSettings::ColorAuto)
        .global_setting(AppSettings::UnifiedHelpMessage)
        .after_help(
            "Run a subcommand with `--help` to know how it's used.\n\
                     To send files, use `wormhole send <PATH>`.\n\
                     To receive files, use `wormhole receive <CODE>`.",
        )
        .subcommand(send_command)
        .subcommand(send_many_command)
        .subcommand(receive_command)
        .arg(log_arg);
    let matches = clap.get_matches();

    let mut term = Term::stdout();

    if matches.is_present("log") {
        env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            // .filter_module("magic_wormhole::core", LevelFilter::Trace)
            .filter_module("mio", log::LevelFilter::Debug)
            .filter_module("ws", log::LevelFilter::Error)
            .init();
        log::debug!("Logging enabled.");
    }

    /* Handling of the argument matches (one branch per subcommand) */

    if let Some(matches) = matches.subcommand_matches("send") {
        let (welcome, connector, relay_server) = parse_and_connect(&matches, true).await?;
        print_welcome(&mut term, &welcome)?;
        sender_print_code(&mut term, &welcome.code)?;
        let wormhole = connector.connect_to_client().await?;
        writeln!(&term, "Successfully connected to peer.")?;

        let file_path = matches.value_of_os("file").unwrap();
        let file_name = matches.value_of_os("file-name")
            .or_else(|| std::path::Path::new(file_path).file_name())
            .ok_or_else(|| anyhow::format_err!("You can't send a file without a name. Maybe try --rename"))?;

        send(wormhole,
            &relay_server,
            &file_path,
            &file_name,
        ).await?;
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let (welcome, connector, relay_server) = parse_and_connect(&matches, true).await?;
        let timeout_secs = u64::from_str(matches.value_of("timeout").unwrap_or("3600"))?;
        let timeout = Duration::from_secs(timeout_secs);

        print_welcome(&mut term, &welcome)?;
        sender_print_code(&mut term, &welcome.code)?;

        let file_path = matches.value_of_os("file").unwrap();
        let file_name = matches.value_of_os("file-name")
            .or_else(|| std::path::Path::new(file_path).file_name())
            .ok_or_else(|| anyhow::format_err!("You can't send a file without a name. Maybe try --rename"))?;

        let mp = MultiProgress::new();
        send_many(
            relay_server,
            &welcome.code,
            file_path,
            file_name,
            timeout,
            connector,
            &mp,
        )
        .await?;
        async_std::task::spawn_blocking(move || mp.join()).await?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let (welcome, connector, relay_server) = parse_and_connect(&matches, false).await?;
        print_welcome(&mut term, &welcome)?;

        let w = connector.connect_to_client().await?;
        writeln!(&term, "Successfully connected to peer.")?;

        let file_path = matches.value_of_os("file-path").unwrap();

        receive(
            w,
            &relay_server,
            &file_path,
            matches.value_of_os("file-name"),
        )
        .await?;
    } else {
        let _code = matches.subcommand_name();
        // TODO implement this properly once clap 3.0 is out
        // if might_be_code(code) {
        writeln!(
            &term,
            "No command provided, assuming you simply want to receive a file."
        )?;
        writeln!(&term, "To receive files, use `wormhole receive <CODE>`.")?;
        writeln!(
            &term,
            "To list all available commands and options, type `wormhole --help`"
        )?;
        writeln!(
            &term,
            "Please refer to `{} --help` for further usage instructions.",
            crate_name!()
        )?;
        // } else {
        // clap.print_long_help();
        // }
        unimplemented!();
    }

    Ok(())
}

/**
 * Parse the necessary command line arguments to establish an initial server connection.
 * This is used over and over again by the different subcommands.
 *
 * If this `is_send` and the code is not specified via the CLI, then a code will be allocated.
 * Otherwise, the user will be prompted interactively to enter it.
 */
async fn parse_and_connect(
    matches: &clap::ArgMatches<'_>,
    is_send: bool,
) -> anyhow::Result<(WormholeWelcome, WormholeConnector, RelayUrl)> {
    let relay_server: RelayUrl = matches
        .value_of("relay-server")
        .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER)
        .parse()
        .unwrap();
    let rendezvous_server = matches
        .value_of("rendezvous-server")
        .unwrap_or(magic_wormhole::DEFAULT_MAILBOX_SERVER);
    let code = matches
        .value_of("code")
        .map(ToOwned::to_owned)
        .map(CodeProvider::SetCode)
        .unwrap_or_else(|| {
            if is_send {
                let numwords = matches
                    .value_of("code-length")
                    .unwrap()
                    .parse()
                    .expect("TODO error handling");
                CodeProvider::AllocateCode(numwords)
            } else {
                CodeProvider::SetCode(enter_code().expect("TODO handle this gracefully"))
            }
        });
    let (welcome, connector) = magic_wormhole::connect_to_server(
        magic_wormhole::transfer::APPID,
        magic_wormhole::transfer::AppVersion::default(),
        rendezvous_server,
        code,
    )
    .await?;
    anyhow::Result::<_>::Ok((welcome, connector, relay_server))
}

fn _might_be_code(_code: Option<&str>) -> bool {
    // let re = Regex::new(r"\d+-\w+-\w+").unwrap();
    // if !re.is_match(&line) {
    //     panic!("Not a valid code format");
    // }
    unimplemented!()
}

fn enter_code() -> anyhow::Result<String> {
    use dialoguer::Input;

    Input::new()
        .with_prompt("Enter code")
        .interact_text()
        .map_err(From::from)
}

fn print_welcome(term: &mut Term, welcome: &magic_wormhole::WormholeWelcome) -> anyhow::Result<()> {
    writeln!(term, "Got welcome from server: {}", &welcome.welcome)?;
    Ok(())
}

fn sender_print_code(term: &mut Term, code: &magic_wormhole::Code) -> anyhow::Result<()> {
    writeln!(term, "This wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole receive {}\n", &code)?;
    Ok(())
}

async fn send(
    mut wormhole: Wormhole,
    relay_server: &RelayUrl,
    file: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
) -> anyhow::Result<()> {
    use async_std::fs::File;

    let mut file = File::open(file)
        .await
        .context(format!("Could not open {:?}", file))?;
    let file_size = file.metadata().await?.len();

    let pb = ProgressBar::new(0);
    // pb.format("╢█▌░╟");
    // pb.set_units(Units::Bytes);
    pb.set_length(file_size);

    transfer::send_file(
        &mut wormhole,
        &relay_server,
        &mut file,
        &std::path::Path::new(file_name),
        file_size,
        move |sent, _total| {
            pb.set_position(sent);
        },
    )
    .await?;
    Ok(())
}

async fn send_many(
    relay_server: RelayUrl,
    code: &str,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    timeout: Duration,
    connector: WormholeConnector,
    mp: &MultiProgress,
) -> anyhow::Result<()> {
    let file_name = Arc::new(file_name.to_owned());
    let url = Arc::new(relay_server);

    let time = Instant::now();

    use async_std::fs::File;
    let file = File::open(file_path)
        .await
        .context(format!("Could not open {:?}", file_path))?;
    let file_size = file.metadata().await?.len();

    /* Special-case the first send with reusing the existing connection */
    send_in_background(
        Arc::clone(&url),
        file,
        Arc::clone(&file_name),
        file_size,
        connector,
        &mp,
    )
    .await?;

    while time.elapsed() < timeout {
        let file = File::open(file_path)
            .await
            .context(format!("Could not open {:?}", file_path))?;
        let (_welcome, connector) = magic_wormhole::connect_to_server(
            magic_wormhole::transfer::APPID,
            magic_wormhole::transfer::AppVersion::default(),
            magic_wormhole::DEFAULT_MAILBOX_SERVER,
            CodeProvider::SetCode(code.to_owned()),
        )
        .await?;
        send_in_background(
            Arc::clone(&url),
            file,
            Arc::clone(&file_name),
            file_size,
            connector,
            &mp,
        )
        .await?;
    }

    use futures::AsyncRead;
    async fn send_in_background(
        url: Arc<RelayUrl>,
        mut file: impl AsyncRead + Unpin + Send + 'static,
        file_name: Arc<std::ffi::OsString>,
        file_size: u64,
        connector: WormholeConnector,
        mp: &MultiProgress,
    ) -> anyhow::Result<()> {
        let mut wormhole = connector.connect_to_client().await?;
        let pb = ProgressBar::new(file_size);
        let pb = mp.add(pb);
        async_std::task::spawn(async move {
            pb.enable_steady_tick(1000);

            let pb2 = pb.clone();
            let result = transfer::send_file(
                &mut wormhole,
                &url,
                &mut file,
                file_name.deref(),
                file_size,
                move |sent, _total| {
                    // @TODO: Not sure what kind of experience is best here.
                    pb2.set_position(sent);
                },
            )
            .await;
            match result {
                Ok(_) => {
                    pb.finish();
                    // info!("TODO success message") TODO
                },
                Err(e) => {
                    pb.abandon();
                    // warn!("Send failed, {}", e) TODO
                },
            };
        });
        Ok(())
    }

    Ok(())
}

async fn receive(
    mut w: Wormhole,
    relay_server: &RelayUrl,
    target_dir: &std::ffi::OsStr,
    file_name: Option<&std::ffi::OsStr>,
) -> anyhow::Result<()> {
    let req = transfer::request_file(&mut w, &relay_server).await?;

    /*
     * Control flow is a bit tricky here:
     * - First of all, we ask if we want to receive the file at all
     * - Then, we check if the file already exists
     * - If it exists, ask whether to overwrite and act accordingly
     * - If it doesn't, directly accept, but DON'T overwrite any files
     */

    if !util::ask_user(
        format!(
            "Receive file '{}' (size: {} bytes)?",
            req.filename.display(),
            req.filesize
        ),
        false,
    )
    .await
    {
        return req.reject().await;
    }

    let file_name = file_name.unwrap_or_else(|| req.filename.as_ref());
    let file_path = std::path::Path::new(target_dir).join(file_name);

    let pb = ProgressBar::new(req.filesize);
    pb.enable_steady_tick(1000);
    //pb.format("╢█▌░╟");
    // pb.set_units(Units::Bytes);

    let on_progress = move |received, _total| {
        pb.set_position(received);
    };

    /* Then, accept if the file exists */
    if !file_path.exists() {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .await?;
        return req.accept(on_progress, &mut file).await;
    }

    /* If there is a collision, ask whether to overwrite */
    if !util::ask_user(
        format!("Override existing file {}?", file_path.display()),
        false,
    )
    .await
    {
        return req.reject().await;
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .await?;
    req.accept(on_progress, &mut file).await
}
