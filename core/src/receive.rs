use events::Event;
use events::Event::{R_GotCode, R_GotKey};

pub struct Receive {}

impl Receive {
    pub fn new() -> Receive {
        Receive {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            R_GotCode => vec![],
            R_GotKey => vec![],
            _ => panic!(),
        }
    }
}
