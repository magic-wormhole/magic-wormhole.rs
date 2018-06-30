extern crate magic_wormhole_core;
extern crate parking_lot;
extern crate regex;
extern crate rustyline;
extern crate serde_json;
extern crate url;
extern crate ws;

use magic_wormhole_core::{
    APIAction, APIEvent, Action, IOAction, IOEvent, OfferType, PeerMessage,
    WSHandle, WormholeCore,
};

use std::error::Error;
use std::io;
use std::sync::{
    mpsc::{channel, Sender}, Arc,
};
use std::thread::{sleep, spawn};
use std::time::Duration;

use parking_lot::{deadlock, Mutex};
use regex::Regex;
use rustyline::{completion::Completer, error::ReadlineError};
use url::Url;

const MAILBOX_SERVER: &'static str = "ws://localhost:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

struct Factory {
    wsh: WSHandle,
    wcr: Arc<Mutex<WormholeCore>>,
}

struct WSHandler {
    wsh: WSHandle,
    wcr: Arc<Mutex<WormholeCore>>,
    out: ws::Sender,
}

impl ws::Factory for Factory {
    type Handler = WSHandler;
    fn connection_made(&mut self, out: ws::Sender) -> WSHandler {
        WSHandler {
            wsh: self.wsh,
            wcr: Arc::clone(&self.wcr),
            out: out,
        }
    }
}

struct CodeCompleter {
    wcr: Arc<Mutex<WormholeCore>>,
    tx_event: Sender<APIEvent>,
}

impl Completer for CodeCompleter {
    fn complete(
        &self,
        line: &str,
        _pos: usize,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let got_nameplate = line.find('-').is_some();
        let mwc = Arc::clone(&self.wcr);

        if got_nameplate {
            let ns: Vec<_> = line.splitn(2, '-').collect();
            let nameplate = ns[0].to_string();
            let word = ns[1].to_string();
            let committed_nameplate;

            {
                let mc = mwc.lock();
                committed_nameplate = mc.input_helper_committed_nameplate()
                    .map(|s| s.to_string());
            }

            if committed_nameplate.is_some() {
                if committed_nameplate.unwrap() != nameplate {
                    return Err(ReadlineError::from(io::Error::new(
                        io::ErrorKind::Other,
                        "Nameplate already chosen can't go back",
                    )));
                }

                let mut wc = mwc.lock();
                let completions = wc.input_helper_get_word_completions(&word);
                drop(&wc);

                match completions {
                    Ok(completions) => Ok((
                        0,
                        completions
                            .iter()
                            .map(|c| format!("{}-{}", nameplate, c))
                            .collect(),
                    )),
                    Err(err) => Err(ReadlineError::from(io::Error::new(
                        io::ErrorKind::Other,
                        err.description(),
                    ))),
                }
            } else {
                self.tx_event
                    .send(APIEvent::InputHelperChooseNameplate(
                        nameplate.clone(),
                    ))
                    .unwrap();
                Ok((0, Vec::new()))
            }
        } else {
            let nameplate = line.to_string();
            let mut mc = mwc.lock();
            let completions =
                mc.input_helper_get_nameplate_completions(&nameplate);
            match completions {
                Ok(completions) => Ok((0, completions)),
                Err(err) => Err(ReadlineError::from(io::Error::new(
                    io::ErrorKind::Other,
                    err.description(),
                ))),
            }
        }
    }
}

impl ws::Handler for WSHandler {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        // println!("On_open");
        let mwc = Arc::clone(&self.wcr);
        {
            println!("Initial lock on socket open");
            let mut wc = mwc.lock();
            let actions = wc.do_io(IOEvent::WebSocketConnectionMade(self.wsh));
            process_actions(&self.out, actions);

            // TODO: This is for experiment we are starting the Input machine
            // manually
            let actions = wc.do_api(APIEvent::InputCode);
            process_actions(&self.out, actions);
            println!("Initial lock should be dropped now");
        }

        let (tx_event, rx_event) = channel();
        let completer = CodeCompleter {
            wcr: Arc::clone(&self.wcr),
            tx_event: tx_event.clone(),
        };

