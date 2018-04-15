use events::Events;
// we process these
use events::ReceiveEvent;
// we emit these

pub struct Receive {}

impl Receive {
    pub fn new() -> Receive {
        Receive {}
    }

    pub fn process(&mut self, event: ReceiveEvent) -> Events {
        use events::ReceiveEvent::*;
        match event {
            GotCode => events![],
            GotKey(_) => events![],
        }
    }
}
