extern crate ws;
extern crate magic_wormhole_core;
use magic_wormhole_core::{WSHandle, WormholeCore, create_core, Core};
use magic_wormhole_core::Action::{WebSocketOpen, WebSocketClose,
                                  WebSocketSendMessage};

// Can ws do hostname lookup? Use ip addr, not localhost, for now
const MAILBOX_SERVER: &'static str = "ws://127.0.0.1:4000/v1";
const APPID: &'static str = "lothar.com/wormhole/text-or-file-xfer";

struct Client {
    out: ws::Sender,
    wsh: WSHandle,
    wc: WormholeCore,
}

fn main() {
    println!("start");
    // for now, pretend that the WormholeCore is only ever going to ask us to
    // make a single connection. Eventually, it will manage reconnects too,
    // and we must be prepared to make multiple connections when it asks.
    let mut wc = create_core(APPID, MAILBOX_SERVER);
    let wsh;
    let ws_url;
    if let Some(WebSocketOpen(handle, url)) = wc.get_action() {
        wsh = handle;
        ws_url = url;
    } else {
        panic!();
    }
    /*match wc.get_action() {
        Some(WebSocketOpen(handle, url)) => {
            wsh = handle;
            ws_url = url;
        },
        _ => panic!(),
    };*/

    // connect() blocks until disconnect, so the Client drives everything
    ws::connect(ws_url, |out| {
        let c = Client { out: out, wsh: wsh, wc: wc };
        c
    }).unwrap();

}

impl ws::Handler for Client {
    fn on_open(&mut self, _: ws::Handshake) -> Result<(), ws::Error> {
        println!("on_open");
        Ok(())
    }
    fn on_message(&mut self, msg: ws::Message) -> Result<(), ws::Error> {
        println!("got message {}", msg);
        self.wc.websocket_message_received(self.wsh, msg.as_text().unwrap());
        process_actions(&mut self.wc, &self.out);
        Ok(())
    }
    fn on_close(&mut self, code: ws::CloseCode, reason: &str) {
        println!("closing {:?} {}", code, reason);
        self.wc.websocket_connection_lost(self.wsh);
        process_actions(&mut self.wc, &self.out);
    }
}

fn process_actions(wc: &mut WormholeCore, out: &ws::Sender) {
    while let Some(a) = wc.get_action() {
        match a {
            WebSocketSendMessage(_wsh, msg) => {
                out.send(msg);
            },
            WebSocketClose(_wsh) => {
                out.close(ws::CloseCode::Normal);
            },
            _ => {
                println!("action {:?}", a);
            },
        }
    }
}
