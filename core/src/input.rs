use events::Events;
// we process these
use events::InputEvent::{self, ChooseNameplate, ChooseWords, GotNameplates,
                         GotWordlist, RefreshNameplates, Start};
// we emit these
use events::ListerEvent::Refresh as L_Refresh;
use events::CodeEvent::{FinishedInput as C_FinishedInput,
                        GotNameplate as C_GotNameplate};
use events::InputHelperEvent::{GotNameplates as IH_GotNameplates,
                               GotWordlist as IH_GotWordlist};

pub struct Input {
    state: State,
    _nameplate: String,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    S0_Idle,
    S1_typing_nameplate,
    S2_typing_code_without_wordlist,
    S3_typing_code_with_wordlist,
    S4_done,
}

impl Input {
    pub fn new() -> Input {
        Input {
            state: State::S0_Idle,
            _nameplate: String::new(),
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            S0_Idle => self.in_idle(event),
            S1_typing_nameplate => self.in_typing_nameplate(event),
            S2_typing_code_without_wordlist => {
                self.in_type_without_wordlist(event)
            }
            S3_typing_code_with_wordlist => self.in_type_with_wordlist(event),
            State::S4_done => (self.state, events![]),
        };

        self.state = newstate;
        actions
    }

    fn in_idle(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            Start => (State::S1_typing_nameplate, events![L_Refresh]),
            _ => (self.state, events![]),
        }
    }

    fn in_typing_nameplate(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            GotNameplates(nameplates) => (
                State::S1_typing_nameplate,
                events![IH_GotNameplates(nameplates)],
            ),
            ChooseNameplate(nameplate) => {
                self._nameplate = nameplate.to_owned();
                (
                    State::S2_typing_code_without_wordlist,
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
                State::S2_typing_code_without_wordlist,
                events![IH_GotNameplates(nameplates)],
            ),
            GotWordlist(wordlist) => (
                State::S3_typing_code_with_wordlist,
                events![IH_GotWordlist(wordlist)],
            ),
            ChooseWords(words) => {
                let code = format!("{}-{}", self._nameplate, words);
                (State::S4_done, events![C_FinishedInput(code)])
            }
            _ => (self.state, events![]),
        }
    }

    fn in_type_with_wordlist(&mut self, event: InputEvent) -> (State, Events) {
        match event {
            GotNameplates(..) => (self.state, events![]),
            ChooseWords(words) => {
                let code = format!{"{}-{}", self._nameplate, words};
                (State::S4_done, events![C_FinishedInput(code)])
            }
            _ => (self.state, events![]),
        }
    }
}
