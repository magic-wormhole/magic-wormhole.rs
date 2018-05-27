use events::{Code, Events, Wordlist};
use std::sync::Arc;

// we process these
use events::AllocatorEvent::{self, Allocate, Connected, Lost, RxAllocated};
// we emit these
use events::CodeEvent::Allocated as C_Allocated;
use events::RendezvousEvent::TxAllocate as RC_TxAllocate;

pub struct AllocatorMachine {
    state: State,
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
            state: State::S0AIdleDisconnected,
        }
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0AIdleDisconnected => self.do_idle_disconnected(event),
            S0BIdleConnected => self.do_idle_connected(event),
            S1AAllocatingDisconnected(ref wordlist) => {
                self.do_allocating_disconnected(event, wordlist.clone())
            }
            S1BAllocatingConnected(ref wordlist) => {
                self.do_allocating_connected(event, wordlist.clone())
            }
            S2Done => (None, events![]),
        };

        if let Some(s) = newstate {
            self.state = s;
        }
        actions
    }

    fn do_idle_disconnected(
        &self,
        event: AllocatorEvent,
    ) -> (Option<State>, Events) {
        match event {
            Connected => (Some(State::S0BIdleConnected), events![]),
            Allocate(wordlist) => (
                Some(State::S1AAllocatingDisconnected(wordlist)),
                events![],
            ),
            _ => (None, events![]),
        }
    }

    fn do_idle_connected(
        &self,
        event: AllocatorEvent,
    ) -> (Option<State>, Events) {
        match event {
            Lost => (Some(State::S0AIdleDisconnected), events![]),
            Allocate(wordlist) => (
                Some(State::S1BAllocatingConnected(wordlist)),
                events![RC_TxAllocate],
            ),
            _ => (None, events![]),
        }
    }

    fn do_allocating_disconnected(
        &self,
        event: AllocatorEvent,
        wordlist: Arc<Wordlist>,
    ) -> (Option<State>, Events) {
        match event {
            Connected => (
                Some(State::S1BAllocatingConnected(wordlist)),
                events![RC_TxAllocate],
            ),
            _ => (None, events![]),
        }
    }

    fn do_allocating_connected(
        &self,
        event: AllocatorEvent,
        wordlist: Arc<Wordlist>,
    ) -> (Option<State>, Events) {
        match event {
            Lost => (
                Some(State::S1AAllocatingDisconnected(wordlist)),
                events![],
            ),
            RxAllocated(nameplate) => {
                let words = wordlist.choose_words();
                let code = Code(nameplate.to_string() + "-" + &words);
                (
                    Some(State::S2Done),
                    events![C_Allocated(nameplate, code)],
                )
            }
            _ => (None, events![]),
        }
    }
}

#[cfg(test)]
mod test {
    //use super::AllocatorMachine;
    //use super::State::*;
}
