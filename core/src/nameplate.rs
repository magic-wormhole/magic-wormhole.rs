use events::Events;
// we process these
use events::NameplateEvent;
// we emit these

pub struct Nameplate {}

impl Nameplate {
    pub fn new() -> Nameplate {
        Nameplate {}
    }

    pub fn process(&mut self, event: NameplateEvent) -> Events {
        use events::NameplateEvent::*;
        match event {
            NameplateDone => events![],
            Connected => events![],
            Lost => events![],
            RxClaimed => events![],
            RxReleased => events![],
            SetNameplate(nameplate) => events![],
            Release => events![],
        }
    }
}
