use events::Events;

pub struct Allocator {}

impl Allocator {
    pub fn new() -> Allocator {
        Allocator {}
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        match event {
            Connected => events![],
            Lost => events![],
            RxAllocated => events![],
        }
    }
}
