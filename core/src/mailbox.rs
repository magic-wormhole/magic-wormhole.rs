use std::collections::HashMap;
use std::collections::HashSet;

use api::Mood;
use events::{Events, MySide};
// we process these
use events::MailboxEvent;
use events::NameplateEvent::Release as N_Release;
use events::OrderEvent::GotMessage as O_GotMessage;
use events::RendezvousEvent::{TxAdd as RC_TxAdd, TxClose as RC_TxClose,
                              TxOpen as RC_TxOpen};
use events::TerminatorEvent::MailboxDone as T_MailboxDone;
// we emit these

#[derive(Debug, PartialEq)]
enum State {
    // S0: We know nothing
    S0A,
    S0B,
    // S1: mailbox known
    S1A(String),
    // S2: mailbox known, maybe open
    S2A(String),
    S2B(String), // opened
    // S3: closing
    S3A(String, Mood), // mailbox, mood
    S3B(String, Mood), // mailbox, mood
    // S4: closed
    S4A,
    S4B,
}

pub struct MailboxMachine {
    state: State,
    side: MySide,
    pending_outbound: HashMap<String, Vec<u8>>, // HashMap<phase, body>
    processed: HashSet<String>,
}

enum QueueCtrl {
    Enqueue(Vec<(String, Vec<u8>)>), // append
    Drain,                           // replace with an empty vec
    NoAction,                        // TODO: find a better name for the field
    AddToProcessed(String),          // add to the list of processed "phase"
    Dequeue(String), // remove an element from the Map given the key
}

impl MailboxMachine {
    pub fn new(side: &MySide) -> MailboxMachine {
        MailboxMachine {
            state: State::S0A,
            side: side.clone(),
            pending_outbound: HashMap::new(),
            processed: HashSet::new(),
        }
    }

    pub fn process(&mut self, event: MailboxEvent) -> Events {
        use self::State::*;

        println!(
            "mailbox: current state = {:?}, got event = {:?}",
            self.state, event
        );

        let (newstate, actions, queue) = match self.state {
            S0A => self.do_s0a(event),
            S0B => self.do_s0b(event),
            S1A(ref mailbox) => self.do_s1a(&mailbox, event),
            S2A(ref mailbox) => self.do_s2a(&mailbox, event),
            S2B(ref mailbox) => self.do_s2b(&mailbox, event),
            S3A(ref mailbox, mood) => self.do_s3a(&mailbox, mood, event),
            S3B(ref mailbox, mood) => self.do_s3b(&mailbox, mood, event),
            S4A => self.do_s4a(event),
            S4B => self.do_s4b(event),
        };
        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        match queue {
            QueueCtrl::Enqueue(v) => for &(ref phase, ref body) in &v {
                self.pending_outbound
                    .insert(phase.to_string(), body.to_vec());
            },
            QueueCtrl::Drain => self.pending_outbound.clear(),
            QueueCtrl::NoAction => (),
            QueueCtrl::AddToProcessed(phase) => {
                self.processed.insert(phase);
            }
            QueueCtrl::Dequeue(phase) => {
                self.pending_outbound.remove(&phase);
            }
        }

        actions
    }

