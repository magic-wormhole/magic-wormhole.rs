use std::collections::HashMap;
use std::collections::HashSet;

use api::Mood;
use events::{Events, Mailbox, MySide, Phase};
// we process these
use events::MailboxEvent;
use events::NameplateEvent::Release as N_Release;
use events::OrderEvent::GotMessage as O_GotMessage;
use events::RendezvousEvent::{
    TxAdd as RC_TxAdd, TxClose as RC_TxClose, TxOpen as RC_TxOpen,
};
use events::TerminatorEvent::MailboxDone as T_MailboxDone;
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
    state: State,
    side: MySide,
    pending_outbound: HashMap<Phase, Vec<u8>>, // HashMap<phase, body>
    processed: HashSet<Phase>,
}

enum QueueCtrl {
    Enqueue(Vec<(Phase, Vec<u8>)>), // append
    Drain,                          // replace with an empty vec
    NoAction,                       // TODO: find a better name for the field
    AddToProcessed(Phase),          // add to the list of processed "phase"
    Dequeue(Phase), // remove an element from the Map given the key
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
            S3A(ref mailbox, mood) => self.do_s3a(&mailbox, mood, &event),
            S3B(ref mailbox, mood) => self.do_s3b(&mailbox, mood, event),
            S4A => self.do_s4a(&event),
            S4B => self.do_s4b(event),
        };

        if let Some(s) = newstate {
            self.state = s;
        }

        match queue {
            QueueCtrl::Enqueue(v) => for &(ref phase, ref body) in &v {
                self.pending_outbound.insert(phase.clone(), body.to_vec());
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
            GotMailbox(mailbox) => {
                (Some(State::S1A(mailbox)), events![], QueueCtrl::NoAction)
            }
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S0A), events![], QueueCtrl::Enqueue(v))
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
                for (ph, body) in &self.pending_outbound {
                    rc_events.push(RC_TxAdd(ph.clone(), body.to_vec()));
                }
                (
                    Some(State::S2B(mailbox.clone())),
                    rc_events,
                    QueueCtrl::Drain,
                )
            }
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S0B), events![], QueueCtrl::Enqueue(v))
            }
        }
    }

    fn do_s1a(
        &self,
        mailbox: &Mailbox,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut rc_events = events![RC_TxOpen(mailbox.clone())];
                for (ph, body) in &self.pending_outbound {
                    rc_events.push(RC_TxAdd(ph.clone(), body.to_vec()));
                }
                (
                    Some(State::S2B(mailbox.clone())),
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
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S1A(mailbox.clone())),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s2a(
        &self,
        mailbox: &Mailbox,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut events = events![RC_TxOpen(mailbox.clone())];
                for (ph, body) in &self.pending_outbound {
                    events.push(RC_TxAdd(ph.clone(), body.to_vec()));
                }
                (Some(State::S2B(mailbox.clone())), events, QueueCtrl::Drain)
            }
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(mood) => (
                Some(State::S3A(mailbox.clone(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (
                    Some(State::S2A(mailbox.clone())),
                    events![],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s2b(
        &self,
        mailbox: &Mailbox,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (
                Some(State::S2A(mailbox.clone())),
                events![],
                QueueCtrl::NoAction,
            ),
            RxMessage(side, phase, body) => {
                if *side != *self.side {
                    // theirs
                    // N_release_and_accept
                    let is_phase_in_processed = self.processed.contains(&phase);
                    if is_phase_in_processed {
                        (
                            Some(State::S2B(mailbox.clone())),
                            events![N_Release],
                            QueueCtrl::NoAction,
                        )
                    } else {
                        (
                            Some(State::S2B(mailbox.clone())),
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
                        Some(State::S2B(mailbox.clone())),
                        events![],
                        QueueCtrl::Dequeue(phase),
                    )
                }
            }
            RxClosed => panic!(),
            Close(mood) => (
                Some(State::S3B(mailbox.clone(), mood)),
                events![RC_TxClose(mailbox.clone(), mood)],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            AddMessage(phase, body) => {
                // queue
                let mut v = vec![];
                v.push((phase.clone(), body.clone()));
                // rc_tx_add
                (
                    Some(State::S2B(mailbox.clone())),
                    events![RC_TxAdd(phase, body)],
                    QueueCtrl::Enqueue(v),
                )
            }
        }
    }

    fn do_s3a(
        &self,
        mailbox: &Mailbox,
        mood: Mood,
        event: &MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match *event {
            Connected => (
                Some(State::S3B(mailbox.clone(), mood)),
                events![RC_TxClose(mailbox.clone(), mood)],
                QueueCtrl::NoAction,
            ),
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(_) => panic!(),
            GotMailbox(_) => panic!(),
            AddMessage(_, _) => panic!(),
        }
    }

    fn do_s3b(
        &self,
        mailbox: &Mailbox,
        mood: Mood,
        event: MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (
                Some(State::S3A(mailbox.clone(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            RxMessage(_side, _phase, _body) => {
                // irrespective of the side, enter into S3B, do nothing, generate no events
                (
                    Some(State::S3B(mailbox.clone(), mood)),
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
                Some(State::S3B(mailbox.clone(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
            GotMailbox(_) => panic!(),
            AddMessage(_, _) => (
                Some(State::S3B(mailbox.clone(), mood)),
                events![],
                QueueCtrl::NoAction,
            ),
        }
    }

    fn do_s4a(
        &self,
        event: &MailboxEvent,
    ) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (Some(State::S4B), events![], QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage(_, _, _) => panic!(),
            RxClosed => panic!(),
            Close(..) => panic!(),
            GotMailbox(..) => panic!(),
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
            AddMessage(_, _) => {
                (Some(State::S4B), events![], QueueCtrl::NoAction)
            }
        }
    }
}
