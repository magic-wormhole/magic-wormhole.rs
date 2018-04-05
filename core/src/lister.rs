use events::Events;
// we process these
use events::ListerEvent;
// we emit these

pub struct Lister {}

impl Lister {
    pub fn new() -> Lister {
        Lister {}
    }

    pub fn process(&mut self, event: ListerEvent) -> Events {
        use events::ListerEvent::*;
        match event {
            Connected => events![],
            Lost => events![],
            RxNameplates => events![],
            Refresh => events![],
        }
    }
}
