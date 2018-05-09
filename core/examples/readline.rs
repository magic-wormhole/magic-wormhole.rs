extern crate magic_wormhole_core;
extern crate rustyline;
#[macro_use]
extern crate serde_json;
extern crate url;
extern crate ws;

use magic_wormhole_core::{APIAction, APIEvent, Action, IOAction, IOEvent,
                          TimerHandle, WSHandle, WormholeCore};

use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use url::Url;
use rustyline::completion::{extract_word, Completer};

const MAILBOX_SERVER: &'static str = "ws://localhost:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

struct Factory {
    wsh: WSHandle,
    wcr: Rc<RefCell<WormholeCore>>,
}

struct WSHandler {
    wsh: WSHandle,
    wcr: Rc<RefCell<WormholeCore>>,
    out: ws::Sender,
}

impl ws::Factory for Factory {
    type Handler = WSHandler;
    fn connection_made(&mut self, out: ws::Sender) -> WSHandler {
        WSHandler {
            wsh: self.wsh,
            wcr: Rc::clone(&self.wcr),
            out: out,
        }
    }
}

struct CodeCompleter<'a> {
    wcr: Rc<RefCell<WormholeCore>>,
    out: &'a ws::Sender,
}

static BREAK_CHARS: [char; 1] = [' '];

impl<'a> Completer for CodeCompleter<'a> {
    fn complete(
        &self,
        line: &str,
        pos: usize,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let (start, word) =
            extract_word(line, pos, &BREAK_CHARS.iter().cloned().collect());
        let mut wc = self.wcr.borrow_mut();
        let (actions, completions) = wc.get_completions(word);
        process_actions(&self.out, actions);
        Ok((start, completions))
    }
}

impl ws::Handler for WSHandler {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        println!("On_open");
        {
            let mut wc = self.wcr.borrow_mut();
            let actions = wc.do_io(IOEvent::WebSocketConnectionMade(self.wsh));
            process_actions(&self.out, actions);

            // TODO: This is for experiment we are starting the Input machine
            // manually
            let actions = wc.do_api(APIEvent::InputCode);
            process_actions(&self.out, actions);
        }

        let mut line_out = String::new();
        {
            let completer = CodeCompleter {
                wcr: Rc::clone(&self.wcr),
                out: &self.out,
            };

            let mut rl = rustyline::Editor::new();
            rl.set_completer(Some(completer));
            loop {
                match rl.readline("Enter receive wormhole code: ") {
                    Ok(line) => {
                        if line.trim().is_empty() {
                            // Wait till user enter the code
                            continue;
                        }

                        // We got full code lets inform input about it.
                        line_out = line.to_string();
                        break;
                    }
                    Err(rustyline::error::ReadlineError::Interrupted) => {
                        println!("Interrupted");
                        continue;
                    }
                    Err(rustyline::error::ReadlineError::Eof) => {
                        break;
                    }
                    Err(err) => {
                        println!("Error: {:?}", err);
                        break;
                    }
                }
            }
        }

        let mut wc = self.wcr.borrow_mut();
        let actions = wc.do_api(APIEvent::HelperChoseWord(line_out));
        process_actions(&self.out, actions);

        Ok(())
    }

    fn on_message(&mut self, msg: ws::Message) -> Result<(), ws::Error> {
        println!("got message {}", msg);
        let mut wc = self.wcr.borrow_mut();
        let text = msg.as_text()?.to_string();
        let rx = IOEvent::WebSocketMessageReceived(self.wsh, text);
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
                    println!("sending {:?}", msg);
                    out.send(msg).unwrap();
                }
                IOAction::WebSocketClose(_wsh) => {
                    out.close(ws::CloseCode::Normal).unwrap();
                }
                _ => {
                    println!("action: {:?}", io);
                }
            },
            Action::API(api) => match api {
                APIAction::GotMessage(msg) => println!(
                    "API Got Message: {}",
                    String::from_utf8(msg).unwrap()
                ),
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
        wcr: Rc::new(RefCell::new(wc)),
    };

    let b = ws::Builder::new();
    let mut w1 = b.build(f).unwrap();
    w1.connect(ws_url).unwrap();
    w1.run().unwrap();
}
