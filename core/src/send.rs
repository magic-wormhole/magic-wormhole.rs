use events::Event;
use events::Event::{S_GotVerifiedKey, S_Send};

pub struct Send {}

impl Send {
    pub fn new() -> Send {
        Send {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            S_Send(plaintext) => vec![],
            S_GotVerifiedKey => vec![],
            _ => panic!(),
        }
    }
}
