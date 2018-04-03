use events::Event;
use events::Event::{K_GotPake, K_GotMessage};

pub struct Key {}

pub fn new() -> Key {
    Key {}
}

impl Key {
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            K_GotPake => vec![],
            K_GotMessage => vec![],
            _ => panic!(),
        }
    }
}