        spawn(move || {
            let mut rl = rustyline::Editor::new();
            rl.set_completer(Some(completer));
            loop {
                match rl.readline("Enter receive wormhole code: ") {
                    Ok(line) => {
                        if line.trim().is_empty() {
                            // Wait till user enter the code
                            continue;
                        }
                        let re = Regex::new(r"\d+-\w+-\w+").unwrap();
                        if !re.is_match(&line) {
                            panic!("Not a valid code format");
                        }
                        let pieces: Vec<_> = line.splitn(2, '-').collect();
                        let words = pieces[1].to_string();
                        tx_event
                            .send(APIEvent::InputHelperChooseWords(words))
                            .unwrap();
                        break;
                    }
                    Err(rustyline::error::ReadlineError::Interrupted) => {
                        println!("Interrupted");
                        continue;
                    }
                    Err(rustyline::error::ReadlineError::Eof) => {
                        println!("Got EOF");
                        break;
                    }
                    Err(err) => {
                        println!("Error: {:?}", err);
                        break;
                    }
                }
            }
        });

        let out_events = self.out.clone();
        let emwc = Arc::clone(&self.wcr);
        spawn(move || {
            for received in rx_event {
                let mut wc = emwc.lock();
                let actions = wc.do_api(received);
                process_actions(&out_events, actions);
            }
        });

        Ok(())
    }

    fn on_message(&mut self, msg: ws::Message) -> Result<(), ws::Error> {
        println!("got message {}", msg);
        let mwc = Arc::clone(&self.wcr);
        let text = msg.as_text()?.to_string();
        let rx = IOEvent::WebSocketMessageReceived(self.wsh, text);

        let mut wc = mwc.lock();
        let actions = wc.do_io(rx);
        process_actions(&self.out, actions);

        Ok(())
    }
}

fn process_actions(out: &ws::Sender, actions: Vec<Action>) {
    for a in actions {
        match a {
            Action::IO(io) => match io {
                IOAction::WebSocketSendMessage(_wsh, msg) => {
                    // println!("sending {:?}", msg);
                    out.send(msg).unwrap();
                }
                IOAction::WebSocketClose(_wsh) => {
                    out.close(ws::CloseCode::Normal).unwrap();
                }
                _ => {
                    // println!("action: {:?}", io);
                }
            },
            Action::API(api) => match api {
                APIAction::GotMessage(msg) => {
                    let message = String::from_utf8(msg).unwrap();
                    let peer_msg = PeerMessage::deserialize(&message);
                    match peer_msg {
                        PeerMessage::Offer(offer) => match offer {
                            OfferType::Message(msg) => println!("{}", msg),
                            OfferType::File { .. } => {
                                println!("Recieved file offer")
                            }
                            OfferType::Directory { .. } => {
                                println!("Recieved directory offer")
                            }
                        },
                        _ => panic!("Unknown message received: {:?}", peer_msg),
                    }
                }
                _ => println!("action {:?}", api),
            },
        }
    }
}

fn main() {
    println!("Receive start");

    let mut wc = WormholeCore::new(APPID, MAILBOX_SERVER);
    let wsh;
    let ws_url;
    let mut actions = wc.start();

    if let Action::IO(IOAction::WebSocketOpen(handle, url)) =
        actions.pop().unwrap()
    {
        wsh = handle;
        ws_url = Url::parse(&url).unwrap();
    } else {
        panic!();
    }

    let f = Factory {
        wsh: wsh,
        wcr: Arc::new(Mutex::new(wc)),
    };

    spawn(move || loop {
        sleep(Duration::from_secs(10));
        let deadlocks = deadlock::check_deadlock();
        if deadlocks.is_empty() {
            continue;
        }

        println!("{} deadlocks detected", deadlocks.len());
        for (i, threads) in deadlocks.iter().enumerate() {
            println!("Deadlock #{}", i);
            for t in threads {
                println!("Thread Id {:#?}", t.thread_id());
                println!("{:#?}", t.backtrace());
            }
        }
    });

    let b = ws::Builder::new();
    let mut w1 = b.build(f).unwrap();
    w1.connect(ws_url).unwrap();
    w1.run().unwrap();
}
