extern crate magic_wormhole_core;
#[macro_use]
extern crate serde_json;
extern crate url;
extern crate ws;
use magic_wormhole_core::{APIAction, APIEvent, Action, IOAction, IOEvent,
                          TimerHandle, WSHandle, WormholeCore};
use std::cell::RefCell;
use std::rc::Rc;
use url::Url;

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &'static str = "ws://127.0.0.1:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

struct MyFactory {
    wsh: WSHandle,
    wcr: Rc<RefCell<WormholeCore>>,
}

struct MyHandler {
    wsh: WSHandle,
    wcr: Rc<RefCell<WormholeCore>>,
    out: ws::Sender,
}

fn main() {
    println!("start");
    // for now, pretend that the WormholeCore is only ever going to ask us to
    // make a single connection. Eventually, it will manage reconnects too,
    // and we must be prepared to make multiple connections when it asks.

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

    let f = MyFactory {
        wsh: wsh,
        wcr: Rc::new(RefCell::new(wc)),
    };

    // connect() blocks until disconnect, so the Manager drives everything
    // connect() requires a closure, use Builder to provide a real Factory
    //ws::connect(ws_url, f).unwrap();

    let b = ws::Builder::new();
    let mut w1 = b.build(f).unwrap();
    w1.connect(ws_url).unwrap();
    w1.run().unwrap();
}

impl ws::Factory for MyFactory {
    type Handler = MyHandler;
    fn connection_made(&mut self, out: ws::Sender) -> MyHandler {
        MyHandler {
            wsh: self.wsh,
            wcr: Rc::clone(&self.wcr),
            out: out,
        }
    }
}

impl ws::Handler for MyHandler {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        println!("on_open");
        let mut wc = self.wcr.borrow_mut();
        let actions = wc.do_io(IOEvent::WebSocketConnectionMade(self.wsh));
        process_actions(&self.out, actions);
        // TODO: this should go just after .start()
        let actions = wc.do_api(APIEvent::SetCode(
            "4-purple-sausages".to_string(),
        ));
        process_actions(&self.out, actions);
        let offer = json!({"offer": {"message": "hello from rust"}});
        // then expect {"answer": {"message_ack": "ok"}}
        let actions = wc.do_api(APIEvent::Send(offer.to_string().into_bytes()));
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
    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        println!("closing {:?} {}", code, reason);
        let mut wc = self.wcr.borrow_mut();
        let actions = wc.do_io(IOEvent::WebSocketConnectionLost(self.wsh));
        process_actions(&self.out, actions);
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
                // TODO: open new connections, handle timers
                _ => {
                    println!("action {:?}", io);
                }
            },
            Action::API(api) => match api {
                // TODO: deliver API events to app
                _ => println!("action {:?}", api),
            },
        }
    }
}
