// Manage connections to the Mailbox Server (which used to be known as the
// Rendezvous Server). The "Mailbox" machine specifically handles the mailbox
// object within that server, whereas this module manages the websocket
// connection (reconnecting after a delay when necessary), preliminary setup
// messages, and message packing/unpacking/dispatch.

// in Twisted, we delegate all of this to a ClientService, so there's a lot
// more code and more states here

use core::traits::TimerHandle;


#[derive(Debug)]
enum State {
    Idle,
    Connecting,
    Connected,
    Disconnecting,
    Waiting,
    Stopped,
}

#[derive(Debug)]
pub struct Rendezvous {
    retry_timer: f32,
    state: Option<Box<State>>,
    connected_at_least_once: bool,
    reconnect_timer: Option<TimerHandle>,
}

pub fn create(retry_timer: f32) -> Rendezvous {
    Rendezvous {
        retry_timer: retry_timer,
        state: Some(Box::new(State::Idle)),
        connected_at_least_once: false,
        reconnect_timer: None,
    }
}

impl Rendezvous {
    pub fn start(&mut self) -> () {
        /*match self.state {
            ref Idle => {
                self.state = State::Connecting;
            },
            _ => panic!("bad transition from {:?}", self),
        }*/
    }
}
