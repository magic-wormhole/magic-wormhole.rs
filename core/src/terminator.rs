use events::Events;
// we process these
use events::TerminatorEvent;
// we emit these

pub struct Terminator {}

impl Terminator {
    pub fn new() -> Terminator {
        Terminator {}
    }

    pub fn process(&mut self, event: TerminatorEvent) -> Events {
        use events::TerminatorEvent::*;
        match event {
            Close(_mood) => events![],
            MailboxDone => events![],
            NameplateDone => events![],
            Stopped => events![],
        }
    }
}
