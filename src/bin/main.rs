use std::path::Path;
use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, Arg,
    SubCommand,
    AppSettings,
};
use magic_wormhole::core::{
    OfferType, PeerMessage,
};
use magic_wormhole::io::blocking::{Wormhole, filetransfer};
use std::str;
use log::*;
use anyhow::{Result, Error, ensure, bail, format_err, Context};

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
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
                .help("The file or directory to create, overriding the name suggested by the sender"),
        )
        .arg(
            Arg::with_name("code")
                .index(1)
                .value_name("CODE")
                .help("Provide the code now rather than typing it interactively")
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
        .subcommand(receive_command);
    let matches = clap.get_matches();

    if let Some(matches) = matches.subcommand_matches("send") {
        let relay_server = matches.value_of("relay-server").unwrap_or(RELAY_SERVER);

        let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
        match matches.value_of("code") {
            None => {
                let numwords = matches.value_of("code-length").unwrap().parse()?;
                w.allocate_code(numwords);
            }
            Some(code) => {
                w.set_code(code);
            }
        }
        let file = matches.value_of("file").unwrap();
        let code = async_std::task::block_on(w.get_code());
        info!("This wormhole's code is: {}", code);
        info!("On the other computer, please run:\n");
        info!("wormhole receive {}\n", code);

        send(w, relay_server, file)?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let relay_server = matches.value_of("relay-server").unwrap_or(RELAY_SERVER);
        let mut w = Wormhole::new(APPID, MAILBOX_SERVER);

        let code = matches.value_of("code")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| enter_code().expect("TODO handle this gracefully"));

        w.set_code(code.trim());

        receive(w, relay_server)?;
    } else {
        let code = matches.subcommand_name();
        // TODO implement this properly once clap 3.0 is out
        // if might_be_code(code) {
            warn!("No command provided, assuming you simply want to receive a file.");
            warn!("To receive files, use `wormhole receive <CODE>`.");
            warn!("To list all available commands and options, type `wormhole --help`");
            warn!("Please refer to `{} --help` for further usage instructions.", crate_name!());
        // } else {
            // clap.print_long_help();
        // }
        unimplemented!();
    }

    Ok(())
}

fn _might_be_code(code: Option<&str>) -> bool {
    unimplemented!()
}

fn enter_code() -> Result<String> {
    info!("Enter code: ");
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    Ok(code)
}

fn send(mut w: Wormhole, relay_server: &str, filename: impl AsRef<Path>) -> Result<()> {
    async_std::task::block_on(filetransfer::send_file(&mut w, filename, APPID, &relay_server.parse().unwrap()))?;

    w.close();
    Ok(())
}

fn send_many(relay_server: &str) -> Result<()> {
    loop {
        
    }
}

fn receive(mut w: Wormhole, relay_server: &str) -> Result<()> {
    match PeerMessage::deserialize(str::from_utf8(&async_std::task::block_on(w.get_message())).unwrap()) {
        PeerMessage::Transit(transit) => {
            async_std::task::block_on(filetransfer::receive_file(&mut w, transit, APPID, &relay_server.parse().unwrap()))?;
        },
        PeerMessage::Error(err) => {
            bail!("Something went wrong on the other side: {}", err);
        },
        other => {
            bail!("Got an unexpected message type, is the other side all right? Got: '{:?}'", other);
        }
    };

    w.close();
    Ok(())
}
