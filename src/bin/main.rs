use clap::{
    crate_authors, crate_description, crate_name, crate_version, App, Arg,
    SubCommand,
};

fn main() {
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
        if let Some(text) = sc.value_of("text") {
            println!("send text {}", text);
        } else {
            println!("file transfer is not yet implemented, so --text=MSG is required");
            return;
        }
    } else if let Some(_sc) = matches.subcommand_matches("receive") {
        println!("receive");
    } else {
        panic!("shouldn't happen, unknown subcommand")
    }
}
