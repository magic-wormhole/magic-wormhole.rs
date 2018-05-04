use events::Events;
// we process these
use events::InputEvent::{self, ChooseNameplate, ChooseWords,
                         GetNameplateCompletions, GetWordCompletions,
                         GotNameplates, GotWordlist, RefreshNameplates, Start};
// we emit these
use events::ListerEvent::Refresh as L_Refresh;
use events::CodeEvent::{FinishedInput as C_FinishedInput,
                        GotNameplate as C_GotNameplate};

pub struct Input {
    state: State,
    _all_nameplates: Vec<String>,
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
            _all_nameplates: Vec::new(),
            _nameplate: String::new(),
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        let (newstate, actions) = match self.state {
            S0_idle => self.in_idle(event),
            S1_typing_nameplate => self.in_typing_nameplate(event),
            S2_typing_code_without_wordlist => {
                self.in_type_without_wordlist(event)
            }
            S3_typing_code_with_wordlist => self.in_type_with_wordlist(event),
            S4_done => (self.state, events![]),
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
            GotNameplates(nameplates) => {
                self._all_nameplates = nameplates;
                (State::S1_typing_nameplate, events![])
            }
            ChooseNameplate(nameplate) => {
                self._nameplate = nameplate.to_owned();
                (
                    State::S2_typing_code_without_wordlist,
                    events![C_GotNameplate(nameplate)],
                )
            }
            RefreshNameplates => (self.state, events![L_Refresh]),
            GetNameplateCompletions(prefix) => {
                // TODO: How do we send back set of possible nameplates back to
                // caller? and do we need to generate any events?
                (self.state, events![])
            }
            _ => (self.state, events![]),
        }
    }

    fn in_type_without_wordlist(
        &mut self,
        event: InputEvent,
    ) -> (State, Events) {
        match event {
            GotNameplates(nameplates) => {
                self._all_nameplates = nameplates;
                (State::S2_typing_code_without_wordlist, events![])
            }
            GotWordlist(wordlist) => {
                (State::S3_typing_code_with_wordlist, events![])
            }
            ChooseWords(words) => {
                let code = format!("{}-{}", self._nameplate, words);
                (State::S4_done, events![C_FinishedInput(code)])
            }
            GetWordCompletions(prefix) => {
                // TODO: We can't do any word completions here we should raise
                // error as this should not happen.
                (self.state, events![])
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
            GetWordCompletions(prefix) => {
                // TODO: Here we need to use wordlist to create possible set of
                // completions based on user input but how do we pass it to user?
                (self.state, events![])
            }
            _ => (self.state, events![]),
        }
    }
}
