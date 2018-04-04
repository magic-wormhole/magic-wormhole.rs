use events::Events;
enum AllocatorEvent {
    Connected,
    Lost,
    RxAllocated,
}

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
