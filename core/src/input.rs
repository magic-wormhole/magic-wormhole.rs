use events::Event;
use events::Event::{I_GotNameplates, I_GotWordlist, I_Start};

pub struct Input {}

impl Input {
    pub fn new() -> Input {
        Input {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            I_Start => vec![],
            I_GotNameplates => vec![],
            I_GotWordlist => vec![],
            _ => panic!(),
        }
    }
}
