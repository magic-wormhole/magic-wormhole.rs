extern crate ws;
extern crate url;
extern crate magic_wormhole_core;
use magic_wormhole_core::{WSHandle, WormholeCore, create_core, Core};
use magic_wormhole_core::Action::{WebSocketOpen, WebSocketClose,
                                  WebSocketSendMessage};
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
        
    let mut wc = create_core(APPID, MAILBOX_SERVER);
    let wsh;
    let ws_url;
    {
        if let Some(WebSocketOpen(handle, url)) = wc.get_action() {
            wsh = handle;
            ws_url = Url::parse(&url).unwrap();
        } else {
            panic!();
        }
    }
    let f = MyFactory { wsh: wsh, wcr: Rc::new(RefCell::new(wc)) };

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
        MyHandler { wsh: self.wsh, wcr: Rc::clone(&self.wcr), out: out }
    }
}

impl ws::Handler for MyHandler {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        println!("on_open");
        let mut wc = self.wcr.borrow_mut();
        wc.websocket_connection_made(self.wsh);
        Ok(())
    }
    fn on_message(&mut self, msg: ws::Message) -> Result<(), ws::Error> {
        println!("got message {}", msg);
        let mut wc = self.wcr.borrow_mut();
        wc.websocket_message_received(self.wsh, msg.as_text().unwrap());
        process_actions(&mut wc, &self.out);
        Ok(())
    }
    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        println!("closing {:?} {}", code, reason);
        let mut wc = self.wcr.borrow_mut();
        wc.websocket_connection_lost(self.wsh);
        process_actions(&mut wc, &self.out);
    }
}

fn process_actions(wc: &mut WormholeCore, out: &ws::Sender) {
    while let Some(a) = wc.get_action() {
        match a {
            WebSocketSendMessage(_wsh, msg) => {
                println!("sending {:?}", msg);
                out.send(msg).unwrap();
            },
            WebSocketClose(_wsh) => {
                out.close(ws::CloseCode::Normal).unwrap();
            },
            _ => {
                println!("action {:?}", a);
            },
        }
    }
}
