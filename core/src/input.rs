use events::Events;
// we process these
use events::InputEvent::{self, ChooseNameplate, ChooseWords, GotNameplates,
                         GotWordlist, RefreshNameplates, Start};
// we emit these
use events::CodeEvent::{FinishedInput as C_FinishedInput,
                        GotNameplate as C_GotNameplate};
use events::InputHelperEvent::{GotNameplates as IH_GotNameplates,
                               GotWordlist as IH_GotWordlist};
use events::ListerEvent::Refresh as L_Refresh;

pub struct Input {
    state: State,
    _nameplate: String,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    S0Idle,
    S1TypingNameplate,
    S2TypingCodeWithoutWordlist,
    S3TypingCodeWithWordlist,
    S4Done,
}

impl Input {
    pub fn new() -> Input {
        Input {
            state: State::S0Idle,
            _nameplate: String::new(),
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0Idle => self.in_idle(event),
            S1TypingNameplate => self.in_typing_nameplate(event),
            S2TypingCodeWithoutWordlist => self.in_type_without_wordlist(event),
            S3TypingCodeWithWordlist => self.in_type_with_wordlist(event),
            State::S4Done => (self.state, events![]),
        };

        self.state = newstate;
        actions
    }

    fn in_idle(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            Start => (State::S1TypingNameplate, events![L_Refresh]),
            _ => (self.state, events![]),
        }
    }

    fn in_typing_nameplate(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            GotNameplates(nameplates) => (
                State::S1TypingNameplate,
                events![IH_GotNameplates(nameplates)],
            ),
            ChooseNameplate(nameplate) => {
                self._nameplate = nameplate.to_owned();
                (
                    State::S2TypingCodeWithoutWordlist,
                    events![C_GotNameplate(nameplate)],
                )
            }
            RefreshNameplates => (self.state, events![L_Refresh]),
            _ => (self.state, events![]),
        }
    }

    fn in_type_without_wordlist(
        &mut self,
        event: InputEvent,
    ) -> (State, Events) {
        match event {
            GotNameplates(nameplates) => (
                State::S2TypingCodeWithoutWordlist,
                events![IH_GotNameplates(nameplates)],
            ),
            GotWordlist(wordlist) => (
                State::S3TypingCodeWithWordlist,
                events![IH_GotWordlist(wordlist)],
            ),
            ChooseWords(words) => {
                let code = format!("{}-{}", self._nameplate, words);
                (State::S4Done, events![C_FinishedInput(code)])
            }
            _ => (self.state, events![]),
        }
    }

    fn in_type_with_wordlist(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            GotNameplates(nameplates) => (
                self.state,
                events![IH_GotNameplates(nameplates)],
            ),
            ChooseWords(words) => {
                let code = format!{"{}-{}", self._nameplate, words};
                (State::S4Done, events![C_FinishedInput(code)])
            }
            _ => (self.state, events![]),
        }
    }
}
