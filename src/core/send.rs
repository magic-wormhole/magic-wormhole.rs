use super::events::{Events, Key, MySide, Phase};
use super::key;
use log::trace;
// we process these
use super::events::SendEvent;
// we emit these
use super::events::MailboxEvent::AddMessage as M_AddMessage;

#[derive(Debug, PartialEq)]
enum State {
    S0NoKey,
    S1HaveVerifiedKey(Key),
}

pub struct SendMachine {
    state: Option<State>,
    side: MySide,
    queue: Vec<(Phase, Vec<u8>)>,
}

impl SendMachine {
    pub fn new(side: &MySide) -> SendMachine {
        SendMachine {
            state: Some(State::S0NoKey),
            side: side.clone(),
            queue: Vec::new(),
        }
    }

    pub fn process(&mut self, event: SendEvent) -> Events {
        trace!(
            "send: current state = {:?}, got event = {:?}",
            self.state,
            event
        );
        use super::events::SendEvent::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            State::S0NoKey => {
                match event {
                    GotVerifiedKey(ref key) => {
                        for (phase, plaintext) in self.queue.drain(..) {
                            let data_key =
                                key::derive_phase_key(&self.side, &key, &phase);
                            let (_nonce, encrypted) =
                                key::encrypt_data(&data_key, &plaintext);
                            actions.push(M_AddMessage(phase, encrypted));
                        }
                        State::S1HaveVerifiedKey(key.clone())
                    }
                    Send(phase, plaintext) => {
                        // we don't have a verified key, yet we got messages to
                        // send, so queue it up.
                        self.queue.push((phase, plaintext));
                        State::S0NoKey
                    }
                }
            }
            State::S1HaveVerifiedKey(ref key) => match event {
                GotVerifiedKey(_) => panic!(),
                Send(phase, plaintext) => {
                    let data_key =
                        key::derive_phase_key(&self.side, &key, &phase);
                    let (_nonce, encrypted) =
                        key::encrypt_data(&data_key, &plaintext);
                    actions.push(M_AddMessage(phase, encrypted));
                    State::S1HaveVerifiedKey(key.clone())
                }
            },
        });

        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::super::events::{Event, MailboxEvent};
    use super::*;

    #[test]
    fn test_queue() {
        let s = MySide("side1".to_string());
        let mut m = SendMachine::new(&s);

        // sending messages before we have a key: messages are queued
        let p1 = Phase("phase1".to_string());
        let plaintext1 = "plaintext1".as_bytes().to_vec();
        let e1 = m.process(SendEvent::Send(p1.clone(), plaintext1));
        assert_eq!(e1.events.len(), 0);

        let p2 = Phase("phase2".to_string());
        let plaintext2 = "plaintext2".as_bytes().to_vec();
        let e2 = m.process(SendEvent::Send(p2.clone(), plaintext2));
        assert_eq!(e2.events.len(), 0);

        // now providing the key should release the encrypted messages to the
        // Mailbox machine
        let key = Key("key".as_bytes().to_vec());
        let mut e3 = m.process(SendEvent::GotVerifiedKey(key));
        assert_eq!(e3.events.len(), 2);

        match e3.events.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(p, _ct1)) => {
                assert_eq!(p, p1);
            }
            _ => panic!(),
        };
        match e3.events.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(p, _ct1)) => {
                assert_eq!(p, p2);
            }
            _ => panic!(),
        };

        // and subsequent Sends should be encrypted immediately

        let p3 = Phase("phase3".to_string());
        let plaintext3 = "plaintext3".as_bytes().to_vec();
        let mut e4 = m.process(SendEvent::Send(p3.clone(), plaintext3));
        assert_eq!(e4.events.len(), 1);
        match e4.events.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(p, _ct1)) => {
                assert_eq!(p, p3);
            }
            _ => panic!(),
        };
    }

    #[test]
    fn test_key_first() {
        let s = MySide("side1".to_string());
        let mut m = SendMachine::new(&s);

        let key = Key("key".as_bytes().to_vec());
        let e1 = m.process(SendEvent::GotVerifiedKey(key));
        assert_eq!(e1.events.len(), 0);

        // subsequent Sends should be encrypted immediately

        let p1 = Phase("phase1".to_string());
        let plaintext1 = "plaintext1".as_bytes().to_vec();
        let mut e2 = m.process(SendEvent::Send(p1.clone(), plaintext1));
        assert_eq!(e2.events.len(), 1);
        match e2.events.remove(0) {
            Event::Mailbox(MailboxEvent::AddMessage(p, _ct1)) => {
                assert_eq!(p, p1);
            }
            _ => panic!(),
        };
    }
}
