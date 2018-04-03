use events::Event;
use events::Event::{K_GotMessage, K_GotPake};

pub struct Key {}

impl Key {
    pub fn new() -> Key {
        Key {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            K_GotPake => vec![],
            K_GotMessage => vec![],
            _ => panic!(),
        }
    }
}
