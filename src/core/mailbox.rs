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
    S2B(Mailbox), // opened
    // S3: closing
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
                },
                GotMailbox(mailbox) => S1A(mailbox),
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S0A
                },
                _ => panic!(),
            },

            S0B => match event {
                Close(_) => {
                    self.pending_outbound.clear();
                    actions.push(T_MailboxDone);
                    S4B
                },
                GotMailbox(mailbox) => {
                    self.send_open_and_queue(&mut actions, &mailbox);
                    S2B(mailbox)
                },
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S0B
                },
                _ => panic!(),
            },

            S1A(mailbox) => match event {
                Connected => {
                    self.send_open_and_queue(&mut actions, &mailbox);
                    S2B(mailbox)
                },
                Close(_) => {
                    self.pending_outbound.clear();
                    actions.push(T_MailboxDone);
                    S4A
                },
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase, body);
                    S1A(mailbox)
                },
                _ => panic!(),
            },

            S2B(mailbox) => match event {
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
                },
                Close(mood) => {
                    self.pending_outbound.clear();
                    actions.push(RC_TxClose(mailbox.clone(), mood));
                    S3B(mailbox, mood)
                },
                AddMessage(phase, body) => {
                    self.pending_outbound.insert(phase.clone(), body.to_vec());
                    actions.push(RC_TxAdd(phase, body));
                    S2B(mailbox)
                },
                _ => panic!(),
            },

            S3B(mailbox, mood) => match event {
                // irrespective of the side, enter into S3B, do nothing,
                // generate no events
                RxMessage(..) => S3B(mailbox, mood),
                RxClosed => {
                    actions.push(T_MailboxDone);
                    S4B
                },
                Close(close_mood) => S3B(mailbox, close_mood),
                AddMessage(..) => S3B(mailbox, mood),
                _ => panic!(),
            },

            S4A => match event {
                Connected => S4B,
                _ => panic!(),
            },

            S4B => match event {
                RxMessage(..) | Close(_) | AddMessage(..) => old_state,
                _ => panic!(),
            },
        });

        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::api::Mood;
    use crate::core::events::{
        MailboxEvent::*, MySide, NameplateEvent, OrderEvent, RendezvousEvent, TerminatorEvent,
        TheirSide,
    };

    #[test]
    fn test_mgc() {
        // add_M_essage, _G_otmailbox, then _C_onnected
        let s = MySide::unchecked_from_string(String::from("side1"));
        let phase1 = Phase(String::from("p1"));
        let body1 = b"body1".to_vec();
        let mbox1 = Mailbox(String::from("mbox1"));
        let mut m = MailboxMachine::new(&s);

        let mut e = m.process(AddMessage(phase1.clone(), body1.clone()));
        assert_eq!(e, events![]);
        e = m.process(GotMailbox(mbox1.clone()));
        assert_eq!(e, events![]);

        e = m.process(Connected);
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1.clone(), body1.clone()),
            ]
        );

        // e = m.process(Lost);
        assert_eq!(e, events![]);
        e = m.process(Connected);
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1, body1),
            ]
        );

        e = m.process(Close(Mood::Happy));
        assert_eq!(e, events![RendezvousEvent::TxClose(mbox1, Mood::Happy)]);

        e = m.process(RxClosed);
        assert_eq!(e, events![TerminatorEvent::MailboxDone]);
    }

    #[test]
    fn test_gmc() {
        // _G_otmailbox, add_M_essage, then _C_onnected
        let s = MySide::unchecked_from_string(String::from("side1"));
        let phase1 = Phase(String::from("p1"));
        let body1 = b"body1".to_vec();
        let mbox1 = Mailbox(String::from("mbox1"));
        let mut m = MailboxMachine::new(&s);

        let mut e = m.process(GotMailbox(mbox1.clone()));
        assert_eq!(e, events![]);
        e = m.process(AddMessage(phase1.clone(), body1.clone()));
        assert_eq!(e, events![]);

        e = m.process(Connected);
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1.clone(), body1.clone()),
            ]
        );

        // e = m.process(Lost);
        assert_eq!(e, events![]);
        e = m.process(Connected);
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1, body1),
            ]
        );

        e = m.process(Close(Mood::Happy));
        assert_eq!(e, events![RendezvousEvent::TxClose(mbox1, Mood::Happy)]);

        e = m.process(RxClosed);
        assert_eq!(e, events![TerminatorEvent::MailboxDone]);
    }

    #[test]
    fn test_cmg() {
        // _C_onnected, add_M_essage, then _G_otmailbox
        let s = MySide::unchecked_from_string(String::from("side1"));
        let phase1 = Phase(String::from("p1"));
        let body1 = b"body1".to_vec();
        let mbox1 = Mailbox(String::from("mbox1"));
        let mut m = MailboxMachine::new(&s);

        let mut e = m.process(Connected);
        assert_eq!(e, events![]);
        e = m.process(AddMessage(phase1.clone(), body1.clone()));
        assert_eq!(e, events![]);

        e = m.process(GotMailbox(mbox1.clone()));
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1, body1),
            ]
        );

        e = m.process(Close(Mood::Happy));
        assert_eq!(e, events![RendezvousEvent::TxClose(mbox1, Mood::Happy)]);

        e = m.process(RxClosed);
        assert_eq!(e, events![TerminatorEvent::MailboxDone]);
    }

    #[test]
    fn test_messages() {
        let s = MySide::unchecked_from_string(String::from("side1"));
        let phase1 = Phase(String::from("p1"));
        let body1 = b"body1".to_vec();
        let mbox1 = Mailbox(String::from("mbox1"));
        let mut m = MailboxMachine::new(&s);

        let mut e = m.process(Connected);
        assert_eq!(e, events![]);
        e = m.process(AddMessage(phase1.clone(), body1.clone()));
        assert_eq!(e, events![]);

        e = m.process(GotMailbox(mbox1.clone()));
        assert_eq!(
            e,
            events![
                RendezvousEvent::TxOpen(mbox1.clone()),
                RendezvousEvent::TxAdd(phase1.clone(), body1.clone()),
            ]
        );

        // receiving an echo of our own message is an ack, so we don't need
        // to re-send it after a Lost/Connected cycle. We do re-open the
        // mailbox though.
        let t1 = TheirSide::from(String::from("side1"));
        e = m.process(RxMessage(t1, phase1.clone(), body1.clone()));
        assert_eq!(e, events![]);
        // e = m.process(Lost);
        assert_eq!(e, events![]);
        e = m.process(Connected);
        assert_eq!(e, events![RendezvousEvent::TxOpen(mbox1)]);

        // now a message from the other side means we don't need the
        // Nameplate anymore
        let t2 = TheirSide::from(String::from("side2"));
        e = m.process(RxMessage(t2.clone(), phase1.clone(), body1.clone()));
        assert_eq!(
            e,
            events![
                NameplateEvent::Release,
                OrderEvent::GotMessage(t2.clone(), phase1.clone(), body1.clone()),
            ]
        );

        // receiving a duplicate message should not be forwarded to Order
        e = m.process(RxMessage(t2, phase1, body1));
        assert_eq!(e, events![NameplateEvent::Release]);
    }
}
