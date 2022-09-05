#![allow(clippy::too_many_arguments)]
mod util;

use std::{
    ops::Deref,
    time::{Duration, Instant},
};

use async_std::{fs::OpenOptions, sync::Arc};
use clap::{crate_description, crate_name, crate_version, Arg, Args, Command, Parser, Subcommand};
use cli_clipboard::{ClipboardContext, ClipboardProvider};
use color_eyre::{eyre, eyre::Context};
use console::{style, Term};
use futures::{future::Either, Future, FutureExt};
use indicatif::{MultiProgress, ProgressBar};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

use magic_wormhole::{forwarding, transfer, transit, Wormhole};
use std::str::FromStr;

fn install_ctrlc_handler(
) -> eyre::Result<impl Fn() -> futures::future::BoxFuture<'static, ()> + Clone> {
    use async_std::sync::{Condvar, Mutex};

    let notifier = Arc::new((Mutex::new(false), Condvar::new()));

    /* Register the handler */
    let notifier2 = notifier.clone();
    ctrlc::set_handler(move || {
        futures::executor::block_on(async {
            let mut has_notified = notifier2.0.lock().await;
            if *has_notified {
                /* Second signal. Exit */
                log::debug!("Exit.");
                std::process::exit(130);
            }
            /* First signal. */
            log::info!("Got Ctrl-C event. Press again to exit immediately");
            *has_notified = true;
            notifier2.1.notify_all();
        })
    })
    .context("Error setting Ctrl-C handler")?;

    Ok(move || {
        /* Transform the notification into a future that waits */
        let notifier = notifier.clone();
        async move {
            let (lock, cvar) = &*notifier;
            let mut started = lock.lock().await;
            while !*started {
                started = cvar.wait(started).await;
            }
        }
        .boxed()
    })
}

// send, send-many
#[derive(Debug, Args)]
struct CommonSenderArgs {
    /// Suggest a different name to the receiver to keep the file's actual name secret.
    #[clap(long = "rename", visible_alias = "name", value_name = "FILE_NAME")]
    file_name: Option<PathBuf>,
    #[clap(index = 1, required = true, value_name = "FILENAME|DIRNAME")]
    file: PathBuf,
}

// send, send-many, serve
#[derive(Debug, Args)]
struct CommonLeaderArgs {
    /// Enter a code instead of generating one automatically
    #[clap(long, value_name = "CODE")]
    code: Option<String>,
    /// Length of code (in bytes/words)
    #[clap(short = 'c', long, value_name = "NUMWORDS", default_value = "2")]
    code_length: usize,
}

// receive
#[derive(Debug, Args)]
struct CommonReceiverArgs {
    /// Rename the received file or folder, overriding the name suggested by the sender.
    #[clap(long = "rename", visible_alias = "name", value_name = "FILE_NAME")]
    file_name: Option<PathBuf>,
    /// Store transferred file or folder in the specified directory. Defaults to $PWD.
    #[clap(long = "out-dir", value_name = "PATH", default_value = ".")]
    file_path: PathBuf,
}

// receive, connect
#[derive(Debug, Args)]
struct CommonFollowerArgs {
    /// Provide the code now rather than typing it interactively
    #[clap(value_name = "CODE")]
    code: Option<String>,
}

// send, send-mane, receive, serve, connect
#[derive(Debug, Clone, Args)]
struct CommonArgs {
    /// Use a custom relay server (specify multiple times for multiple relays)
    #[clap(
        long = "relay-server",
        visible_alias = "relay",
        multiple_occurrences = true,
        value_name = "tcp://HOSTNAME:PORT"
    )]
    relay_server: Vec<url::Url>,
    /// Use a custom rendezvous server. Both sides need to use the same value in order to find each other.
    #[clap(long, value_name = "ws://example.org")]
    rendezvous_server: Option<url::Url>,
    /// Disable the relay server support and force a direct connection.
    #[clap(long)]
    force_direct: bool,
    /// Always route traffic over a relay server. This hides your IP address from the peer (but not from the server operators. Use Tor for that).
    #[clap(long, conflicts_with = "force-direct")]
    force_relay: bool,
}

