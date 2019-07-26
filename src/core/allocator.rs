use super::events::{Code, Events, Wordlist};
use std::sync::Arc;

// we process these
use super::events::AllocatorEvent;

// we emit these
use super::events::CodeEvent::Allocated as C_Allocated;
use super::events::RendezvousEvent::TxAllocate as RC_TxAllocate;

pub struct AllocatorMachine {
    state: Option<State>,
}

#[derive(Debug, PartialEq, Clone)]
enum State {
    S0AIdleDisconnected,
    S0BIdleConnected,
    S1AAllocatingDisconnected(Arc<Wordlist>),
    S1BAllocatingConnected(Arc<Wordlist>),
    S2Done,
}

impl AllocatorMachine {
    pub fn new() -> AllocatorMachine {
        AllocatorMachine {
            state: Some(State::S0AIdleDisconnected),
        }
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        use AllocatorEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            S0AIdleDisconnected => match event {
                Connected => S0BIdleConnected,
                Allocate(wordlist) => S1AAllocatingDisconnected(wordlist),
                _ => panic!(),
            },
            S0BIdleConnected => match event {
                Lost => S0AIdleDisconnected,
                Allocate(wordlist) => {
                    actions.push(RC_TxAllocate);
                    S1BAllocatingConnected(wordlist)
                }
                _ => panic!(),
            },
            S1AAllocatingDisconnected(wordlist) => match event {
                Connected => {
                    actions.push(RC_TxAllocate);
                    S1BAllocatingConnected(wordlist)
                }
                _ => panic!(),
            },
            S1BAllocatingConnected(wordlist) => match event {
                Lost => S1AAllocatingDisconnected(wordlist),
                RxAllocated(nameplate) => {
                    let words = wordlist.choose_words();
                    let code = Code(nameplate.to_string() + "-" + &words);
                    actions.push(C_Allocated(nameplate, code));
                    S2Done
                }
                _ => panic!(),
            },
            S2Done => old_state,
        });
        actions
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::super::events::{CodeEvent, Event, Nameplate, RendezvousEvent};
    use super::super::wordlist::default_wordlist;
    use super::AllocatorEvent::*;
    use super::AllocatorMachine;
    use std::sync::Arc;

    #[test]
    fn test_acr() {
        // start Allocation, then Connect, then Rx the nameplate
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();

        assert_eq!(a.process(Allocate(w)).events.len(), 0);
        let mut e = a.process(Connected);
        match e.events.remove(0) {
            Event::Rendezvous(RendezvousEvent::TxAllocate) => (),
            _ => panic!(),
        }
        assert_eq!(e.events.len(), 0);
        let n = Nameplate::new("123");
        e = a.process(RxAllocated(n));
        match e.events.remove(0) {
            Event::Code(CodeEvent::Allocated(nameplate, _code)) => {
                assert_eq!(nameplate.to_string(), "123");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_aclcr() {
        // start Allocation, Connect, Lose the connection, re-Connect, Rx
        // nameplate
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();

        assert_eq!(a.process(Allocate(w)).events.len(), 0);
        let mut e = a.process(Connected);
        match e.events.remove(0) {
            Event::Rendezvous(RendezvousEvent::TxAllocate) => (),
            _ => panic!(),
        }
        assert_eq!(e.events.len(), 0);

        assert_eq!(a.process(Lost).events.len(), 0);
        e = a.process(Connected);
        match e.events.remove(0) {
            Event::Rendezvous(RendezvousEvent::TxAllocate) => (),
            _ => panic!(),
        }
        assert_eq!(e.events.len(), 0);

        let n = Nameplate::new("123");
        e = a.process(RxAllocated(n));
        match e.events.remove(0) {
            Event::Code(CodeEvent::Allocated(nameplate, _code)) => {
                assert_eq!(nameplate.to_string(), "123");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_car() {
        // Connect first, then start Allocation, then Rx the nameplate
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();

        assert_eq!(a.process(Connected).events.len(), 0);
        let mut e = a.process(Allocate(w));
        match e.events.remove(0) {
            Event::Rendezvous(RendezvousEvent::TxAllocate) => (),
            _ => panic!(),
        }
        assert_eq!(e.events.len(), 0);
        let n = Nameplate::new("123");
        e = a.process(RxAllocated(n));
        match e.events.remove(0) {
            Event::Code(CodeEvent::Allocated(nameplate, _code)) => {
                assert_eq!(nameplate.to_string(), "123");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_clacr() {
        // Connect, Lose connection, then start Allocation, re-Connect, then
        // Rx the nameplate
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();

        assert_eq!(a.process(Connected).events.len(), 0);
        assert_eq!(a.process(Lost).events.len(), 0);
        assert_eq!(a.process(Allocate(w)).events.len(), 0);
        let mut e = a.process(Connected);
        match e.events.remove(0) {
            Event::Rendezvous(RendezvousEvent::TxAllocate) => (),
            _ => panic!(),
        }
        assert_eq!(e.events.len(), 0);
        let n = Nameplate::new("123");
        e = a.process(RxAllocated(n));
        match e.events.remove(0) {
            Event::Code(CodeEvent::Allocated(nameplate, _code)) => {
                assert_eq!(nameplate.to_string(), "123");
            }
            _ => panic!(),
        }
    }

    #[test]
    #[should_panic]
    fn test_aa_panic() {
        // duplicate allocation should panic
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();
        a.process(Allocate(w.clone()));
        a.process(Allocate(w));
    }

    #[test]
    #[should_panic]
    fn test_cc_panic() {
        let mut a = AllocatorMachine::new();
        a.process(Connected);
        a.process(Connected);
    }

    #[test]
    #[should_panic]
    fn test_l_panic() {
        let mut a = AllocatorMachine::new();
        a.process(Lost);
    }

    #[test]
    fn test_acrr() {
        // a duplicate receive is ignored
        let w = Arc::new(default_wordlist(2));
        let mut a = AllocatorMachine::new();

        a.process(Allocate(w));
        a.process(Connected);
        let n1 = Nameplate::new("123");
        a.process(RxAllocated(n1));
        let n2 = Nameplate::new("123");
        a.process(RxAllocated(n2));
    }

}
