use events::Events;
// we process these
use events::InputEvent;
// we emit these

pub struct Input {}

impl Input {
    pub fn new() -> Input {
        Input {}
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        use events::InputEvent::*;
        match event {
            Start => events![],
            GotNameplates => events![],
            GotWordlist => events![],
        }
    }
}
