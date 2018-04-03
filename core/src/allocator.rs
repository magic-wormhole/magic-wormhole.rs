use events::Event;
use events::Event::{A_Connected, A_Lost, A_RxAllocated};
pub struct Allocator {}

pub fn new() -> Allocator {
    Allocator {}
}

impl Allocator {
    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            A_Connected => vec![],
            A_Lost => vec![],
            A_RxAllocated => vec![],
            _ => panic!(),
        }
    }
}
