use log::trace;
use std::collections::HashMap;
use std::collections::HashSet;

use super::api::Mood;
use super::events::{Events, Mailbox, MySide, Phase};
// we process these
use super::events::MailboxEvent;
use super::events::NameplateEvent::Release as N_Release;
use super::events::OrderEvent::GotMessage as O_GotMessage;
use super::events::RendezvousEvent::{
    TxAdd as RC_TxAdd, TxClose as RC_TxClose, TxOpen as RC_TxOpen,
};
use super::events::TerminatorEvent::MailboxDone as T_MailboxDone;
// we emit these

#[derive(Debug, PartialEq)]
enum State {
    // S0: We know nothing
    S0A,
    S0B,
    // S1: mailbox known
    S1A(Mailbox),
    // S2: mailbox known, maybe open
    S2A(Mailbox),
    S2B(Mailbox), // opened
    // S3: closing
    S3A(Mailbox, Mood),
    S3B(Mailbox, Mood),
    // S4: closed
    S4A,
    S4B,
}

pub struct MailboxMachine {
    state: Option<State>,
    side: MySide,
    pending_outbound: HashMap<Phase, Vec<u8>>, // HashMap<phase, body>
    processed: HashSet<Phase>,
}

impl MailboxMachine {
    pub fn new(side: &MySide) -> MailboxMachine {
        MailboxMachine {
            state: Some(State::S0A),
            side: side.clone(),
            pending_outbound: HashMap::new(),
            processed: HashSet::new(),
        }
    }

    fn send_open_and_queue(&mut self, actions: &mut Events, mailbox: &Mailbox) {
        actions.push(RC_TxOpen(mailbox.clone()));
        for (ph, body) in &self.pending_outbound {
            actions.push(RC_TxAdd(ph.clone(), body.to_vec()));
        }
    }

    pub fn process(&mut self, event: MailboxEvent) -> Events {
        use self::State::*;
        use MailboxEvent::*;

        trace!(
            "mailbox: current state = {:?}, got event = {:?}",
            self.state,
            event
        );

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0A => match event {
                Connected => S0B,
                Close(_) => {
                    self.pending_outbound.clear();
                    actions.push(T_MailboxDone);
                    S4A
                }
                GotMailbox(mailbox) => S1A(mailbox),
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S0A
                }
                _ => panic!(),
            },

            S0B => match event {
                Lost => S0A,
                Close(_) => {
                    self.pending_outbound.clear();
                    actions.push(T_MailboxDone);
                    S4B
                }
                GotMailbox(mailbox) => {
                    self.send_open_and_queue(&mut actions, &mailbox);
                    S2B(mailbox)
                }
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S0B
                }
                _ => panic!(),
            },

            S1A(mailbox) => match event {
                Connected => {
                    self.send_open_and_queue(&mut actions, &mailbox);
                    S2B(mailbox)
                }
                Close(_) => {
                    self.pending_outbound.clear();
                    actions.push(T_MailboxDone);
                    S4A
                }
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S1A(mailbox)
                }
                _ => panic!(),
            },

            S2A(mailbox) => match event {
                Connected => {
                    self.send_open_and_queue(&mut actions, &mailbox);
                    S2B(mailbox)
                }
                Close(mood) => {
                    self.pending_outbound.clear();
                    S3A(mailbox, mood)
                }
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S2A(mailbox)
                }
                _ => panic!(),
            },

            S2B(mailbox) => match event {
                Lost => S2A(mailbox),
                RxMessage(side, phase, body) => {
                    if *side != *self.side {
                        // theirs
                        actions.push(N_Release);
                        if !self.processed.contains(&phase) {
                            self.processed.insert(phase.clone());
                            actions.push(O_GotMessage(side, phase, body));
                        }
                    } else {
                        // ours
                        self.pending_outbound.remove(&phase);
                    }
                    S2B(mailbox)
                }
                Close(mood) => {
                    self.pending_outbound.clear();
                    actions.push(RC_TxClose(mailbox.clone(), mood));
                    S3B(mailbox, mood)
                }
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase.clone(), body.to_vec());
                    actions.push(RC_TxAdd(phase, body));
                    S2B(mailbox)
                }
                _ => panic!(),
            },

            S3A(mailbox, mood) => match event {
                Connected => {
                    actions.push(RC_TxClose(mailbox.clone(), mood));
                    S3B(mailbox, mood)
                }
                _ => panic!(),
            },

            S3B(mailbox, mood) => match event {
                Lost => S3A(mailbox, mood),
                // irrespective of the side, enter into S3B, do nothing,
                // generate no events
                RxMessage(..) => S3B(mailbox, mood),
                RxClosed => {
                    actions.push(T_MailboxDone);
                    S4B
                }
                Close(close_mood) => S3B(mailbox, close_mood),
                AddMessage(..) => S3B(mailbox, mood),
                _ => panic!(),
            },

            S4A => match event {
                Connected => S4B,
                _ => panic!(),
            },

            S4B => match event {
                Lost => S4A,
                RxMessage(..) | Close(_) | AddMessage(..) => old_state,
                _ => panic!(),
            },
        });

        actions
    }
}