#[derive(Debug, Subcommand)]
#[clap(arg_required_else_help = true)]
enum ForwardCommand {
    /// Make the following ports of your system available to your peer
    #[clap(
        visible_alias = "open",
        alias = "server", /* Muscle memory <3 */
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    Serve {
        /// List of ports to open up. You can optionally specify a domain/address to forward remote ports
        #[clap(value_name = "[DOMAIN:]PORT", multiple_occurrences = true)]
        targets: Vec<String>,
        #[clap(flatten)]
        common: CommonArgs,
        #[clap(flatten)]
        common_leader: CommonLeaderArgs,
    },
    /// Connect to some ports forwarded to you
    #[clap(
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    Connect {
        /// Bind to specific ports instead of taking random free high ports. Can be provided multiple times.
        #[clap(
            short = 'p',
            long = "port",
            multiple_occurrences = true,
            value_name = "PORT"
        )]
        ports: Vec<u16>,
        /// Bind to a specific address to accept the forwarding. Depending on your system and firewall, this may make the forwarded ports accessible from the outside.
        #[clap(long = "bind", value_name = "ADDRESS", default_value = "::")]
        bind_address: std::net::IpAddr,
        /// Accept the forwarding without asking for confirmation
        #[clap(long, visible_alias = "yes")]
        noconfirm: bool,
        #[clap(flatten)]
        common: CommonArgs,
        #[clap(flatten)]
        common_follower: CommonFollowerArgs,
    },
}

#[derive(Debug, Subcommand)]
enum WormholeCommand {
    /// Send a file or a folder
    #[clap(
        visible_alias = "tx",
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    Send {
        /// The file or directory to send
        #[clap(flatten)]
        common: CommonArgs,
        #[clap(flatten)]
        common_leader: CommonLeaderArgs,
        #[clap(flatten)]
        common_send: CommonSenderArgs,
    },
    /// Send a file to many recipients. READ HELP PAGE FIRST!
    #[clap(
        mut_arg("help", |a| a.help("Print this help message")),
        after_help = "This works by sending the file in a loop with the same code over \
        and over again. Note that this also gives an attacker multiple tries \
        to guess the code, whereas normally they have only one. This can be \
        countered by using a longer than usual code (default 4 bytes entropy).\n\n\
        The application terminates on interruption, after a timeout or after a
        number of sent files, whichever comes first. It will always try to send
        at least one file, regardless of the limits."
    )]
    SendMany {
        /// Only send the file up to n times, limiting the number of people that may receive it.
        /// These are also the number of tries a potential attacker gets at guessing the password.
        #[clap(short = 'n', long, value_name = "N", default_value = "30")]
        tries: u64,
        /// Automatically stop providing the file after a certain amount of time.
        #[clap(long, value_name = "MINUTES", default_value = "60")]
        timeout: u64,
        #[clap(flatten)]
        common: CommonArgs,
        #[clap(flatten)]
        common_leader: CommonLeaderArgs,
        #[clap(flatten)]
        common_send: CommonSenderArgs,
    },
    /// Receive a file or a folder
    #[clap(
        visible_alias = "rx",
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    Receive {
        /// Accept file transfer without asking for confirmation
        #[clap(long, visible_alias = "yes")]
        noconfirm: bool,
        #[clap(flatten)]
        common: CommonArgs,
        #[clap(flatten)]
        common_follower: CommonFollowerArgs,
        #[clap(flatten)]
        common_receiver: CommonReceiverArgs,
    },
    /// Forward ports from one machine to another
    #[clap(subcommand)]
    Forward(ForwardCommand),
    #[clap(hide = true)]
    Help,
}

#[derive(Debug, Parser)]
#[clap(
    version,
    author,
    about,
    arg_required_else_help = true,
    disable_help_subcommand = true,
    propagate_version = true,
    after_help = "Run a subcommand with `--help` to know how it's used.\n\
                 To send files, use `wormhole send <PATH>`.\n\
                 To receive files, use `wormhole receive <CODE>`.",
    mut_arg("help", |a| a.help("Print this help message")),
)]
struct WormholeCli {
    /// Enable logging to stdout, for debugging purposes
    #[clap(short = 'v', long = "verbose", alias = "log", global = true)]
    log: bool,
    #[clap(subcommand)]
    command: WormholeCommand,
}

