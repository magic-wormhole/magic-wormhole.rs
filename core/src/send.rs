use events::Events;
// we process these
use events::SendEvent;
// we emit these

pub struct Send {}

impl Send {
    pub fn new() -> Send {
        Send {}
    }

    pub fn process(&mut self, event: SendEvent) -> Events {
        use events::SendEvent::*;
        match event {
            Send(plaintext) => events![],
            GotVerifiedKey => events![],
        }
    }
}
