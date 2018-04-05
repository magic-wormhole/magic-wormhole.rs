use events::Events;
// we process these
use events::AllocatorEvent;
// we emit these

pub struct Allocator {}

impl Allocator {
    pub fn new() -> Allocator {
        Allocator {}
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        use events::AllocatorEvent::*;
        match event {
            Connected => events![],
            Lost => events![],
            RxAllocated => events![],
        }
    }
}
