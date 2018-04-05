use events::Events;
// we process these
use events::OrderEvent;
// we emit these

pub struct Order {}

impl Order {
    pub fn new() -> Order {
        Order {}
    }

    pub fn process(&mut self, event: OrderEvent) -> Events {
        use events::OrderEvent::*;
        match event {
            GotMessage => events![],
        }
    }
}
