mod seeds;
mod util;

use std::{
    ops::Deref,
    time::{Duration, Instant},
};

use async_std::{fs::OpenOptions, sync::Arc};
use clap::{Args, Parser, Subcommand};
use color_eyre::{eyre, eyre::Context};
use console::{style, Term};
use futures::{future::Either, Future, FutureExt};
use indicatif::{MultiProgress, ProgressBar};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

use magic_wormhole::{forwarding, transfer, transit, Wormhole};

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
    #[clap(
        short = 'c',
        long,
        value_name = "NUMWORDS",
        default_value = "2",
        conflicts_with = "code"
    )]
    code_length: usize,
    /// Send to one of your contacts instead of using a code
    #[clap(long, value_name = "NAME", conflicts_with_all = &["code", "code-length"])]
    to: Option<String>,
}

impl CommonLeaderArgs {
    fn into_connect_options<'a, 'b>(
        self,
        seeds: &'a seeds::Database,
        follower_command: &'b str,
    ) -> eyre::Result<ConnectOptions<'b>> {
        Ok(match (self.code, self.to) {
            (None, None) => ConnectOptions::GenerateCode {
                size: self.code_length,
                follower_command,
            },
            (Some(code), None) => ConnectOptions::ProvideCode(code),
            (None, Some(to)) if to == "myself" => ConnectOptions::ProvideSeed {
                seed: seeds.myself.into(),
                follower_command: Some(follower_command),
            },
            (None, Some(to)) => {
                let peer = seeds
                    .find(&to)
                    .ok_or_else(|| eyre::format_err!("Contact '{to}' not found"))?;
                ConnectOptions::ProvideSeed {
                    seed: peer.seed.into(),
                    follower_command: Some(follower_command),
                }
            },
            (Some(_), Some(_)) => unreachable!(),
        })
    }
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
    /// Receive from one of your contacts instead of using a code
    #[clap(long, value_name = "NAME", conflicts_with = "code")]
    from: Option<String>,
}

