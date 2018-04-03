use events::Event;
use events::Event::{S_Send, S_GotVerifiedKey};

pub struct Send {}

pub fn new() -> Send {
    Send {}
}

impl Send {
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            S_Send => vec![],
            S_GotVerifiedKey => vec![],
            _ => panic!(),
        }
    }
}
