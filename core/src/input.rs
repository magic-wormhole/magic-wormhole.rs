use events::Events;
// we process these
use events::InputEvent::{self, ChooseNameplate, ChooseWords, GotNameplates,
                         GotWordlist, RefreshNameplates, Start};
// we emit these
use api::InputHelperError;
use events::CodeEvent::{FinishedInput as C_FinishedInput,
                        GotNameplate as C_GotNameplate};
use events::ListerEvent::Refresh as L_Refresh;
use events::Wordlist;
use wordlist::PGPWordlist;

pub struct Input {
    state: State,
}

#[derive(Debug)]
enum State {
    Idle,
    WantNameplateNoNameplates,
    WantNameplateHaveNameplates(Vec<String>), // nameplates
    WantCodeNoWordlist(String),               // nameplate
    WantCodeHaveWordlist(String, Box<Wordlist>), // (nameplate, wordlist)
    Done,
}

impl Input {
    pub fn new() -> Input {
        Input {
            state: State::Idle,
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        use self::State::*;
        let (newstate, actions) = match self.state {
            Idle => self.idle(event),
            WantNameplateNoNameplates => self.want_nameplate(event),
            WantNameplateHaveNameplates(_) => self.want_nameplate(event),
            WantCodeNoWordlist(ref nameplate) => {
                self.want_code(event, &nameplate)
            }
            WantCodeHaveWordlist(ref nameplate, _) => {
                self.want_code(event, &nameplate)
            }
            Done => (Some(Done), events![]),
        };

        if let Some(s) = newstate {
            self.state = s;
        }
        actions
    }

    fn idle(&self, event: InputEvent) -> (Option<State>, Events) {
        use self::State::*;
        use events::InputEvent::*;
        match event {
            Start => (
                Some(WantNameplateNoNameplates),
                events![L_Refresh],
            ),
            ChooseNameplate(_) => panic!("too soon"),
            ChooseWords(_) => panic!("too soon"),
            GotNameplates(_) => panic!("also too soon"),
            GotWordlist(_) => panic!("probably too soon"),
            RefreshNameplates => panic!("almost certainly too soon"),
        }
    }

    pub fn get_nameplate_completions(
        &self,
        _prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::State::*;
        match self.state {
            Idle => Err(InputHelperError::Inactive),
            WantNameplateNoNameplates => Ok(Vec::new()),
            WantNameplateHaveNameplates(ref _nameplates) => {
                Ok(Vec::new()) // TODO
            }
            WantCodeNoWordlist(_) => {
                Err(InputHelperError::AlreadyChoseNameplate)
            }
            WantCodeHaveWordlist(_, _) => {
                Err(InputHelperError::AlreadyChoseNameplate)
            }
            Done => Err(InputHelperError::AlreadyChoseNameplate),
        }
    }

    pub fn get_word_completions(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        let _completions: Vec<String>;
        use self::State::*;
        match self.state {
            Idle => Err(InputHelperError::Inactive),
            WantNameplateNoNameplates => {
                Err(InputHelperError::MustChooseNameplateFirst)
            }
            WantNameplateHaveNameplates(_) => {
                Err(InputHelperError::MustChooseNameplateFirst)
            }
            WantCodeNoWordlist(_) => Ok(Vec::new()), // no wordlist, no completions
            WantCodeHaveWordlist(_, ref _wordlist) => {
                let wordlist = PGPWordlist::new(); // TODO
                let _completions = wordlist.get_completions(prefix, 2);
                Ok(Vec::new()) // TODO
            }
            Done => Err(InputHelperError::AlreadyChoseWords),
        }
    }

    // can we get the wordlist before setting the nameplate??
    fn want_nameplate(&self, event: InputEvent) -> (Option<State>, Events) {
        use self::State::*;
        match event {
            Start => panic!("already started"),
            ChooseNameplate(nameplate) => (
                Some(WantCodeNoWordlist(nameplate.clone())),
                events![C_GotNameplate(nameplate.clone())],
            ),
            ChooseWords(_) => panic!("expecting nameplate, not words"),
            GotNameplates(nameplates) => (
                Some(WantNameplateHaveNameplates(nameplates)),
                events![],
            ),
            GotWordlist(_) => panic!("expecting nameplate, not words"),
            RefreshNameplates => (None, events![L_Refresh]),
        }
    }

    fn want_code(
        &self,
        event: InputEvent,
        nameplate: &str,
    ) -> (Option<State>, Events) {
        use self::State::*;
        match event {
            Start => panic!("already started"),
            ChooseNameplate(_) => panic!("expecting words, not nameplate"),
            ChooseWords(words) => {
                let code = format!("{}-{}", nameplate, words);
                (Some(Done), events![C_FinishedInput(code)])
            }
            GotNameplates(_) => (None, events![]),
            GotWordlist(wordlist) => (
                Some(WantCodeHaveWordlist(
                    nameplate.to_string(),
                    Box::new(wordlist),
                )),
                events![],
            ),
            RefreshNameplates => panic!("already set nameplate"),
        }
    }
}
