use events::Events;
use key::Key;
// we process these
use events::SendEvent;
// we emit these
use events::MailboxEvent::AddMessage as M_AddMessage;

pub struct Send {
    state: State,
    side: String,
    key: Vec<u8>,
    queue: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, PartialEq)]
enum State {
    S0,
    S1(Vec<u8>),
}

enum QueueStatus {
    Enqueue((String, Vec<u8>)),
    Drain,
    NoAction,
}

impl Send {
    pub fn new(side: &str) -> Send {
        Send {
            state: State::S0,
            side: side.to_string(),
            key: Vec::new(),
            queue: Vec::new(),
        }
    }

    pub fn process(&mut self, event: SendEvent) -> Events {
        use events::SendEvent::*;

        println!(
            "send: current state = {:?}, got event = {:?}",
            self.state, event
        );
        let (newstate, actions, queue_status) = match self.state {
            State::S0 => self.do_S0(event),
            State::S1(ref key) => self.do_S1(key.to_vec(), event),
        };

        // process the queue
        match queue_status {
            QueueStatus::Enqueue(tup) => self.queue.push(tup),
            QueueStatus::Drain => {
                self.queue = Vec::new();
            }
            QueueStatus::NoAction => (),
        };

        self.state = newstate;
        actions
    }

    fn drain(&self, key: Vec<u8>) -> Events {
        let mut es = Events::new();

        for &(ref phase, ref plaintext) in &self.queue {
            let data_key = Key::derive_phase_key(&self.side, &key, phase);
            let (nonce, encrypted) = Key::encrypt_data(data_key, plaintext);
            es.push(M_AddMessage(phase.to_string(), encrypted));
        }

        es
    }

    fn deliver(
        &self,
        key: Vec<u8>,
        phase: String,
        plaintext: Vec<u8>,
    ) -> Events {
        let data_key = Key::derive_phase_key(&self.side, &key, &phase);
        let (nonce, encrypted) = Key::encrypt_data(data_key, &plaintext);
        events![M_AddMessage(phase, encrypted)]
    }

    fn do_S0(&self, event: SendEvent) -> (State, Events, QueueStatus) {
        use events::SendEvent::*;
        match event {
            GotVerifiedKey(ref key) => (
                State::S1(key.to_vec()),
                self.drain(key.to_vec()),
                QueueStatus::Drain,
            ),
            // we don't have a verified key, yet we got messages to send, so queue it up.
            Send(phase, plaintext) => (
                State::S0,
                events![],
                QueueStatus::Enqueue((phase, plaintext)),
            ),
        }
    }

    fn do_S1(
        &self,
        key: Vec<u8>,
        event: SendEvent,
    ) -> (State, Events, QueueStatus) {
        use events::SendEvent::*;
        match event {
            GotVerifiedKey(_) => panic!(),
            Send(phase, plaintext) => {
                let deliver_events =
                    self.deliver(key.clone(), phase, plaintext);
                (
                    State::S1(key),
                    deliver_events,
                    QueueStatus::NoAction,
                )
            }
        }
    }
}
