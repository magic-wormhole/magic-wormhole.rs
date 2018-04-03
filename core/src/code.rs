use events::Event;
use events::Event::{C_Allocated, C_FinishedInput, C_GotNameplate};
pub struct Code {}

impl Code {
    pub fn new() -> Code {
        Code {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            C_Allocated => vec![],
            C_GotNameplate => vec![],
            C_FinishedInput => vec![],
            _ => panic!(),
        }
    }
}
