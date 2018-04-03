use events::Event;
use events::Event::{M_AddMessage, M_Close, M_Connected, M_GotMailbox,
                    M_GotMessage, M_Lost, M_RxClosed, M_RxMessage};

pub struct Mailbox {}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox {}
    }

    pub fn process(&mut self, event: Event) -> Vec<Event> {
        match event {
            M_Connected => vec![],
            M_Lost => vec![],
            M_RxMessage => vec![],
            M_RxClosed => vec![],
            M_Close => vec![],
            M_GotMailbox => vec![],
            M_GotMessage => vec![],
            M_AddMessage => vec![],
            _ => panic!(),
        }
    }
}
