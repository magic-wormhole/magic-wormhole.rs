use events::Events;
// we process these
use events::KeyEvent;
// we emit these

pub struct Key {}

impl Key {
    pub fn new() -> Key {
        Key {}
    }

    pub fn process(&mut self, event: KeyEvent) -> Events {
        use events::KeyEvent::*;
        match event {
            GotCode(code) => events![],
            GotPake => events![],
            GotMessage => events![],
        }
    }
}
