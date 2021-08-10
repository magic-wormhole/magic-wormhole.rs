use crate::core::{
    server_messages::{EncryptedMessage, OutboundMessage},
    Event, Mood,
};
use std::collections::{HashSet, VecDeque};

use super::events::{Mailbox, MySide, Phase};

#[derive(Debug, derive_more::Display)]
#[display(
    fmt = "MailboxMachine {{ mailbox: {}, side: {}, processed: [{}] }} ",
    mailbox,
    side,
    "processed.iter().map(|p| format!(\"{}\", p)).collect::<Vec<String>>().join(\", \")"
)]
pub struct MailboxMachine {
    mailbox: Mailbox,
    processed: HashSet<Phase>,
    side: MySide,
}

impl MailboxMachine {
    pub fn new(side: &MySide, mailbox: Mailbox) -> MailboxMachine {
        MailboxMachine {
            mailbox,
            processed: HashSet::new(),
            side: side.clone(),
        }
    }

    pub fn send_message(&mut self, actions: &mut VecDeque<Event>, phase: Phase, body: Vec<u8>) {
        actions.push_back(OutboundMessage::add(phase, body).into());
    }

    pub fn receive_message(&mut self, message: &EncryptedMessage) -> bool {
        if *message.side != *self.side {
            // Got a message from them
            if !self.processed.contains(&message.phase) {
                self.processed.insert(message.phase.clone());
                return true;
            }
        } else {
            // Echo of ours. Ignore
        }
        false
    }

    pub fn close(self, actions: &mut VecDeque<Event>, mood: Mood) {
        actions.push_back(OutboundMessage::close(self.mailbox, mood).into());
    }
}
