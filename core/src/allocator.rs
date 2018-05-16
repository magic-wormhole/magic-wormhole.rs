use events::{Events, Wordlist};
use std::rc::Rc;
use wordlist::default_wordlist;

// we process these
use events::AllocatorEvent::{self, Allocate, Connected, Lost, RxAllocated};
// we emit these
use events::CodeEvent::Allocated as C_Allocated;
use events::RendezvousEvent::TxAllocate as RC_TxAllocate;

pub struct Allocator {
    state: State,
}

#[derive(Debug, PartialEq, Clone)]
enum State {
    S0AIdleDisconnected,
    S0BIdleConnected,
    S1AAllocatingDisconnected(usize, Rc<Wordlist>),
    S1BAllocatingConnected(usize, Rc<Wordlist>),
    S2Done,
}

impl Allocator {
    pub fn new() -> Allocator {
        Allocator {
            state: State::S0AIdleDisconnected,
        }
    }

    pub fn process(&mut self, event: AllocatorEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0AIdleDisconnected => self.do_idle_disconnected(event),
            S0BIdleConnected => self.do_idle_connected(event),
            S1AAllocatingDisconnected(length, ref wordlist) => {
                let wordlist2 = wordlist.clone();
                self.do_allocating_disconnected(event, length, wordlist2)
            }
            S1BAllocatingConnected(length, ref wordlist) => {
                let wordlist2 = wordlist.clone();
                self.do_allocating_connected(event, length, wordlist2)
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
            Allocate(_length, _wordlist) => (
                Some(State::S1AAllocatingDisconnected(
                    _length,
                    _wordlist,
                )),
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
            Allocate(_length, _wordlist) => (
                Some(State::S1BAllocatingConnected(
                    _length,
                    _wordlist,
                )),
                events![RC_TxAllocate],
            ),
            _ => (None, events![]),
        }
    }

    fn do_allocating_disconnected(
        &self,
        event: AllocatorEvent,
        length: usize,
        wordlist: Rc<Wordlist>,
    ) -> (Option<State>, Events) {
        match event {
            Connected => (
                Some(State::S1BAllocatingConnected(length, wordlist)),
                events![RC_TxAllocate],
            ),
            _ => (None, events![]),
        }
    }

    fn do_allocating_connected(
        &self,
        event: AllocatorEvent,
        length: usize,
        wordlist: Rc<Wordlist>,
    ) -> (Option<State>, Events) {
        match event {
            Lost => (
                Some(State::S1AAllocatingDisconnected(
                    length,
                    wordlist,
                )),
                events![],
            ),
            RxAllocated(nameplate) => {
                let _wordlist = default_wordlist(length);
                let words = wordlist.choose_words();
                let code = nameplate.clone() + "-" + &words;
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
    //use super::Allocator;
    //use super::State::*;
}
