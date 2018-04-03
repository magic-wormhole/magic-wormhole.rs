use events::Event;
use events::Event::{T_Close, T_MailboxDone, T_NameplateDone, T_Stopped};

pub struct Terminator {}

impl Terminator {
    pub fn new() -> Terminator {
        Terminator {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            T_Close => vec![],
            T_MailboxDone => vec![],
            T_NameplateDone => vec![],
            T_Stopped => vec![],
            _ => panic!(),
        }
    }
}
