use std::path::Path;
use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, Arg,
    SubCommand,
    AppSettings,
};
use magic_wormhole::core::{
    PeerMessage,
};
use magic_wormhole::io::blocking::{Wormhole, filetransfer};
use std::str;
use log::*;
use anyhow::{Result, Error, ensure, bail, format_err, Context};

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://relay.magic-wormhole.io:4000/v1";
const RELAY_SERVER: &str = "tcp:transit.magic-wormhole.io:4001";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

#[async_std::main]
async fn main() -> Result<()> {
    env_logger::builder()
        .filter_level(LevelFilter::Debug)
        .filter_module("magic_wormhole::core", LevelFilter::Trace)
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
        .subcommand(send_many_command)
        .subcommand(receive_command);
    let matches = clap.get_matches();

    if let Some(matches) = matches.subcommand_matches("send") {
        let relay_server = matches.value_of("relay-server").unwrap_or(RELAY_SERVER);
        if true {
            use magic_wormhole::io::blocking::{Wormhole2, CodeProvider};
            let (welcome, connector) = Wormhole2::new(APPID, MAILBOX_SERVER, match matches.value_of("code") {
                None => {
                    let numwords = matches.value_of("code-length").unwrap().parse().expect("TODO error handling");
                    CodeProvider::AllocateCode(numwords)
                }
                Some(code) => {
                    CodeProvider::SetCode(code.to_string())
                }
            }).await;
            info!("Got welcome: {}", &welcome.welcome);
            info!("This wormhole's code is: {}", &welcome.code);
            info!("On the other computer, please run:\n");
            info!("wormhole receive {}\n", &welcome.code);
            let (mut tx, mut rx, key) = connector.do_pake().await;
            info!("Got key: {:x?}", key);
            let file = matches.value_of("file").unwrap();
            filetransfer::send_file(
                &key,
                &mut tx,
                &mut rx,
                file,
                APPID,
                &relay_server.parse().unwrap(),
            ).await.unwrap();
            // async_std::task::sleep(std::time::Duration::from_secs(5)).await;
        } else {
            let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
            match matches.value_of("code") {
                None => {
                    let numwords = matches.value_of("code-length").unwrap().parse()?;
                    w.allocate_code(numwords).await;
                }
                Some(code) => {
                    w.set_code(code).await;
                }
            }
            let file = matches.value_of("file").unwrap();
            let code = w.get_code().await;
            info!("This wormhole's code is: {}", code);
            info!("On the other computer, please run:\n");
            info!("wormhole receive {}\n", code);

            async_std::task::sleep(std::time::Duration::from_secs(5)).await;
            // w.get_key().await;
            // send(w, relay_server, file)?;
        }
    } else if let Some(matches) = matches.subcommand_matches("send-many") {
        let relay_server = matches.value_of("relay-server").unwrap_or(RELAY_SERVER);
        let code = match matches.value_of("code") {
            None => {
                let numwords = matches.value_of("code-length").unwrap().parse()?;
                let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
                w.allocate_code(numwords).await;
                let code = w.get_code().await;
                w.close().await;
                code
            }
            Some(code) => code.trim().to_owned(),
        };
        let file = matches.value_of("file").unwrap();

        info!("This wormhole's code is: {}", code);
        info!("On the other computer, please run:\n");
        info!("wormhole receive {}\n", code);
        
        send_many(relay_server, &code, file).await?;
    } else if let Some(matches) = matches.subcommand_matches("receive") {
        let relay_server = matches.value_of("relay-server").unwrap_or(RELAY_SERVER);
        let mut w = Wormhole::new(APPID, MAILBOX_SERVER);

        let code = matches.value_of("code")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| enter_code().expect("TODO handle this gracefully"));

        w.set_code(code.trim()).await;

        receive(w, relay_server).await?;
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

fn _might_be_code(_code: Option<&str>) -> bool {
    unimplemented!()
}

fn enter_code() -> Result<String> {
    info!("Enter code: ");
    let mut code = String::new();
    std::io::stdin().read_line(&mut code)?;
    Ok(code)
}

async fn send(mut w: Wormhole, relay_server: &str, filename: impl AsRef<Path>) -> Result<()> {
    todo!();
    // let result = filetransfer::send_file(&mut w, filename, APPID, &relay_server.parse().unwrap()).await;
    w.close().await;
    // result
}

async fn send_many(relay_server: &str, code: &str, filename: impl AsRef<Path>) -> Result<()> {
    loop {
        // match {
        //     let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
        //     w.set_code(code).await;
        //     w.get_code().await;
        //     todo!();
        //     // let result = filetransfer::send_file(&mut w, &filename, APPID, &relay_server.parse().unwrap()).await;
        //     w.close().await;
        //     // result
        // } {
        //     Ok(_) => {
        //         info!("TOOD success message");
        //     },
        //     Err(e) => {
        //         warn!("Send failed, {}", e);
        //     }
        // }
    }
    // Ok(())
}

async fn receive(mut w: Wormhole, relay_server: &str) -> Result<()> {
    match PeerMessage::deserialize(str::from_utf8(&w.get_message().await).unwrap()) {
        PeerMessage::Transit(transit) => {
            todo!();
            // filetransfer::receive_file(&mut w, transit, APPID, &relay_server.parse().unwrap()).await?;
        },
        PeerMessage::Error(err) => {
            bail!("Something went wrong on the other side: {}", err);
        },
        other => {
            bail!("Got an unexpected message type, is the other side all right? Got: '{:?}'", other);
        }
    };

    w.close().await;
    Ok(())
}
