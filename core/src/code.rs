use events::Events;
// we process these
use events::CodeEvent;
// we emit these

pub struct Code {}

impl Code {
    pub fn new() -> Code {
        Code {}
    }

    pub fn process(&mut self, event: CodeEvent) -> Events {
        use events::CodeEvent::*;
        match event {
            AllocateCode => events![],
            InputCode => events![],
            SetCode(code) => events![],
            Allocated => events![],
            GotNameplate => events![],
            FinishedInput => events![],
        }
    }
}
