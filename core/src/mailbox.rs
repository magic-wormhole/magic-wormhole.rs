use events::Events;
// we process these
use events::MailboxEvent;
// we emit these

pub struct Mailbox {}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox {}
    }

    pub fn process(&mut self, event: MailboxEvent) -> Events {
        use events::MailboxEvent::*;
        match event {
            Connected => events![],
            Lost => events![],
            RxMessage => events![],
            RxClosed => events![],
            Close => events![],
            GotMailbox => events![],
            GotMessage => events![],
            AddMessage => events![],
        }
    }
}