impl CommonFollowerArgs {
    fn into_connect_options<'a, 'b>(
        self,
        seeds: &'a seeds::Database,
    ) -> eyre::Result<ConnectOptions<'b>> {
        Ok(match (self.code, self.from) {
            (None, None) => ConnectOptions::EnterCode,
            (Some(code), None) => ConnectOptions::ProvideCode(code),
            (None, Some(from)) if from == "myself" => ConnectOptions::ProvideSeed {
                seed: seeds.myself.into(),
                follower_command: None,
            },
            (None, Some(from)) => {
                let peer = seeds
                    .find(&from)
                    .ok_or_else(|| eyre::format_err!("Contact '{from}' not found"))?;
                ConnectOptions::ProvideSeed {
                    seed: peer.seed.into(),
                    follower_command: None,
                }
            },
            (Some(_), Some(_)) => unreachable!(),
        })
    }
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
#[clap(arg_required_else_help = true)]
enum ContactCommand {
    /// List your existing contacts
    #[clap(
        visible_alias = "show",
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    List,
    /// Store a previously made connection in your contacts
    #[clap(
        mut_arg("help", |a| a.help("Print this help message")),
    )]
    Add {
        /// The ID of the previous connection
        #[clap(value_name = "ID")]
        id: String,
        /// The name under which to add the contact
        #[clap(value_name = "NAME")]
        name: String,
        /// Overwrite contacts with the same name
        #[clap(short, long)]
        force: bool,
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
    /// Manage your contacts to which you may send files without having to enter a code
    #[clap(subcommand, visible_alias = "contact")]
    Contacts(ContactCommand),
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

    let directories = directories::ProjectDirs::from("io", "magic-wormhole", "wormhole-rs")
        .ok_or_else(|| eyre::format_err!("Could not find the data storage location"))?;
    std::fs::create_dir_all(directories.data_dir()).context(format!(
        "Failed to create data dir at '{}'",
        directories.data_dir().display()
    ))?;
    let database_path = directories.data_dir().join("seeds.json");
    let mut seeds = if database_path.exists() {
        seeds::Database::load(&database_path).context(format!(
            "Failed to load '{}'. Please delete or fix it and try again",
            database_path.display()
        ))?
    } else {
        let mut seeds = seeds::Database::default();
        seeds.myself = rand::random();
        seeds
    };
    if seeds.our_names.is_empty() {
        let username =
            std::env::var("USER").context("Failed to fetch $USER environment variable")?;
        log::warn!(
            "No name configured yet. You will be identified to the other side as '{username}'."
        );
        log::warn!("If you are not comfortable with this, abort and use `wormhole-rs TODO` to set a differnt name. This warning won't be shown again.");
        seeds.our_names.push(username);
    }
    {
        let now = std::time::SystemTime::now();
        let old_size = seeds.peers.len();
        seeds.peers.retain(|_, peer| peer.expires() >= now);
        let new_size = seeds.peers.len();
        if old_size > new_size {
            log::info!("Removed {} old contacts from database", old_size - new_size);
        }
    }
    seeds.save(&database_path).context(format!(
        "Failed to write seeds database to '{}'",
        &database_path.display()
    ))?;
    let seed_ability = magic_wormhole::SeedAbility::<false> {
        display_names: seeds.our_names.clone(),
        known_seeds: seeds.iter_known_peers().collect(),
    };

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
            common_leader,
            common_send:
                CommonSenderArgs {
                    file_name,
                    file: file_path,
                },
            ..
        } => {
            let file_name = concat_file_name(&file_path, file_name.as_ref())?;

            eyre::ensure!(file_path.exists(), "{} does not exist", file_path.display());

            let (wormhole, _code, relay_server) = match util::cancellable(
                parse_and_connect(
                    &mut term,
                    common,
                    common_leader.into_connect_options(&seeds, "receive")?,
                    transfer::APP_CONFIG,
                    Some(seed_ability),
                    &mut seeds,
                    &database_path,
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
                ctrl_c.clone(),
            )
            .await?;
        },
        WormholeCommand::SendMany {
            tries,
            timeout,
            common,
            common_leader,
            common_send: CommonSenderArgs { file_name, file },
        } => {
            let (wormhole, code, relay_server) = {
                let connect_fut = parse_and_connect(
                    &mut term,
                    common,
                    common_leader.into_connect_options(&seeds, "receive")?,
                    transfer::APP_CONFIG,
                    None,
                    &mut seeds,
                    &database_path,
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
                &code.unwrap(),
                file.as_ref(),
                &file_name,
                tries,
                timeout,
                wormhole,
                &mut term,
                ctrl_c,
            )
            .await?;
        },
        WormholeCommand::Receive {
            noconfirm,
            common,
            common_follower,
            common_receiver:
                CommonReceiverArgs {
                    file_name,
                    file_path,
                },
            ..
        } => {
            let (wormhole, _code, relay_server) = {
                let connect_fut = parse_and_connect(
                    &mut term,
                    common,
                    common_follower.into_connect_options(&seeds)?,
                    transfer::APP_CONFIG,
                    Some(seed_ability),
                    &mut seeds,
                    &database_path,
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
                ctrl_c,
            )
            .await?;
        },
        WormholeCommand::Forward(ForwardCommand::Serve {
            targets,
            common,
            common_leader,
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
            let connect_options = common_leader.into_connect_options(&seeds, "forward connect")?;
            loop {
                let connect_fut = parse_and_connect(
                    &mut term,
                    common.clone(),
                    connect_options.clone(),
                    forwarding::APP_CONFIG,
                    None,
                    &mut seeds,
                    &database_path,
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
            common_follower,
        }) => {
            // TODO make fancy
            log::warn!("This is an unstable feature. Make sure that your peer is running the exact same version of the program as you.");
            let (wormhole, _code, relay_server) = parse_and_connect(
                &mut term,
                common,
                common_follower.into_connect_options(&seeds)?,
                forwarding::APP_CONFIG,
                None,
                &mut seeds,
                &database_path,
            )
            .await?;
            let relay_server = vec![transit::RelayHint::from_urls(None, [relay_server])];

            let offer =
                forwarding::connect(wormhole, relay_server, Some(bind_address), &ports).await?;
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
        WormholeCommand::Contacts(ContactCommand::List) => {
            use time::format_description::{Component, FormatItem};
            /* YYYY-mm-dd */
            let format = [
                FormatItem::Component(Component::Year(
                    time::format_description::modifier::Year::default(),
                )),
                FormatItem::Literal(b"-"),
                FormatItem::Component(Component::Month(
                    time::format_description::modifier::Month::default(),
                )),
                FormatItem::Literal(b"-"),
                FormatItem::Component(Component::Day(
                    time::format_description::modifier::Day::default(),
                )),
            ];

            #[allow(clippy::print_literal)]
            {
                let contacts = seeds
                    .peers
                    .iter()
                    .filter(|(_id, peer)| peer.contact_name.is_some())
                    .collect::<Vec<_>>();
                if contacts.is_empty() {
                    println!("You have no stored contacts yet!");
                } else {
                    println!("Known contacts:\n");
                    println!("{:8} {:12} {}", "ID", "NAME", "EXPIRES");
                    for (id, peer) in &contacts {
                        println!(
                            "{:8} {:12} {}",
                            id,
                            peer.contact_name.as_ref().unwrap(),
                            time::OffsetDateTime::from(peer.expires())
                                .date()
                                .format(&format[..])
                                .unwrap(),
                        );
                    }
                }
            }
            #[allow(clippy::print_literal)]
            {
                let contacts = seeds
                    .peers
                    .iter()
                    .filter(|(_id, peer)| peer.contact_name.is_none())
                    .collect::<Vec<_>>();
                if !contacts.is_empty() {
                    println!("\nContacts you haven't stored yet:");
                    println!("{:8} {:12} {}", "ID", "ALSO KNOWN AS", "EXPIRES");
                    for (id, peer) in &contacts {
                        println!(
                            "{:8} {:12} {}",
                            id,
                            peer.names.join(", "),
                            time::OffsetDateTime::from(peer.expires())
                                .date()
                                .format(&format[..])
                                .unwrap(),
                        );
                    }
                    println!("\nRun `wormhole-rs contact add <ID> <NAME>` to add them. Remember that both sides need to do this.");
                }
            }
        },
        WormholeCommand::Contacts(ContactCommand::Add { id, name, force }) => {
            eyre::ensure!(
                name != "myself",
                "'myself' is a reserved contact name, to send files between two clients on the same machine (mostly useful for testing)",
            );
            eyre::ensure!(
                !name.chars().any(|c| ['\'', '"', ','].contains(&c)),
                "For practical purposes, please choose a name without quotes or commas in it",
            );
            if !force {
                eyre::ensure!(
                    !seeds.peers.values().any(|peer| peer.contact_name.as_ref() == Some(&name)),
                    "You already stored a contact under that name. Either use --force to overwrite it, or chose another name",
                );
            }
            let peer = seeds
                .find_mut(&id)
                .ok_or_else(|| eyre::format_err!("No connection with that ID known"))?;
            if !force {
                eyre::ensure!(
                    peer.contact_name.is_none(),
                    "This contact is already known as {}. Use --force to rename it.",
                    peer.contact_name.as_ref().unwrap(),
                );
            }
            peer.contact_name = Some(name.clone());
            seeds.save(&database_path).context(format!(
                "Failed to write seeds database to '{}'",
                &database_path.display()
            ))?;
            log::info!("Successfully added '{name}' to your contacts.");
            log::info!("You can now use '--to {name}' or '--from {name}'");
        },
        WormholeCommand::Help => {
            println!("Use --help to get help");
            std::process::exit(2);
        },
    }

    Ok(())
}

#[derive(Clone)]
enum ConnectOptions<'a> {
    /* Leader/follower */
    ProvideCode(String),
    /* Leader only */
    GenerateCode {
        size: usize,
        follower_command: &'a str,
    },
    /* Follower only */
    EnterCode,
    /* Leader/follower */
    ProvideSeed {
        seed: xsalsa20poly1305::Key,
        follower_command: Option<&'a str>,
    },
}

/**
 * Parse the necessary command line arguments to establish an initial server connection.
 * This is used over and over again by the different subcommands.
 *
 * If this `is_send` and the code is not specified via the CLI, then a code will be allocated.
 * Otherwise, the user will be prompted interactively to enter it.
 */
#[allow(deprecated)]
async fn parse_and_connect<'a>(
    term: &'a mut Term,
    common_args: CommonArgs,
    connect_options: ConnectOptions<'_>,
    mut app_config: magic_wormhole::AppConfig<impl serde::Serialize>,
    seed_ability: Option<magic_wormhole::SeedAbility<false>>,
    seeds: &mut seeds::Database,
    database_path: &Path,
) -> eyre::Result<(Wormhole, Option<magic_wormhole::Code>, url::Url)> {
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

    if let Some(rendezvous_server) = common_args.rendezvous_server {
        app_config = app_config.rendezvous_url(rendezvous_server.into_string().into());
    }

    let (mut wormhole, code, relay_server) = match connect_options {
        /* Leader/follower */
        ConnectOptions::ProvideCode(code) => {
            let (server_welcome, wormhole) = magic_wormhole::Wormhole::connect_with_code(
                app_config,
                magic_wormhole::Code(code),
                seed_ability,
            )
            .await?;
            print_welcome(term, &server_welcome)?;
            (wormhole, Some(server_welcome.code), relay_server)
        },
        /* Leader only */
        ConnectOptions::GenerateCode {
            size,
            follower_command,
        } => {
            let (server_welcome, connector) =
                magic_wormhole::Wormhole::connect_without_code(app_config, size, seed_ability)
                    .await?;
            print_welcome(term, &server_welcome)?;
            print_code(term, &server_welcome.code, follower_command)?;
            let wormhole = connector.await?;
            (wormhole, Some(server_welcome.code), relay_server)
        },
        /* Follower only */
        ConnectOptions::EnterCode => {
            let code = magic_wormhole::Code(enter_code()?);
            let (server_welcome, wormhole) =
                magic_wormhole::Wormhole::connect_with_code(app_config, code, seed_ability).await?;
            print_welcome(term, &server_welcome)?;
            (wormhole, Some(server_welcome.code), relay_server)
        },
        /* Leader/follower */
        ConnectOptions::ProvideSeed {
            seed,
            follower_command,
        } => {
            if let Some(follower_command) = follower_command {
                print_seed(term, follower_command)?;
            }
            let mut wormhole =
                magic_wormhole::Wormhole::connect_with_seed(app_config, seed).await?;
            /* We don't want to execute the code block below if the connection came from a seed */
            wormhole.take_seed();
            (wormhole, None, relay_server)
        },
    };

    /* Handle the seeds result */
    if let Some(result) = wormhole.take_seed() {
        /* Sending to ourselves, are we? */
        if result
            .existing_seeds
            .contains(&xsalsa20poly1305::Key::from(seeds.myself))
        {
            log::info!(
                "You appear to be sending a file to yourself. You may use `--to myself` and `--from myself` instead.",
            );
        } else {
            /* We are interested in common seeds that we've already given a name */
            match seeds
                .peers
                .values_mut()
                .filter(|peer| {
                    result
                        .existing_seeds
                        .contains(&xsalsa20poly1305::Key::from(peer.seed))
                })
                .find(|peer| peer.contact_name.is_some())
            {
                /* We only care about the first one and ignore the others. It should be rare enough to see duplicate contacts */
                Some(peer) => {
                    peer.seen();
                    log::info!(
                        "You already know your peer as '{}'. You may use the appropriate `--to` and `--from` arguments for connecting to that person without having to enter a code.",
                        peer.contact_name.as_ref().unwrap(),
                    );
                },
                None => {
                    /* Check if we have at least one that wasn't saved */
                    match seeds.peers.iter_mut().find(|(_, peer)| {
                        result
                            .existing_seeds
                            .contains(&xsalsa20poly1305::Key::from(peer.seed))
                    }) {
                        Some((id, peer)) => {
                            peer.seen();
                            let name = if !peer.names.is_empty() && !peer.names[0].contains(' ') {
                                peer.names[0].clone()
                            } else {
                                "<contact name>".into()
                            };
                            log::info!(
                                "If you want to connect to your peer without password the next time, run"
                            );
                            log::info!("wormhole-rs contacts add {} {}", id, name);
                        },
                        None => {
                            /* New seed, store it in database */
                            let seed = result.session_seed;
                            let name = if !seed.display_names.is_empty()
                                && !seed.display_names[0].contains(' ')
                            {
                                seed.display_names[0].clone()
                            } else {
                                "<contact name>".into()
                            };
                            let id = seeds.insert_peer(seed);
                            log::info!(
                                "If you want to connect to your peer without password the next time, run"
                            );
                            log::info!("wormhole-rs contacts add {} {}", id, name);
                        },
                    }
                },
            }
            seeds.save(database_path).context(format!(
                "Failed to write seeds database to '{}'",
                &database_path.display()
            ))?;
        }
    }

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
fn print_code(term: &mut Term, code: &magic_wormhole::Code, command: &str) -> eyre::Result<()> {
    writeln!(term, "\nThis wormhole's code is: {}", &code)?;
    writeln!(term, "On the other computer, please run:\n")?;
    writeln!(term, "wormhole {} {}\n", command, &code)?;
    Ok(())
}

// For port forwarding
fn print_seed(term: &mut Term, command: &str) -> eyre::Result<()> {
    writeln!(term, "\nOn the other computer, please run (replace <NAME> with the corresponding name of the contact):\n")?;
    writeln!(term, "wormhole {} --from <NAME>\n", command)?;
    Ok(())
}

async fn send(
    wormhole: Wormhole,
    relay_server: url::Url,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    ctrl_c: impl Fn() -> futures::future::BoxFuture<'static, ()>,
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
        ctrl_c(),
    )
    .await
    .context("Send process failed")?;
    pb2.finish();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_many(
    relay_server: url::Url,
    code: &magic_wormhole::Code,
    file_path: &std::ffi::OsStr,
    file_name: &std::ffi::OsStr,
    max_tries: u64,
    timeout: Duration,
    wormhole: Wormhole,
    term: &mut Term,
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
            magic_wormhole::Wormhole::connect_with_code(transfer::APP_CONFIG, code.clone(), None)
                .await?;
        send_in_background(
            relay_server.clone(),
            Arc::clone(&file_path),
            Arc::clone(&file_name),
            wormhole,
            term.clone(),
            // &mp,
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
    ctrl_c: impl Fn() -> futures::future::BoxFuture<'static, ()>,
) -> eyre::Result<()> {
    let req = transfer::request_file(wormhole, relay_server, ctrl_c())
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
            .accept(on_progress, &mut file, ctrl_c())
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
        .accept(on_progress, &mut file, ctrl_c())
        .await
        .context("Receive process failed")?)
}
