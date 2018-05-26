use events::Events;
// we process these
use events::OrderEvent;
// we emit these
use events::KeyEvent::GotPake as K_GotPake;
use events::ReceiveEvent::GotMessage as R_GotMessage;

#[derive(Debug, PartialEq)]
enum State {
    S0, //no pake
    S1, //yes pake
}

pub struct OrderMachine {
    state: State,
    queue: Vec<(String, String, Vec<u8>)>,
}

enum QueueStatus {
    Enqueue((String, String, Vec<u8>)),
    Drain,
    NoAction,
}

impl OrderMachine {
    pub fn new() -> OrderMachine {
        OrderMachine {
            state: State::S0,
            queue: Vec::new(),
        }
    }

    pub fn process(&mut self, event: OrderEvent) -> Events {
        use self::State::*;

        println!(
            "order: current state = {:?}, got event = {:?}",
            self.state, event
        );

        let (newstate, actions, queue_status) = match self.state {
            S0 => self.do_s0(event),
            S1 => self.do_s1(event),
        };

        self.state = newstate;

        match queue_status {
            QueueStatus::Enqueue(tup) => self.queue.push(tup),
            QueueStatus::Drain => {
                self.queue = Vec::new();
            }
            QueueStatus::NoAction => (),
        };

        actions
    }

    fn drain(&self) -> Events {
        let mut es = Events::new();

        for &(ref side, ref phase, ref body) in &self.queue {
            es.push(R_GotMessage(
                side.to_string(),
                phase.to_string(),
                body.to_vec(),
            ));
        }

        es
    }

    fn do_s0(&self, event: OrderEvent) -> (State, Events, QueueStatus) {
        use events::OrderEvent::*;
        match event {
            GotMessage(side, phase, body) => {
                if phase == "pake" {
                    // got a pake message
                    let mut es = self.drain();
                    let mut key_events = events![K_GotPake(body)];
                    key_events.append(&mut es);
                    (State::S1, key_events, QueueStatus::Drain)
                } else {
                    // not a  pake message, queue it.
                    (
                        State::S0,
                        events![],
                        QueueStatus::Enqueue((side, phase, body)),
                    )
                }
            }
        }
    }

    fn do_s1(&self, event: OrderEvent) -> (State, Events, QueueStatus) {
        use events::OrderEvent::*;
        match event {
            GotMessage(side, phase, body) => (
                State::S1,
                events![R_GotMessage(side, phase, body)],
                QueueStatus::NoAction,
            ),
        }
    }
}
