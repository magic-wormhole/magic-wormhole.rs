use events::Event;
use events::Event::O_GotMessage;

pub struct Order {}

impl Order {
    pub fn new() -> Order {
        Order {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            O_GotMessage => vec![],
            _ => panic!(),
        }
    }
}
