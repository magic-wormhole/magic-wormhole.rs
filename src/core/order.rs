use super::events::{Events, Phase, TheirSide};
// we process these
use super::events::OrderEvent;
// we emit these
use super::events::KeyEvent::GotPake as K_GotPake;
use super::events::ReceiveEvent::GotMessage as R_GotMessage;
use log::trace;

#[derive(Debug, PartialEq)]
enum State {
    S0NoPake,
    S1YesPake,
}

pub struct OrderMachine {
    state: Option<State>,
    queue: Vec<(TheirSide, Phase, Vec<u8>)>,
}

impl OrderMachine {
    pub fn new() -> OrderMachine {
        OrderMachine {
            state: Some(State::S0NoPake),
            queue: Vec::new(),
        }
    }

    pub fn process(&mut self, event: OrderEvent) -> Events {
        use self::State::*;
        use OrderEvent::*;

        trace!(
            "order: current state = {:?}, got event = {:?}",
            self.state,
            event
        );

        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0NoPake => match event {
                GotMessage(side, phase, body) => {
                    if phase.is_pake() {
                        // got a pake message
                        actions.push(K_GotPake(body));
                        for &(ref side, ref phase, ref body) in &self.queue {
                            actions.push(R_GotMessage(
                                side.clone(),
                                phase.clone(),
                                body.to_vec(),
                            ));
                        }
                        self.queue = Vec::new(); // todo just empty it
                        S1YesPake
                    } else {
                        // not a  pake message, queue it.
                        self.queue.push((side.clone(), phase, body));
                        S0NoPake
                    }
                }
            },
            S1YesPake => match event {
                GotMessage(side, phase, body) => {
                    actions.push(R_GotMessage(side.clone(), phase, body));
                    State::S1YesPake
                }
            },
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::*;
    use crate::core::events::{Phase, TheirSide};

    #[test]
    fn test_messages_then_pake() {
        let mut m = super::OrderMachine::new();
        let s1: TheirSide = TheirSide::from(String::from("side1"));
        let p1: Phase = Phase(String::from("phase1"));
        let p2: Phase = Phase(String::from("phase2"));
        let p3: Phase = Phase(String::from("phase3"));
        let ppake: Phase = Phase(String::from("pake"));

        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            p1.clone(),
            b"body1".to_vec(),
        ));
        assert_eq!(out, events![]);
        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            p2.clone(),
            b"body2".to_vec(),
        ));
        assert_eq!(out, events![]);
        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            ppake.clone(),
            b"pake".to_vec(),
        ));
        assert_eq!(
            out,
            events![
                K_GotPake(b"pake".to_vec()),
                R_GotMessage(s1.clone(), p1.clone(), b"body1".to_vec()),
                R_GotMessage(s1.clone(), p2.clone(), b"body2".to_vec()),
            ]
        );
        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            p3.clone(),
            b"body3".to_vec(),
        ));
        assert_eq!(
            out,
            events![R_GotMessage(s1.clone(), p3.clone(), b"body3".to_vec()),]
        );
    }

    #[test]
    fn test_pake_then_messages() {
        let mut m = super::OrderMachine::new();
        let s1: TheirSide = TheirSide::from(String::from("side1"));
        let p1: Phase = Phase(String::from("phase1"));
        let p2: Phase = Phase(String::from("phase2"));
        let _p3: Phase = Phase(String::from("phase3"));
        let ppake: Phase = Phase(String::from("pake"));

        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            ppake.clone(),
            b"pake".to_vec(),
        ));
        assert_eq!(out, events![K_GotPake(b"pake".to_vec()),]);
        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            p1.clone(),
            b"body1".to_vec(),
        ));
        assert_eq!(
            out,
            events![R_GotMessage(s1.clone(), p1.clone(), b"body1".to_vec()),]
        );
        let out = m.process(OrderEvent::GotMessage(
            s1.clone(),
            p2.clone(),
            b"body2".to_vec(),
        ));
        assert_eq!(
            out,
            events![R_GotMessage(s1.clone(), p2.clone(), b"body2".to_vec()),]
        );
    }
}
