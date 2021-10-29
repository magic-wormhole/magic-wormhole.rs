mod util;

use std::{
    ops::Deref,
    str,
    time::{Duration, Instant},
};

use async_std::{fs::OpenOptions, sync::Arc};
use clap::{crate_description, crate_name, crate_version, App, AppSettings, Arg, SubCommand};
use color_eyre::eyre;
use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar};
use std::io::Write;

use magic_wormhole::{transfer, transit::RelayUrl, Wormhole};
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
    let shell_command = Arg::with_name("shell-command")
        .long("handle-code")
        .conflicts_with("code")
        .takes_value(true)
        .value_name("COMMAND")
        .help("Execute a shell command and pass the codeword via stdin.");
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
        .arg(shell_command.clone())
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
        .arg(shell_command)
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
        .subcommand(SubCommand::with_name("help").setting(AppSettings::Hidden))
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

        let (mut wormhole, _code, relay_server) =
            parse_and_connect(&mut term, matches, true).await?;

        send(&mut wormhole, &relay_server, file_path, &file_name).await?;
        wormhole.close().await?;
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let (wormhole, code, relay_server) = parse_and_connect(&mut term, matches, true).await?;
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

        let (mut wormhole, _code, relay_server) =
            parse_and_connect(&mut term, matches, false).await?;

        receive(
            &mut wormhole,
            &relay_server,
            file_path,
            matches.value_of_os("file-name"),
        )
        .await?;
        wormhole.close().await?;
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
) -> eyre::Result<(Wormhole, magic_wormhole::Code, RelayUrl)> {
    let relay_server: RelayUrl = matches
        .value_of("relay-server")
        .unwrap_or(magic_wormhole::transit::DEFAULT_RELAY_SERVER)
        .parse()
        .unwrap();
    let rendezvous_server = matches
        .value_of("rendezvous-server")
        .unwrap_or(magic_wormhole::rendezvous::DEFAULT_RENDEZVOUS_SERVER)
        .to_string();
    let code = matches
        .value_of("code")
        .map(ToOwned::to_owned)
        .or_else(|| (!is_send).then(|| enter_code().expect("TODO handle this gracefully")))
        .map(magic_wormhole::Code);
    let (wormhole, code) = match code {
        Some(code) => {
            if is_send {
                sender_print_code(term, &code)?;
            }
            let (server_welcome, wormhole) = magic_wormhole::Wormhole::connect_with_code(
                transfer::APP_CONFIG.rendezvous_url(rendezvous_server.into()),
                code,
            )
            .await?;
            print_welcome(term, &server_welcome)?;
            (wormhole, server_welcome.code)
        },
        None => {
            let numwords = matches
                .value_of("code-length")
                .unwrap()
                .parse()
                .expect("TODO error handling");

            let (server_welcome, connector) = magic_wormhole::Wormhole::connect_without_code(
                transfer::APP_CONFIG.rendezvous_url(rendezvous_server.into()),
                numwords,
            )
            .await?;
            print_welcome(term, &server_welcome)?;
            if is_send {
                execute_shell_command(term, matches, &server_welcome.code)?;
                sender_print_code(term, &server_welcome.code)?;
            }
            let wormhole = connector.await?;
            (wormhole, server_welcome.code)
        },
    };
    writeln!(term, "Successfully connected to peer.")?;
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
    writeln!(term, "This wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole receive {}\n", &code)?;
    Ok(())
}

fn execute_shell_command(
    term: &mut Term,
    matches: &clap::ArgMatches<'_>,
    code: &magic_wormhole::Code
) -> eyre::Result<()> {
    use eyre::{ WrapErr, ContextCompat };
    if let Some(shell_command) = matches.value_of_os("shell-command") {
        #[cfg(target_os = "windows")]
        let mut child = std::process::Command::new("cmd")
            .arg("/C")
            .arg(shell_command)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("failed execute shell-command")?;
        #[cfg(not(target_os = "windows"))]
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg(shell_command)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("failed execute shell-command")?;
        let mut stdin = child.stdin.take().context("failed to open stdin of shell-command")?;
        stdin.write_all(code.as_bytes()).context("failed to write to stdin of shell-command")?;
        drop(stdin);
        let status = child.wait().context("failed to wait for shell-command exit")?;
        if !status.success() {
            let rc = status.code().context("failed to get rc")?;
            return Err(eyre::eyre!("Codeword handling command exited with RC {}", rc));
        } else {
            writeln!(term, "Codeword piped to command.")?;
        }
    }
    Ok(())
}

async fn send(
    wormhole: &mut Wormhole,
    relay_server: &RelayUrl,
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
    .await?;
    pb2.finish();
    Ok(())
}

async fn send_many(
    relay_server: RelayUrl,
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
    let url = Arc::new(relay_server);

    let time = Instant::now();

    /* Special-case the first send with reusing the existing connection */
    send_in_background(
        Arc::clone(&url),
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
            Arc::clone(&url),
            Arc::clone(&file_path),
            Arc::clone(&file_name),
            wormhole,
            term.clone(),
            // &mp,
        )
        .await?;
    }

    async fn send_in_background(
        url: Arc<RelayUrl>,
        file_name: Arc<std::ffi::OsString>,
        file_path: Arc<std::ffi::OsString>,
        mut wormhole: Wormhole,
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
                    &mut wormhole,
                    &url,
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
                eyre::Result::<_>::Ok(wormhole.close().await?)
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
) -> eyre::Result<()> {
    let req = transfer::request_file(wormhole, relay_server).await?;

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
        return Ok(req.reject().await?);
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
            .await?;
        return Ok(req.accept(on_progress, &mut file).await?);
    }

    /* If there is a collision, ask whether to overwrite */
    if !util::ask_user(
        format!("Override existing file {}?", file_path.display()),
        false,
    )
    .await
    {
        return Ok(req.reject().await?);
    }

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&file_path)
        .await?;
    Ok(req.accept(on_progress, &mut file).await?)
}
