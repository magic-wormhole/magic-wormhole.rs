mod util;

use std::{
    ops::Deref,
    time::{Duration, Instant},
};

use async_std::{fs::OpenOptions, sync::Arc};
use clap::{crate_description, crate_name, crate_version, App, AppSettings, Arg, SubCommand};
use color_eyre::{eyre, eyre::Context};
use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar};
use std::io::Write;

use magic_wormhole::{forwarding, transfer, transit, Wormhole};
use std::str::FromStr;

#[async_std::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    /* Define some common arguments first */

    let relay_server_arg = Arg::with_name("relay-server")
        .long("relay-server")
        .visible_alias("relay")
        .takes_value(true)
        .multiple(true)
        .value_name("tcp://HOSTNAME:PORT")
        .help("Use a custom relay server (specify multiple times for multiple relays)");
    let rendezvous_server_arg = Arg::with_name("rendezvous-server")
        .long("rendezvous-server")
        .takes_value(true)
        .value_name("ws://example.org")
        .help("Use a custom rendezvous server. Both sides need to use the same value in order to find each other.");
    let log_arg = Arg::with_name("log")
        .long("log")
        .global(true)
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
    let code_send = Arg::with_name("code")
        .long("code")
        .takes_value(true)
        .value_name("CODE")
        .help("Enter a code instead of generating one automatically");
    /* Use in receive commands */
    let code = Arg::with_name("code")
        .index(1)
        .value_name("CODE")
        .help("Provide the code now rather than typing it interactively");
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
        .arg(code_send.clone())
        .arg(relay_server_arg.clone())
        .arg(rendezvous_server_arg.clone())
        .arg(file_name.clone())
        .arg(
            Arg::with_name("file")
                .index(1)
                .required(true)
                .value_name("FILENAME|DIRNAME")
                .help("The file or directory to send"),
        )
        .help_message("Print this help message");
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
        .arg(code_length_arg.clone().default_value("4"))
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
        )
        .help_message("Print this help message");
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
        .arg(code.clone())
        .arg(relay_server_arg)
        .arg(rendezvous_server_arg)
        .help_message("Print this help message");
    let forward_command = SubCommand::with_name("forward")
        .about("Forward ports from one machine to another")
        .setting(AppSettings::ArgRequiredElseHelp)
        .subcommand(SubCommand::with_name("serve")
            .visible_alias("open")
            .alias("server") /* Muscle memory <3 */
            .about("Make the following ports of your system available to your peer")
            .arg(
                Arg::with_name("targets")
                    .index(1)
                    .multiple(true)
                    .required(true)
                    .value_name("[DOMAIN:]PORT")
                    .help("List of ports to open up. You can optionally specify a domain/address to forward remote ports")
            )
            .arg(code_length_arg)
            .arg(code_send)
        )
        .subcommand(SubCommand::with_name("connect")
            .about("Connect to some ports forwarded to you")
            .arg(code)
            .arg(
                Arg::with_name("port")
                    .long("port")
                    .short("p")
                    .takes_value(true)
                    .multiple(true)
                    .value_name("PORT")
                    .help("Bind to specific ports instead of taking random free high ports. Can be provided multiple times.")
            )
            .arg(
                Arg::with_name("bind")
                    .long("bind")
                    .takes_value(true)
                    .value_name("ADDRESS")
                    .default_value("::")
                    .help("Bind to a specific address to accept the forwarding. Depending on your system and firewall, this may make the forwarded ports accessible from the outside.")
            )
            .arg(
                Arg::with_name("noconfirm")
                    .long("noconfirm")
                    .visible_alias("yes")
                    .help("Accept the forwarding without asking for confirmation"),
            )
        )
        .subcommand(SubCommand::with_name("help").setting(AppSettings::Hidden))
        .help_message("Print this help message");

    /* The Clap application */
    let clap = App::new(crate_name!())
        .version(crate_version!())
        .about(crate_description!())
        .setting(AppSettings::ArgRequiredElseHelp)
        .global_setting(AppSettings::DisableHelpSubcommand)
        .global_setting(AppSettings::VersionlessSubcommands)
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
        .subcommand(forward_command)
        .subcommand(SubCommand::with_name("help").setting(AppSettings::Hidden))
        .arg(log_arg)
        .help_message("Print this help message");
    let matches = clap.get_matches();

    let mut term = Term::stdout();

    if matches.is_present("log") {
        env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .filter_module("magic_wormhole::core", log::LevelFilter::Trace)
            .filter_module("mio", log::LevelFilter::Debug)
            .filter_module("ws", log::LevelFilter::Error)
            .try_init()?;
        log::debug!("Logging enabled.");
    } else {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .filter_module("ws", log::LevelFilter::Error)
            .format_timestamp(None)
            .format_target(false)
            .try_init()?;
    }

    let file_name = |file_path| {
        // TODO this has gotten out of hand (it ugly)
        // The correct solution would be to make `file_name` an Option everywhere and
        // move the ".tar" part further down the line.
        // The correct correct solution would be to have working file transfer instead
        // of sending stupid archives.
        matches
            .value_of_os("file-name")
            .map(std::ffi::OsString::from)
            .or_else(|| {
                let path = std::path::Path::new(file_path);
                let mut name = path.file_name().map(std::ffi::OsString::from);
                if path.is_dir() {
                    name = name.map(|mut name| {
                        name.push(".tar");
                        name
                    });
                }
                name
            })
            .ok_or_else(|| {
                eyre::format_err!("You can't send a file without a name. Maybe try --rename")
            })
    };

    /* Handling of the argument matches (one branch per subcommand) */

    if let Some(matches) = matches.subcommand_matches("send") {
        let file_path = matches.value_of_os("file").unwrap();
        let file_name = file_name(file_path)?;

        eyre::ensure!(
            std::path::Path::new(file_path).exists(),
            "{:?} does not exist",
            file_path
        );

        let (wormhole, _code, relay_server) =
            parse_and_connect(&mut term, matches, true, transfer::APP_CONFIG).await?;

        send(wormhole, relay_server, file_path, &file_name).await?;
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let (wormhole, code, relay_server) =
            parse_and_connect(&mut term, matches, true, transfer::APP_CONFIG).await?;
        let timeout =
            Duration::from_secs(u64::from_str(matches.value_of("timeout").unwrap())? * 60);
        let max_tries = u64::from_str(matches.value_of("tries").unwrap())?;

        let file_path = matches.value_of_os("file").unwrap();
        let file_name = file_name(file_path)?;

        send_many(
            relay_server,
            &code,
            file_path,
            &file_name,
            max_tries,
            timeout,
            wormhole,
            &mut term,
        )
        .await?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let file_path = matches.value_of_os("file-path").unwrap();

        let (wormhole, _code, relay_server) =
            parse_and_connect(&mut term, matches, false, transfer::APP_CONFIG).await?;

        receive(
            wormhole,
            relay_server,
            file_path,
            matches.value_of_os("file-name"),
        )
        .await?;
    } else if let Some(matches) = matches.subcommand_matches("forward") {
        // TODO make fancy
        log::warn!("This is an unstable feature. Make sure that your peer is running the exact same version of the program as you.");
        if let Some(matches) = matches.subcommand_matches("serve") {
            /* Map the CLI argument to Strings. Use the occasion to inspect them and fail early on malformed input. */
            let targets = matches
                .values_of("targets")
                .unwrap()
                .enumerate()
                .map(|(index, target)| {
                    let result = (|| {
                        /* Either HOST:PORT or PORT */
                        if target.contains(':') {
                            /* Extract the :PORT at the end */
                            let port = target.split(':').last().unwrap();
                            let host = url::Host::parse(&target[..target.len() - port.len() - 1])
                                .map_err(eyre::Error::from)
                                .context("Invalid host")?;
                            let port: u16 = port.parse().context("Invalid port")?;
                            Ok((Some(host), port))
                        } else {
                            /* It's just a port */
                            target
                                .parse::<u16>()
                                .map(|port| (None, port))
                                .map_err(eyre::Error::from)
                                .context("Invalid port")
                        }
                    })();
                    result.context(format!(
                        "Invalid {}{} target argument ('{}') ",
                        index + 1,
                        match (index + 1) % 10 {
                            1 => "st",
                            2 => "nd",
                            3 => "rd",
                            _ => "th",
                        },
                        target
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            loop {
                let (wormhole, _code, relay_server) =
                    parse_and_connect(&mut term, matches, true, forwarding::APP_CONFIG).await?;
                let relay_server = vec![transit::RelayHint::from_url(relay_server)];
                async_std::task::spawn(forwarding::serve(wormhole, relay_server, targets.clone()));
            }
        } else if let Some(matches) = matches.subcommand_matches("connect") {
            let custom_ports: Vec<u16> = matches
                .values_of("port")
                .into_iter()
                .flatten()
                .map(|port| port.parse().map_err(eyre::Error::from))
                .collect::<Result<_, _>>()?;
            let bind_address: std::net::IpAddr = matches.value_of("bind").unwrap().parse()?;
            let (wormhole, _code, relay_server) =
                parse_and_connect(&mut term, matches, false, forwarding::APP_CONFIG).await?;
            let relay_server = vec![transit::RelayHint::from_url(relay_server)];

            forwarding::connect(wormhole, relay_server, Some(bind_address), &custom_ports).await?;
        } else {
            unreachable!()
        }
    } else if let Some(_matches) = matches.subcommand_matches("help") {
        println!("Use --help to get help");
        std::process::exit(1);
    } else {
        unreachable!()
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
    term: &mut Term,
    matches: &clap::ArgMatches<'_>,
    is_send: bool,
    mut app_config: magic_wormhole::AppConfig<impl serde::Serialize>,
) -> eyre::Result<(Wormhole, magic_wormhole::Code, url::Url)> {
    let relay_server: url::Url = matches
        .value_of("relay-server")
        .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER)
        .parse()
        .unwrap();
    let rendezvous_server = matches.value_of("rendezvous-server");
    let code = matches
        .value_of("code")
        .map(ToOwned::to_owned)
        .or_else(|| (!is_send).then(|| enter_code().expect("TODO handle this gracefully")))
        .map(magic_wormhole::Code);

    if let Some(rendezvous_server) = rendezvous_server {
        app_config = app_config.rendezvous_url(rendezvous_server.to_owned().into());
    }
    let (wormhole, code) = match code {
        Some(code) => {
            if is_send {
                sender_print_code(term, &code)?;
            }
            let (server_welcome, wormhole) =
                magic_wormhole::Wormhole::connect_with_code(app_config, code).await?;
            print_welcome(term, &server_welcome)?;
            (wormhole, server_welcome.code)
        },
        None => {
            let numwords = matches
                .value_of("code-length")
                .unwrap()
                .parse()
                .expect("TODO error handling");

            let (server_welcome, connector) =
                magic_wormhole::Wormhole::connect_without_code(app_config, numwords).await?;
            print_welcome(term, &server_welcome)?;
            if is_send {
                sender_print_code(term, &server_welcome.code)?;
            }
            let wormhole = connector.await?;
            (wormhole, server_welcome.code)
        },
    };
    eyre::Result::<_>::Ok((wormhole, code, relay_server))
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

fn enter_code() -> eyre::Result<String> {
    use dialoguer::Input;

    Input::new()
        .with_prompt("Enter code")
        .interact_text()
        .map_err(From::from)
}

fn print_welcome(term: &mut Term, welcome: &magic_wormhole::WormholeWelcome) -> eyre::Result<()> {
    if let Some(welcome) = &welcome.welcome {
        writeln!(term, "Got welcome from server: {}", welcome)?;
    }
    Ok(())
}

fn sender_print_code(term: &mut Term, code: &magic_wormhole::Code) -> eyre::Result<()> {
    writeln!(term, "\nThis wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole receive {}\n", &code)?;
    Ok(())
}

async fn send(
    wormhole: Wormhole,
    relay_server: url::Url,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
) -> eyre::Result<()> {
    let pb = create_progress_bar(0);
    let pb2 = pb.clone();
    transfer::send_file_or_folder(
        wormhole,
        relay_server,
        file_path,
        file_name,
        move |sent, total| {
            if sent == 0 {
                pb.reset_elapsed();
                pb.set_length(total);
                pb.enable_steady_tick(250);
            }
            pb.set_position(sent);
        },
    )
    .await
    .context("Send process failed")?;
    pb2.finish();
    Ok(())
}

async fn send_many(
    relay_server: url::Url,
    code: &magic_wormhole::Code,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    max_tries: u64,
    timeout: Duration,
    wormhole: Wormhole,
    term: &mut Term,
) -> eyre::Result<()> {
    /* Progress bar is commented out for now. See the issues about threading/async in
     * the Indicatif repository for more information. Multiple progress bars are not usable
     * for us at the moment, so we'll have to do without for now.
     */
    // let mp = MultiProgress::new();
    // async_std::task::spawn_blocking(move || mp.join()).await?;

    let file_path = Arc::new(file_path.to_owned());
    let file_name = Arc::new(file_name.to_owned());
    // TODO go back to reference counting again
    //let url = Arc::new(relay_server);

    let time = Instant::now();

    /* Special-case the first send with reusing the existing connection */
    send_in_background(
        relay_server.clone(),
        Arc::clone(&file_path),
        Arc::clone(&file_name),
        wormhole,
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

        let (_server_welcome, wormhole) =
            magic_wormhole::Wormhole::connect_with_code(transfer::APP_CONFIG, code.clone()).await?;
        send_in_background(
            relay_server.clone(),
            Arc::clone(&file_path),
            Arc::clone(&file_name),
            wormhole,
            term.clone(),
            // &mp,
        )
        .await?;
    }

    async fn send_in_background(
        url: url::Url,
        file_name: Arc<std::ffi::OsString>,
        file_path: Arc<std::ffi::OsString>,
        wormhole: Wormhole,
        mut term: Term,
        // mp: &MultiProgress,
    ) -> eyre::Result<()> {
        writeln!(&mut term, "Sending file to peer").unwrap();
        // let pb = create_progress_bar(file_size);
        // let pb = mp.add(pb);
        async_std::task::spawn(async move {
            // let pb2 = pb.clone();
            let result = async move {
                transfer::send_file_or_folder(
                    wormhole,
                    url,
                    file_path.deref(),
                    file_name.deref(),
                    move |_sent, _total| {
                        // if sent == 0 {
                        //     pb2.reset_elapsed();
                        //     pb2.enable_steady_tick(250);
                        // }
                        // pb2.set_position(sent);
                    },
                )
                .await?;
                eyre::Result::<_>::Ok(())
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
    wormhole: Wormhole,
    relay_server: url::Url,
    target_dir: &std::ffi::OsStr,
    file_name: Option<&std::ffi::OsStr>,
) -> eyre::Result<()> {
    let req = transfer::request_file(wormhole, relay_server)
        .await
        .context("Could get an offer")?;

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
        true,
    )
    .await
    {
        return req.reject().await.context("Could not reject offer");
    }

    let file_name = file_name
        .or_else(|| req.filename.file_name())
        .ok_or_else(|| eyre::format_err!("The sender did not specify a valid file name, and neither did you. Try using --rename."))?;
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
            .await
            .context("Failed to create destination file")?;
        return req
            .accept(on_progress, &mut file)
            .await
            .context("Receive process failed");
    }

    /* If there is a collision, ask whether to overwrite */
    if !util::ask_user(
        format!("Override existing file {}?", file_path.display()),
        false,
    )
    .await
    {
        return req.reject().await.context("Could not reject offer");
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .await?;
    Ok(req
        .accept(on_progress, &mut file)
        .await
        .context("Receive process failed")?)
}
