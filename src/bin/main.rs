use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, Arg,
    SubCommand,
};
use magic_wormhole::core::{
    error_message, message, message_ack, OfferType, PeerMessage,
};
use magic_wormhole::io::blocking::Wormhole;
use std::str;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &str = "ws://127.0.0.1:4000/v1";
const APPID: &str = "lothar.com/wormhole/text-or-file-xfer";

fn main() {
    env_logger::try_init().unwrap();
    let send = SubCommand::with_name("send")
        .aliases(&["tx"])
        .arg(
            Arg::with_name("zero")
                .short("0")
                .help("enable no-code anything-goes mode"),
        )
        .arg(
            Arg::with_name("code-length")
                .short("c")
                .long("code-length")
                .takes_value(true)
                .value_name("NUMWORDS")
                .default_value("2")
                .help("length of code (in bytes/words)"),
        )
        .arg(
            Arg::with_name("hide-progress")
                .long("hide-progress")
                .help("supress progress-bar display"),
        )
        .arg(
            Arg::with_name("no-listen")
                .long("no-listen")
                .help("(debug) don't open a listening socket for Transit"),
        )
        .arg(
            Arg::with_name("code")
                .long("code")
                .takes_value(true)
                .value_name("CODE")
                .help("human-generated code phrase"),
        )
        .arg(
            Arg::with_name("text")
                .long("text")
                .takes_value(true)
                .value_name("MESSAGE")
                .help("send a text message, not a file"),
        );
    let receive = SubCommand::with_name("receive")
        .aliases(&["rx"])
        .arg(
            Arg::with_name("zero")
                .short("0")
                .help("enable no-code anything-goes mode"),
        )
        .arg(
            Arg::with_name("code-length")
                .short("c")
                .long("code-length")
                .takes_value(true)
                .value_name("NUMWORDS")
                .help("length of code (in bytes/words)"),
        )
        .arg(
            Arg::with_name("verify")
                .short("v")
                .long("verify")
                .help("display verification string (and wait for approval)"),
        )
        .arg(
            Arg::with_name("hide-progress")
                .long("hide-progress")
                .help("supress progress-bar display"),
        )
        .arg(
            Arg::with_name("no-listen")
                .long("no-listen")
                .help("(debug) don't open a listening socket for Transit"),
        )
        .arg(
            Arg::with_name("only-text")
                .short("t")
                .long("only-text")
                .help("refuse file transfers, only accept text messages"),
        )
        .arg(
            Arg::with_name("accept-file")
                .long("accept-file")
                .help("accept file transfer without asking for confirmation"),
        )
        .arg(
            Arg::with_name("output-file")
                .short("o")
                .long("output-file")
                .takes_value(true)
                .value_name("FILENAME|DIRNAME")
                .help("The file or directory to create, overriding the name suggested by the sender"),
        )
        .arg(
            Arg::with_name("code")
                .help("provide code as argument, rather than typing it interactively")
        );

    let matches = App::new(crate_name!())
        .version(crate_version!())
        .about(crate_description!())
        .author(crate_authors!())
        .subcommand(send)
        .subcommand(receive)
        .get_matches();

    //println!("m: {:?}", &matches);

    if matches.subcommand_name() == None {
        println!("Must specify subcommand");
        return;
    }

    if let Some(sc) = matches.subcommand_matches("send") {
        if sc.value_of("text") == None {
            println!("file transfer is not yet implemented, so --text=MSG is required");
            return;
        }
        let text = sc.value_of("text").unwrap();
        println!("Sending text message ({} bytes)", text.len());

        let mut w = Wormhole::new(APPID, MAILBOX_SERVER);
        if sc.is_present("zero") {
            w.set_code("0");
        } else {
            match sc.value_of("code") {
                None => {
                    let s = sc.value_of("code-length").unwrap();
                    let numwords: usize = s.parse().unwrap();
                    w.allocate_code(numwords);
                }
                Some(code) => {
                    w.set_code(code);
                }
            }
        }
        let code = w.get_code();
        println!("Wormhole code is: {}", code);
        println!("On the other computer, please run:");
        println!();
        println!("wormhole receive {}", code);
        println!();
        w.send_message(message(text).serialize().as_bytes());
        let _ack = w.get_message();
        println!("text message sent");
    } else if let Some(sc) = matches.subcommand_matches("receive") {
        let mut w = Wormhole::new(APPID, MAILBOX_SERVER);

        if !sc.is_present("code") {
            println!("must provide CODE in argv: no interactive input yet");
            return;
        }
        let code = sc.value_of("code").unwrap();
        w.set_code(code);
        let msg = w.get_message();
        let actual_message =
            PeerMessage::deserialize(str::from_utf8(&msg).unwrap());
        match actual_message {
            PeerMessage::Offer(offer) => match offer {
                OfferType::Message(msg) => {
                    println!("{}", msg);
                    w.send_message(message_ack("ok").serialize().as_bytes());
                }
                OfferType::File { .. } => {
                    println!("Received file offer {:?}", offer);
                    w.send_message(
                        error_message("cannot handle file yet")
                            .serialize()
                            .as_bytes(),
                    );
                }
                OfferType::Directory { .. } => {
                    println!("Received directory offer: {:?}", offer);
                    w.send_message(
                        error_message("cannot handle directories yet")
                            .serialize()
                            .as_bytes(),
                    );
                }
            },
            PeerMessage::Answer(_) => {
                panic!("Should not receive answer type, I'm receiver")
            }
            PeerMessage::Error(err) => {
                println!("Something went wrong: {}", err)
            }
            PeerMessage::Transit(transit) => {
                // TODO: This should start transit server connection or
                // direct file transfer
                println!("Transit Message received: {:?}", transit)
            }
        };
        w.close();
    } else {
        panic!("shouldn't happen, unknown subcommand")
    }
}
