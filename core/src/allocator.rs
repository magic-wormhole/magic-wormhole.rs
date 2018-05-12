use events::{Events, Wordlist};
use wordlist::PGPWordlist;

// we process these
use events::AllocatorEvent::{self, Allocate, Connected, Lost, RxAllocated};
// we emit these
use events::CodeEvent::Allocated as C_Allocated;
use events::RendezvousEvent::TxAllocate as RC_TxAllocate;

pub struct Allocator {
    state: State,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    S0A_idle_disconnected,
    S0B_idle_connected,
    S1A_allocating_disconnected(u8, Wordlist),
    S1B_allocating_connected(u8, Wordlist),
    S2_done,
}

impl Allocator {
    pub fn new() -> Allocator {
        Allocator {
            state: State::S0A_idle_disconnected,
        }
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0A_idle_disconnected => self.do_idle_disconnected(event),
            S0B_idle_connected => self.do_idle_connected(event),
            S1A_allocating_disconnected(..) => {
                self.do_allocating_disconnected(event)
            }
            S1B_allocating_connected(..) => self.do_allocating_connected(event),
            S2_done => (self.state, events![]),
        };

        self.state = newstate;
        actions
    }

    fn do_idle_disconnected(
        &mut self,
        event: AllocatorEvent,
    ) -> (State, Events) {
        match event {
            Connected => (State::S0B_idle_connected, events![]),
            Allocate(_length, _wordlist) => (
                State::S1A_allocating_disconnected(_length, _wordlist),
                events![],
            ),
            _ => (self.state, events![]),
        }
    }

    fn do_idle_connected(&mut self, event: AllocatorEvent) -> (State, Events) {
        match event {
            Lost => (State::S0A_idle_disconnected, events![]),
            Allocate(_length, _wordlist) => (
                State::S1B_allocating_connected(_length, _wordlist),
                events![RC_TxAllocate],
            ),
            _ => (self.state, events![]),
        }
    }

    fn do_allocating_disconnected(
        &self,
        event: AllocatorEvent,
    ) -> (State, Events) {
        match event {
            Connected => {
                if let State::S1A_allocating_disconnected(_length, _wordlist) =
                    self.state
                {
                    (
                        State::S1B_allocating_connected(_length, _wordlist),
                        events![RC_TxAllocate],
                    )
                } else {
                    panic!();
                }
            }
            _ => (self.state, events![]),
        }
    }

    fn do_allocating_connected(
        &mut self,
        event: AllocatorEvent,
    ) -> (State, Events) {
        match event {
            Lost => {
                if let State::S1B_allocating_connected(_length, _wordlist) =
                    self.state
                {
                    (
                        State::S1A_allocating_disconnected(_length, _wordlist),
                        events![],
                    )
                } else {
                    panic!();
                }
            }
            RxAllocated(nameplate) => {
                let _wordlist = PGPWordlist::new();
                if let State::S1B_allocating_connected(_length, _) = self.state
                {
                    let word = _wordlist.choose_words(_length);
                    let code = nameplate.clone() + "-" + &word;
                    (
                        State::S2_done,
                        events![C_Allocated(nameplate, code)],
                    )
                } else {
                    // TODO: This should not happen but if happens we need proper error.
                    panic!()
                }
            }
            _ => (self.state, events![]),
        }
    }
}

#[cfg(test)]
mod test {
    use super::Allocator;
    use super::State::*;
}
