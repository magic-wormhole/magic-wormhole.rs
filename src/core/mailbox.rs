use crate::core::{server_messages::OutboundMessage, EncryptedMessage, Event, Mood};
use std::collections::{HashMap, HashSet, VecDeque};

use super::events::{Mailbox, MySide, Phase};

#[derive(Debug)]
pub struct MailboxMachine {
    mailbox: Mailbox,
    processed: HashSet<Phase>,
    // TODO what do we track these for, as we're never actually reading that hashmap?
    // Maybe it was used for some reconnection logic. Do we still need that?
    pending_outbound: HashMap<Phase, Vec<u8>>,
    side: MySide,
}

impl MailboxMachine {
    pub fn new(side: &MySide, mailbox: Mailbox) -> MailboxMachine {
        MailboxMachine {
            mailbox,
            processed: HashSet::new(),
            pending_outbound: HashMap::new(),
            side: side.clone(),
        }
    }

    pub fn send_message(&mut self, actions: &mut VecDeque<Event>, phase: Phase, body: Vec<u8>) {
        actions.push_back(OutboundMessage::add(phase.clone(), &body).into());
        self.pending_outbound.insert(phase, body);
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
            self.pending_outbound.remove(&message.phase);
        }
        false
    }

    pub fn close(self, actions: &mut VecDeque<Event>, mood: Mood) {
        actions.push_back(OutboundMessage::close(self.mailbox, mood).into());
    }
}
