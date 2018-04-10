use events::Events;
use events::Event;
// we process these
use events::MailboxEvent;
use events::TerminatorEvent::MailboxDone as T_MailboxDone;
use events::RendezvousEvent::TxOpen as RC_TxOpen;
use events::RendezvousEvent::TxAdd as RC_TxAdd;
use events::RendezvousEvent::TxClose as RC_TxClose;
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
    S3A(String, String), // mailbox, mood
    S3B(String, String), // mailbox, mood
    // S4: closed
    S4A,
    S4B,
}

pub struct Mailbox {
    state: State,
    pending_outbound: Vec<(String, String)> // vector of pairs (phase, body)
}

enum QueueCtrl {
    Enqueue(Vec<(String, String)>), // append
    Drain,               // replace with an empty vec
    NoAction             // TODO: find a better name for the field
}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox { state: State::S0A, pending_outbound: vec![] }
    }

    pub fn process(&mut self, event: MailboxEvent) -> Events {
        use self::State::*;
        
        let (newstate, actions, queue) = match self.state {
            S0A => self.do_S0A(event),
            S0B => self.do_S0B(event),
            S1A(ref mailbox) => self.do_S1A(&mailbox, event),
            S2A(ref mailbox) => self.do_S2A(&mailbox, event),
            S2B(ref mailbox) => self.do_S2B(&mailbox, event),
            S3A(ref mailbox, ref mood) => self.do_S3A(&mailbox, &mood, event),
            S3B(ref mailbox, ref mood) => self.do_S3B(&mailbox, &mood, event),
            S4A => self.do_S4A(event),
            S4B => self.do_S4B(event),
            _ => panic!()
        };
        match newstate {
            Some(s) => {
                self.state = s;
            }
            None => {}
        }
        match queue {
            QueueCtrl::Enqueue(mut v) => {
                // append pending_outbound with v
                self.pending_outbound.append(&mut v)
            },
            QueueCtrl::Drain => self.pending_outbound = vec![],
            QueueCtrl::NoAction => (),
        }

        actions
    }

    fn do_S0A(&mut self, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (Some(State::S0B), events![], QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(_) => (Some(State::S4A), events![T_MailboxDone], QueueCtrl::NoAction),
            GotMailbox(mailbox) => (Some(State::S1A(mailbox)), events![], QueueCtrl::NoAction),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S0A), events![], QueueCtrl::Enqueue(v))
            },
        }
    }

    fn do_S0B(&mut self, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S0A), events![], QueueCtrl::NoAction),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(_) => (Some(State::S4B), events![T_MailboxDone], QueueCtrl::NoAction),
            GotMailbox(mailbox) => {
                let mut rc_events = events![RC_TxOpen(mailbox.clone())];
                for &(ref ph, ref body) in &self.pending_outbound {
                    rc_events.push(RC_TxAdd(ph.to_string(), body.to_string()));
                }
                (Some(State::S2B(mailbox.clone())), rc_events, QueueCtrl::Drain)
            },
            GotMessage =>  panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S0B), events![], QueueCtrl::Enqueue(v))
            },
        }
    }

    fn do_S1A(&self, mailbox: &str, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut rc_events = events![RC_TxOpen(mailbox.to_string())];
                for &(ref ph, ref body) in &self.pending_outbound {
                    rc_events.push(RC_TxAdd(ph.to_string(), body.to_string()));
                }
                (Some(State::S2B(mailbox.to_string())), rc_events, QueueCtrl::Drain)
            },
            Lost => panic!(),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(_) => (Some(State::S4A), events![T_MailboxDone], QueueCtrl::NoAction),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S1A(mailbox.to_string())), events![], QueueCtrl::Enqueue(v))
            },
        }
    }

    fn do_S2A(&self, mailbox: &str, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => {
                let mut events = events![RC_TxOpen(mailbox.to_string())];
                for &(ref ph, ref body) in &self.pending_outbound {
                    events.push(RC_TxAdd(ph.to_string(), body.to_string()));
                }
                (Some(State::S2B(mailbox.to_string())), events, QueueCtrl::Drain)
            },
            Lost => panic!(),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(mood) => (Some(State::S3A(mailbox.to_string(), mood)), events![], QueueCtrl::NoAction),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                let mut v = vec![];
                v.push((phase, body));
                (Some(State::S2A(mailbox.to_string())), events![], QueueCtrl::Enqueue(v))
            },
        }
    }

    fn do_S2B(&self, mailbox: &str, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S2A(mailbox.to_string())), events![], QueueCtrl::NoAction),
            RxMessage => panic!(), // TODO, handle theirs vs ours
            RxClosed => panic!(),
            Close(mood) => (Some(State::S3B(mailbox.to_string(),
                                            mood.to_string())),
                            events![RC_TxClose],
                            QueueCtrl::NoAction),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(phase, body) => {
                // queue
                let mut v = vec![];
                v.push((phase.clone(), body.clone()));
                // rc_tx_add
                (Some(State::S2B(mailbox.to_string())), events![RC_TxAdd(phase, body)], QueueCtrl::Enqueue(v))
            }
        }
    }
    
    fn do_S3A(&self, mailbox: &str, mood: &str,
              event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;
        
        match event {
            Connected => (Some(State::S3B(mailbox.to_string(),
                                          mood.to_string())),
                          events![RC_TxClose],
                          QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(_) => panic!(),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => panic!(),
        }
    }

    fn do_S3B(&self, mailbox: &str, mood: &str,
              event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S3A(mailbox.to_string(),
                                     mood.to_string())),
                     events![],
                     QueueCtrl::NoAction),
            RxMessage => panic!(), // TODO
            RxClosed => (Some(State::S4B), events![T_MailboxDone], QueueCtrl::NoAction),
            Close(mood) => (Some(State::S3B(mailbox.to_string(),
                                            mood.to_string())),
                            events![],
                            QueueCtrl::NoAction),
            GotMailbox(_) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => (Some(State::S3B(mailbox.to_string(),
                                                 mood.to_string())),
                                 events![],
                                 QueueCtrl::NoAction)
        }
    }

    fn do_S4A(&self, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => (Some(State::S4B), events![], QueueCtrl::NoAction),
            Lost => panic!(),
            RxMessage => panic!(),
            RxClosed => panic!(),
            Close(String) => panic!(),
            GotMailbox(String) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => panic!(),
        }
    }

    fn do_S4B(&self, event: MailboxEvent) -> (Option<State>, Events, QueueCtrl) {
        use events::MailboxEvent::*;

        match event {
            Connected => panic!(),
            Lost => (Some(State::S4B), events![], QueueCtrl::NoAction),
            RxMessage => panic!(), // TODO
            RxClosed => panic!(),
            Close(_) => (Some(State::S4B), events![], QueueCtrl::NoAction),
            GotMailbox(String) => panic!(),
            GotMessage => panic!(),
            AddMessage(_, _) => (Some(State::S4B), events![], QueueCtrl::NoAction)
        }
    }
}
