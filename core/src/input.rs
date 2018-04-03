use events::Event;
use events::Event::{I_Start, I_GotNameplates, I_GotWordlist};

pub struct Input {}

pub fn new() -> Input {
    Input {}
}

impl Input {
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            I_Start => vec![],
            I_GotNameplates => vec![],
            I_GotWordlist => vec![],
            _ => panic!(),
        }
    }
}