    fn do_s0a(
        &mut self,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (Some(State::S0B), events![], QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(_) => (
                Some(State::S4A),
                events![T_MailboxDone],
                QueueCtrl::NoAction,
            ),
            GotMailbox(mailbox) => (
                Some(State::S1A(mailbox)),
                events![],
                QueueCtrl::NoAction,
            ),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S0A),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s0b(
        &mut self,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S0A), events![], QueueCtrl::NoAction),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(_) => (
                Some(State::S4B),
                events![T_MailboxDone],
                QueueCtrl::NoAction,
            ),
            GotMailbox(mailbox) => {
                // TODO: move this abstraction into a function
                let mut rc_events = events![RC_TxOpen(mailbox.clone())];
                for (ph, body) in self.pending_outbound.iter() {
                    rc_events.push(RC_TxAdd(ph.to_string(), body.to_vec()));
                }
                (
                    Some(State::S2B(mailbox.clone())),
                    rc_events,
                    QueueCtrl::Drain,
                )
            }
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S0B),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s1a(
        &self,
        mailbox: &str,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut rc_events = events![RC_TxOpen(mailbox.to_string())];
                for (ph, body) in self.pending_outbound.iter() {
                    rc_events.push(RC_TxAdd(ph.to_string(), body.to_vec()));
                }
                (
                    Some(State::S2B(mailbox.to_string())),
                    rc_events,
                    QueueCtrl::Drain,
                )
            }
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(_) => (
                Some(State::S4A),
                events![T_MailboxDone],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S1A(mailbox.to_string())),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s2a(
        &self,
        mailbox: &str,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut events = events![RC_TxOpen(mailbox.to_string())];
                for (ph, body) in self.pending_outbound.iter() {
                    events.push(RC_TxAdd(ph.to_string(), body.to_vec()));
                }
                (
                    Some(State::S2B(mailbox.to_string())),
                    events,
                    QueueCtrl::Drain,
                )
            }
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(mood) => (
                Some(State::S3A(mailbox.to_string(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S2A(mailbox.to_string())),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s2b(
        &self,
        mailbox: &str,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (
                Some(State::S2A(mailbox.to_string())),
                events![],
                QueueCtrl::NoAction,
            ),
            RxMessage(side, phase, body) => {
                if side.to_string() != self.side.to_string() {
                    // theirs
                    // N_release_and_accept
                    let is_phase_in_processed = self.processed.contains(&phase);
                    if is_phase_in_processed {
                        (
                            Some(State::S2B(mailbox.to_string())),
                            events![N_Release],
                            QueueCtrl::NoAction,
                        )
                    } else {
                        (
                            Some(State::S2B(mailbox.to_string())),
                            events![
                                N_Release,
                                O_GotMessage(side, phase.clone(), body)
                            ],
                            QueueCtrl::AddToProcessed(phase),
                        )
                    }
                } else {
                    // ours
                    (
                        Some(State::S2B(mailbox.to_string())),
                        events![],
                        QueueCtrl::Dequeue(phase),
                    )
                }
            }
            RxClosed => panic!(),
            Close(mood) => (
                Some(State::S3B(mailbox.to_string(), mood)),
                events![RC_TxClose(mailbox.to_string(), mood)],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                // queue
                let mut v = vec![];
                v.push((phase.clone(), body.clone()));
                // rc_tx_add
                (
                    Some(State::S2B(mailbox.to_string())),
                    events![RC_TxAdd(phase, body)],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s3a(
        &self,
        mailbox: &str,
        mood: Mood,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (
                Some(State::S3B(mailbox.to_string(), mood)),
                events![RC_TxClose(mailbox.to_string(), mood)],
                QueueCtrl::NoAction,
            ),
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(_) => panic!(),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => panic!(),
        }
    }

    fn do_s3b(
        &self,
        mailbox: &str,
        mood: Mood,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (
                Some(State::S3A(mailbox.to_string(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            RxMessage(_side, _phase, _body) => {
                // irrespective of the side, enter into S3B, do nothing, generate no events
                (
                    Some(State::S3B(mailbox.to_string(), mood)),
                    events![],
                    QueueCtrl::NoAction,
                )
            }
            RxClosed => (
                Some(State::S4B),
                events![T_MailboxDone],
                QueueCtrl::NoAction,
            ),
            Close(mood) => (
                Some(State::S3B(mailbox.to_string(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => (
                Some(State::S3B(mailbox.to_string(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
        }
    }

    fn do_s4a(
        &self,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (Some(State::S4B), events![], QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(..) => panic!(),
            GotMailbox(..) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => panic!(),
        }
    }

    fn do_s4b(
        &self,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S4B), events![], QueueCtrl::NoAction),
            RxMessage(_side, _phase, _body) => {
                (Some(State::S4B), events![], QueueCtrl::NoAction)
            }
            RxClosed => panic!(),
            Close(_) => (Some(State::S4B), events![], QueueCtrl::NoAction),
            GotMailbox(..) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => {
                (Some(State::S4B), events![], QueueCtrl::NoAction)
            }
        }
    }
}