#[async_std::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    let ctrl_c = install_ctrlc_handler()?;

    let app = WormholeCli::parse();

    let mut term = Term::stdout();

    if app.log {
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

    let mut clipboard = ClipboardContext::new()
        .map_err(|err| {
            log::warn!("Failed to initialize clipboard support: {}", err);
        })
        .ok();

    let concat_file_name = |file_path: &Path, file_name: Option<_>| {
        // TODO this has gotten out of hand (it ugly)
        // The correct solution would be to make `file_name` an Option everywhere and
        // move the ".tar" part further down the line.
        // The correct correct solution would be to have working file transfer instead
        // of sending stupid archives.
        file_name
            .map(std::ffi::OsString::from)
            .or_else(|| {
                let mut name = file_path.file_name().map(std::ffi::OsString::from);
                if file_path.is_dir() {
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

    match app.command {
        WormholeCommand::Send {
            common,
            common_leader: CommonLeaderArgs { code, code_length },
            common_send:
                CommonSenderArgs {
                    file_name,
                    file: file_path,
                },
            ..
        } => {
            let file_name = concat_file_name(&file_path, file_name.as_ref())?;

            eyre::ensure!(file_path.exists(), "{} does not exist", file_path.display());

            let transit_abilities = parse_transit_args(&common);
            let (wormhole, _code, relay_server) = match util::cancellable(
                parse_and_connect(
                    &mut term,
                    common,
                    code,
                    Some(code_length),
                    true,
                    transfer::APP_CONFIG,
                    Some(&sender_print_code),
                    clipboard.as_mut(),
                ),
                ctrl_c(),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => return Ok(()),
            };

            send(
                wormhole,
                relay_server,
                file_path.as_ref(),
                &file_name,
                transit_abilities,
                ctrl_c.clone(),
            )
            .await?;
        },
        WormholeCommand::SendMany {
            tries,
            timeout,
            common,
            common_leader: CommonLeaderArgs { code, code_length },
            common_send: CommonSenderArgs { file_name, file },
            ..
        } => {
            let transit_abilities = parse_transit_args(&common);
            let (wormhole, code, relay_server) = {
                let connect_fut = parse_and_connect(
                    &mut term,
                    common,
                    code,
                    Some(code_length),
                    true,
                    transfer::APP_CONFIG,
                    Some(&sender_print_code),
                    clipboard.as_mut(),
                );
                futures::pin_mut!(connect_fut);
                match futures::future::select(connect_fut, ctrl_c()).await {
                    Either::Left((result, _)) => result?,
                    Either::Right(((), _)) => return Ok(()),
                }
            };
            let timeout = Duration::from_secs(timeout * 60);

            let file_name = concat_file_name(&file, file_name.as_ref())?;

            send_many(
                relay_server,
                &code,
                file.as_ref(),
                &file_name,
                tries,
                timeout,
                wormhole,
                &mut term,
                transit_abilities,
                ctrl_c,
            )
            .await?;
        },
        WormholeCommand::Receive {
            noconfirm,
            common,
            common_follower: CommonFollowerArgs { code },
            common_receiver:
                CommonReceiverArgs {
                    file_name,
                    file_path,
                },
            ..
        } => {
            let transit_abilities = parse_transit_args(&common);
            let (wormhole, _code, relay_server) = {
                let connect_fut = parse_and_connect(
                    &mut term,
                    common,
                    code,
                    None,
                    false,
                    transfer::APP_CONFIG,
                    None,
                    clipboard.as_mut(),
                );
                futures::pin_mut!(connect_fut);
                match futures::future::select(connect_fut, ctrl_c()).await {
                    Either::Left((result, _)) => result?,
                    Either::Right(((), _)) => return Ok(()),
                }
            };

            receive(
                wormhole,
                relay_server,
                file_path.as_os_str(),
                file_name.map(std::ffi::OsString::from).as_deref(),
                noconfirm,
                transit_abilities,
                ctrl_c,
            )
            .await?;
        },
        WormholeCommand::Forward(ForwardCommand::Serve {
            targets,
            common,
            common_leader: CommonLeaderArgs { code, code_length },
            ..
        }) => {
            // TODO make fancy
            log::warn!("This is an unstable feature. Make sure that your peer is running the exact same version of the program as you.");
            /* Map the CLI argument to Strings. Use the occasion to inspect them and fail early on malformed input. */
            let targets = targets
                .into_iter()
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
                let mut app_config = forwarding::APP_CONFIG;
                app_config.app_version.transit_abilities = parse_transit_args(&common);
                let connect_fut = parse_and_connect(
                    &mut term,
                    common.clone(),
                    code.clone(),
                    Some(code_length),
                    true,
                    app_config,
                    Some(&server_print_code),
                    clipboard.as_mut(),
                );
                futures::pin_mut!(connect_fut);
                let (wormhole, _code, relay_server) =
                    match futures::future::select(connect_fut, ctrl_c()).await {
                        Either::Left((result, _)) => result?,
                        Either::Right(((), _)) => break,
                    };
                let relay_server = vec![transit::RelayHint::from_urls(None, [relay_server])];
                async_std::task::spawn(forwarding::serve(
                    wormhole,
                    &transit::log_transit_connection,
                    relay_server,
                    targets.clone(),
                    ctrl_c(),
                ));
            }
        },
        WormholeCommand::Forward(ForwardCommand::Connect {
            ports,
            noconfirm,
            bind_address,
            common,
            common_follower: CommonFollowerArgs { code },
            ..
        }) => {
            // TODO make fancy
            log::warn!("This is an unstable feature. Make sure that your peer is running the exact same version of the program as you.");
            let mut app_config = forwarding::APP_CONFIG;
            app_config.app_version.transit_abilities = parse_transit_args(&common);
            let (wormhole, _code, relay_server) = parse_and_connect(
                &mut term,
                common,
                code,
                None,
                false,
                app_config,
                None,
                clipboard.as_mut(),
            )
            .await?;
            let relay_server = vec![transit::RelayHint::from_urls(None, [relay_server])];

            let offer = forwarding::connect(
                wormhole,
                &transit::log_transit_connection,
                relay_server,
                Some(bind_address),
                &ports,
            )
            .await?;
            log::info!("Mapping the following open ports to targets:");
            log::info!("  local port -> remote target (no address = localhost on remote)");
            for (port, target) in &offer.mapping {
                log::info!("  {} -> {}", port, target);
            }
            if noconfirm || util::ask_user("Accept forwarded ports?", true).await {
                offer.accept(ctrl_c()).await?;
            } else {
                offer.reject().await?;
            }
        },
        WormholeCommand::Help => {
            println!("Use --help to get help");
            std::process::exit(2);
        },
    }

    Ok(())
}

fn parse_transit_args(args: &CommonArgs) -> transit::Abilities {
    match (args.force_direct, args.force_relay) {
        (false, false) => transit::Abilities::ALL_ABILITIES,
        (true, false) => transit::Abilities::FORCE_DIRECT,
        (false, true) => transit::Abilities::FORCE_RELAY,
        (true, true) => unreachable!("These flags are mutually exclusive"),
    }
}

/**
 * Parse the necessary command line arguments to establish an initial server connection.
 * This is used over and over again by the different subcommands.
 *
 * If this `is_send` and the code is not specified via the CLI, then a code will be allocated.
 * Otherwise, the user will be prompted interactively to enter it.
 */
#[allow(deprecated)]
async fn parse_and_connect(
    term: &mut Term,
    common_args: CommonArgs,
    code: Option<String>,
    code_length: Option<usize>,
    is_send: bool,
    mut app_config: magic_wormhole::AppConfig<impl serde::Serialize + Send + Sync + 'static>,
    print_code: Option<&dyn Fn(&mut Term, &magic_wormhole::Code) -> eyre::Result<()>>,
    clipboard: Option<&mut ClipboardContext>,
) -> eyre::Result<(Wormhole, magic_wormhole::Code, url::Url)> {
    // TODO handle multiple relay servers correctly
    let relay_server: url::Url = common_args
        .relay_server
        .into_iter()
        .next()
        .unwrap_or_else(|| {
            magic_wormhole::transit::DEFAULT_RELAY_SERVER
                .parse()
                .unwrap()
        });
    let code = code
        .map(Result::Ok)
        .or_else(|| (!is_send).then(enter_code))
        .transpose()?
        .map(magic_wormhole::Code);

    if let Some(rendezvous_server) = common_args.rendezvous_server {
        app_config = app_config.rendezvous_url(rendezvous_server.into_string().into());
    }
    let (wormhole, code) = match code {
        Some(code) => {
            if is_send {
                print_code.expect("`print_code` must be `Some` when `is_send` is `true`")(
                    term, &code,
                )?;
            }
            let (server_welcome, wormhole) =
                magic_wormhole::Wormhole::connect_with_code(app_config, code).await?;
            print_welcome(term, &server_welcome)?;
            (wormhole, server_welcome.code)
        },
        None => {
            let numwords = code_length.unwrap();

            let (server_welcome, connector) =
                magic_wormhole::Wormhole::connect_without_code(app_config, numwords).await?;
            print_welcome(term, &server_welcome)?;
            /* Print code and also copy it to clipboard */
            if is_send {
                if let Some(clipboard) = clipboard {
                    match clipboard.set_contents(server_welcome.code.to_string()) {
                        Ok(()) => log::info!("Code copied to clipboard"),
                        Err(err) => log::warn!("Failed to copy code to clipboard: {}", err),
                    }
                }

                print_code.expect("`print_code` must be `Some` when `is_send` is `true`")(
                    term,
                    &server_welcome.code,
                )?;
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

// For file transfer
fn sender_print_code(term: &mut Term, code: &magic_wormhole::Code) -> eyre::Result<()> {
    writeln!(term, "\nThis wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole receive {}\n", &code)?;
    Ok(())
}

// For port forwarding
fn server_print_code(term: &mut Term, code: &magic_wormhole::Code) -> eyre::Result<()> {
    writeln!(term, "\nThis wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole forward connect {}\n", &code)?;
    Ok(())
}

async fn send(
    wormhole: Wormhole,
    relay_server: url::Url,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    transit_abilities: transit::Abilities,
    ctrl_c: impl Fn() -> futures::future::BoxFuture<'static, ()>,
) -> eyre::Result<()> {
    let pb = create_progress_bar(0);
    let pb2 = pb.clone();
    transfer::send_file_or_folder(
        wormhole,
        relay_server,
        file_path,
        file_name,
        transit_abilities,
        &transit::log_transit_connection,
        move |sent, total| {
            if sent == 0 {
                pb.reset_elapsed();
                pb.set_length(total);
                pb.enable_steady_tick(250);
            }
            pb.set_position(sent);
        },
        ctrl_c(),
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
    transit_abilities: transit::Abilities,
    ctrl_c: impl Fn() -> futures::future::BoxFuture<'static, ()>,
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
        transit_abilities,
        ctrl_c(),
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
            transit_abilities,
            ctrl_c(),
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
        transit_abilities: transit::Abilities,
        cancel: impl Future<Output = ()> + Send + 'static,
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
                    transit_abilities,
                    &transit::log_transit_connection,
                    move |_sent, _total| {
                        // if sent == 0 {
                        //     pb2.reset_elapsed();
                        //     pb2.enable_steady_tick(250);
                        // }
                        // pb2.set_position(sent);
                    },
                    cancel,
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
    noconfirm: bool,
    transit_abilities: transit::Abilities,
    ctrl_c: impl Fn() -> futures::future::BoxFuture<'static, ()>,
) -> eyre::Result<()> {
    let req = transfer::request_file(wormhole, relay_server, transit_abilities, ctrl_c())
        .await
        .context("Could not get an offer")?;
    /* If None, the task got cancelled */
    let req = match req {
        Some(req) => req,
        None => return Ok(()),
    };

    /*
     * Control flow is a bit tricky here:
     * - First of all, we ask if we want to receive the file at all
     * - Then, we check if the file already exists
     * - If it exists, ask whether to overwrite and act accordingly
     * - If it doesn't, directly accept, but DON'T overwrite any files
     */

    use number_prefix::NumberPrefix;
    if !(noconfirm
        || util::ask_user(
            format!(
                "Receive file '{}' ({})?",
                req.filename.display(),
                match NumberPrefix::binary(req.filesize as f64) {
                    NumberPrefix::Standalone(bytes) => format!("{} bytes", bytes),
                    NumberPrefix::Prefixed(prefix, n) =>
                        format!("{:.1} {}B in size", n, prefix.symbol()),
                },
            ),
            true,
        )
        .await)
    {
        return req.reject().await.context("Could not reject offer");
    }

    let file_name = file_name
        .or_else(|| req.filename.file_name())
        .ok_or_else(|| eyre::format_err!("The sender did not specify a valid file name, and neither did you. Try using --rename."))?;
    let file_path = Path::new(target_dir).join(file_name);

    let pb = create_progress_bar(req.filesize);

    let on_progress = move |received, _total| {
        pb.set_position(received);
    };

    /* Then, accept if the file exists */
    if !file_path.exists() || noconfirm {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .await
            .context("Failed to create destination file")?;
        return req
            .accept(
                &transit::log_transit_connection,
                on_progress,
                &mut file,
                ctrl_c(),
            )
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
        .accept(
            &transit::log_transit_connection,
            on_progress,
            &mut file,
            ctrl_c(),
        )
        .await
        .context("Receive process failed")?)
}
