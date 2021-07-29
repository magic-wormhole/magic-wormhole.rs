use std::{
    ops::Deref,
    str,
    time::{Duration, Instant},
};

use anyhow::Context;
use async_std::{fs::OpenOptions, sync::Arc};
use clap::{crate_description, crate_name, crate_version, App, AppSettings, Arg, SubCommand};
use color_eyre::eyre;
use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar};
use std::io::Write;

use magic_wormhole::{
    transfer, transit::RelayUrl, util, CodeProvider, Wormhole, WormholeConnector, WormholeWelcome,
};
use std::str::FromStr;

#[derive(thiserror::Error, Debug)]
#[error(transparent)]
pub struct AsStdError(#[from] anyhow::Error);

#[async_std::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

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
            to guess the code, whereas normally they have only one. This can be \
            countered by using a longer than usual code (default 4 bytes entropy).\n\n\
            The application terminates on interruption, after a timeout or after a
            number of sent files, whichever comes first. It will always try to send
            at least one file, regardless of the limits.",
        )
        .arg(code_length_arg.default_value("4"))
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
            Arg::with_name("tries")
                .long("tries")
                .short("n")
                .takes_value(true)
                .value_name("N")
                .default_value("30")
                .help("Only send the file up to n times, limiting the number of people that may receive it. \
                       These are also the number of tries a potential attacker gets at guessing the password."),
        )
        .arg(
            Arg::with_name("timeout")
                .long("timeout")
                .takes_value(true)
                .value_name("MINUTES")
                .default_value("60")
                .help("Automatically stop providing the file after a certain amount of time."),
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
            .filter_module("magic_wormhole::core", log::LevelFilter::Trace)
            .filter_module("mio", log::LevelFilter::Debug)
            .filter_module("ws", log::LevelFilter::Error)
            .init();
        log::debug!("Logging enabled.");
    }

    /* Handling of the argument matches (one branch per subcommand) */

    if let Some(matches) = matches.subcommand_matches("send") {
        let file_path = matches.value_of_os("file").unwrap();
        let file_name = matches
            .value_of_os("file-name")
            .or_else(|| std::path::Path::new(file_path).file_name())
            .ok_or_else(|| {
                eyre::format_err!("You can't send a file without a name. Maybe try --rename")
            })?;

        eyre::ensure!(
            std::path::Path::new(file_path).exists(),
            "{:?} does not exist",
            file_path
        );

        let (welcome, connector, relay_server): (WormholeWelcome, WormholeConnector, RelayUrl) =
            parse_and_connect(&matches, true)
                .await
                .map_err(AsStdError::from)?;
        print_welcome(&mut term, &welcome).map_err(AsStdError::from)?;
        sender_print_code(&mut term, &welcome.code).map_err(AsStdError::from)?;
        let mut wormhole: Wormhole = connector
            .connect_to_client()
            .await
            .map_err(AsStdError::from)?;
        writeln!(&term, "Successfully connected to peer.")?;

        send(&mut wormhole, &relay_server, &file_path, &file_name)
            .await
            .map_err(AsStdError::from)?;
        wormhole.close().await.map_err(AsStdError::from)?;
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let (welcome, connector, relay_server) = parse_and_connect(&matches, true)
            .await
            .map_err(AsStdError::from)?;
        let timeout =
            Duration::from_secs(u64::from_str(matches.value_of("timeout").unwrap())? * 60);
        let max_tries = u64::from_str(matches.value_of("tries").unwrap())?;

        print_welcome(&mut term, &welcome).map_err(AsStdError::from)?;
        sender_print_code(&mut term, &welcome.code).map_err(AsStdError::from)?;

        let file_path = matches.value_of_os("file").unwrap();
        let file_name = matches
            .value_of_os("file-name")
            .or_else(|| std::path::Path::new(file_path).file_name())
            .ok_or_else(|| {
                anyhow::format_err!("You can't send a file without a name. Maybe try --rename")
            })
            .map_err(AsStdError::from)?;

        send_many(
            relay_server,
            &welcome.code,
            file_path,
            file_name,
            max_tries,
            timeout,
            connector,
            &mut term,
        )
        .await
        .map_err(AsStdError::from)?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let (welcome, connector, relay_server): (_, WormholeConnector, _) =
            parse_and_connect(&matches, false)
                .await
                .map_err(AsStdError::from)?;
        print_welcome(&mut term, &welcome).map_err(AsStdError::from)?;

        let mut wormhole = connector
            .connect_to_client()
            .await
            .map_err(AsStdError::from)?;
        writeln!(&term, "Successfully connected to peer.")?;

        let file_path = matches.value_of_os("file-path").unwrap();

        receive(
            &mut wormhole,
            &relay_server,
            &file_path,
            matches.value_of_os("file-name"),
        )
        .await
        .map_err(AsStdError::from)?;
        wormhole.close().await.map_err(AsStdError::from)?;
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

fn create_progress_bar(file_size: u64) -> ProgressBar {
    use indicatif::ProgressStyle;

    let pb = ProgressBar::new(file_size);
    pb.set_style(
        ProgressStyle::default_bar()
            // .template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .template("[{elapsed_precise}] [{wide_bar}] {bytes}/{total_bytes} ({eta})")
            .progress_chars("#>-"),
    );
    pb
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
    wormhole: &mut Wormhole,
    relay_server: &RelayUrl,
    file: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
) -> anyhow::Result<()> {
    use async_std::fs::File;

    let mut file: async_std::fs::File = File::open(file)
        .await
        .context(format!("Could not open {:?}", file))?;
    let file_size = file.metadata().await?.len();

    let pb = create_progress_bar(file_size);
    let pb2 = pb.clone();
    transfer::send_file(
        wormhole,
        &relay_server,
        &mut file,
        &std::path::Path::new(file_name),
        file_size,
        move |sent, _total| {
            if sent == 0 {
                pb.reset_elapsed();
                pb.enable_steady_tick(250);
            }
            pb.set_position(sent);
        },
    )
    .await?;
    pb2.finish();
    Ok(())
}

async fn send_many(
    relay_server: RelayUrl,
    code: &str,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    max_tries: u64,
    timeout: Duration,
    connector: WormholeConnector,
    term: &mut Term,
) -> anyhow::Result<()> {
    /* Progress bar is commented out for now. See the issues about threading/async in
     * the Indicatif repository for more information. Multiple progress bars are not usable
     * for us at the moment, so we'll have to do without for now.
     */
    // let mp = MultiProgress::new();
    // async_std::task::spawn_blocking(move || mp.join()).await?;

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
        term.clone(),
        // &mp,
    )
    .await?;

    for tries in 0.. {
        if time.elapsed() >= timeout {
            writeln!(
                term,
                "{:?} have elapsed, we won't accept any new connections now.",
                timeout
            )?;
            break;
        }
        if tries > max_tries {
            writeln!(
                term,
                "Max number of tries reached, we won't accept any new connections now."
            )?;
            break;
        }

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
            term.clone(),
            // &mp,
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
        mut term: Term,
        // mp: &MultiProgress,
    ) -> anyhow::Result<()> {
        let mut wormhole = connector.connect_to_client().await?;
        writeln!(&mut term, "Sending file to peer").unwrap();
        // let pb = create_progress_bar(file_size);
        // let pb = mp.add(pb);
        async_std::task::spawn(async move {
            // let pb2 = pb.clone();
            let result = async move {
                transfer::send_file(
                    &mut wormhole,
                    &url,
                    &mut file,
                    file_name.deref(),
                    file_size,
                    move |_sent, _total| {
                        // if sent == 0 {
                        //     pb2.reset_elapsed();
                        //     pb2.enable_steady_tick(250);
                        // }
                        // pb2.set_position(sent);
                    },
                )
                .await?;
                wormhole.close().await
            };
            match result.await {
                Ok(_) => {
                    // pb.finish();
                    writeln!(&mut term, "Successfully sent file to peer").unwrap();
                },
                Err(e) => {
                    // pb.abandon();
                    writeln!(&mut term, "Send failed, {}", e).unwrap();
                },
            };
        });
        Ok(())
    }

    Ok(())
}

async fn receive(
    wormhole: &mut Wormhole,
    relay_server: &RelayUrl,
    target_dir: &std::ffi::OsStr,
    file_name: Option<&std::ffi::OsStr>,
) -> anyhow::Result<()> {
    let req = transfer::request_file(wormhole, &relay_server).await?;

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

    let file_name = file_name
        .or_else(|| req.filename.file_name())
        .ok_or_else(|| anyhow::format_err!("The sender did not specify a valid file name, and neither did you. Try using --rename."))?;
    let file_path = std::path::Path::new(target_dir).join(file_name);

    let pb = create_progress_bar(req.filesize);

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
