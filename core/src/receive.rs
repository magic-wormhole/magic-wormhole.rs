use events::Events;
// we process these
use events::ReceiveEvent;
// we emit these
use events::BossEvent::GotMessage as B_GotMessage;

pub struct Receive {}

impl Receive {
    pub fn new() -> Receive {
        Receive {}
    }

    pub fn process(&mut self, event: ReceiveEvent) -> Events {
        use events::ReceiveEvent::*;
        match event {
            GotMessage(side, phase, body) => events![B_GotMessage(side, phase, body)],
            GotKey(_) => events![],
        }
    }
}
