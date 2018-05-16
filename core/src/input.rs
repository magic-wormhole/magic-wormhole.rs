use events::Events;
use std::rc::Rc;
// we process these
use events::InputEvent::{self, ChooseNameplate, ChooseWords, GotNameplates,
                         GotWordlist, RefreshNameplates, Start};
// we emit these
use api::InputHelperError;
use events::CodeEvent::{FinishedInput as C_FinishedInput,
                        GotNameplate as C_GotNameplate};
use events::ListerEvent::Refresh as L_Refresh;
use events::Wordlist;

pub struct Input {
    state: State,
}

#[derive(Debug)]
enum State {
    Idle,
    WantNameplateNoNameplates,
    WantNameplateHaveNameplates(Vec<String>), // nameplates
    WantCodeNoWordlist(String),               // nameplate
    WantCodeHaveWordlist(String, Rc<Wordlist>), // (nameplate, wordlist)
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
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::State::*;
        match self.state {
            Idle => Err(InputHelperError::Inactive),
            WantNameplateNoNameplates => Ok(Vec::new()),
            WantNameplateHaveNameplates(ref nameplates) => {
                let mut completions = Vec::<String>::new();
                for n in nameplates {
                    if n.starts_with(prefix) {
                        completions.push(n.to_string() + "-");
                    }
                }
                completions.sort();
                Ok(completions)
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
            WantCodeHaveWordlist(_, ref wordlist) => {
                Ok(wordlist.get_completions(prefix))
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
                    wordlist,
                )),
                events![],
            ),
            RefreshNameplates => panic!("already set nameplate"),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test() {
        // in this test, we'll pretend the user charges on through before any
        // nameplates or wordlists ever arrive
        let mut i = Input::new();

        let actions = i.process(Start);
        assert_eq!(actions, events![L_Refresh]);
        let actions = i.process(ChooseNameplate("4".to_string()));
        assert_eq!(
            actions,
            events![C_GotNameplate("4".to_string())]
        );
        let actions = i.process(ChooseWords("purple-sausages".to_string()));
        assert_eq!(
            actions,
            events![C_FinishedInput("4-purple-sausages".to_string())]
        );
    }

    fn vecstrings(all: &str) -> Vec<String> {
        all.split_whitespace()
            .map(|s| {
                if s == "." {
                    "".to_string()
                } else {
                    s.to_string()
                }
            })
            .collect()
    }

    #[test]
    fn test_vecstrings() {
        let mut expected: Vec<String>;
        expected = vec![];
        assert_eq!(vecstrings(""), expected);
        expected = vec!["4".to_string(), "5".to_string()];
        assert_eq!(vecstrings("4 5"), expected);
        expected = vec!["4".to_string(), "".to_string()];
        assert_eq!(vecstrings("4 ."), expected);
    }

    #[test]
    #[allow(unreachable_code)]
    fn test_completions() {
        let mut i = Input::new();
        // you aren't allowed to call these before w.input_code()
        assert_eq!(
            i.get_nameplate_completions(""),
            Err(InputHelperError::Inactive)
        );
        assert_eq!(
            i.get_word_completions(""),
            Err(InputHelperError::Inactive)
        );

        let actions = i.process(Start);
        assert_eq!(actions, events![L_Refresh]);
        // we haven't received any nameplates yet, so completions are empty
        assert_eq!(
            i.get_nameplate_completions("").unwrap(),
            vecstrings("")
        );
        // and it's too early to make word compltions
        assert_eq!(
            i.get_word_completions(""),
            Err(InputHelperError::MustChooseNameplateFirst)
        );

        // now we pretend that we've received a set of active nameplates
        let actions = i.process(GotNameplates(vecstrings("4 48 5 49")));
        assert_eq!(actions, events![]);

        // still too early to make word compltions
        assert_eq!(
            i.get_word_completions(""),
            Err(InputHelperError::MustChooseNameplateFirst)
        );

        // but nameplate completions should work now
        assert_eq!(
            i.get_nameplate_completions("").unwrap(),
            vecstrings("4- 48- 49- 5-")
        );
        assert_eq!(
            i.get_nameplate_completions("4").unwrap(),
            vecstrings("4- 48- 49-")
        );

        // choose the nameplate. This enables word completions, but the list
        // will be empty until we get back the wordlist (for now this is
        // synchronous and fixed, but in the long run this will be informed
        // by server-side properties)
        let actions = i.process(ChooseNameplate("4".to_string()));
        assert_eq!(
            actions,
            events![C_GotNameplate("4".to_string())]
        );

        // now it's too late to complete the nameplate
        assert_eq!(
            i.get_nameplate_completions(""),
            Err(InputHelperError::AlreadyChoseNameplate)
        );

        // wordlist hasn't been received yet, so completions are empty
        assert_eq!(
            i.get_word_completions("pur").unwrap(),
            vecstrings("")
        );

        // receive the wordlist for this nameplate
        let words = vec![
            vecstrings("purple green yellow"),
            vecstrings("sausages seltzer snobol"),
        ];
        let wordlist = Rc::new(Wordlist::new(2, words));
        let actions = i.process(GotWordlist(wordlist));
        assert_eq!(actions, events![]);

        return; // TODO: word completions aren't yet implemented
        assert_eq!(
            i.get_word_completions("pur").unwrap(),
            vecstrings("purple-")
        );

        let actions = i.process(ChooseWords("purple-sausages".to_string()));
        assert_eq!(
            actions,
            events![C_FinishedInput("4-purple-sausages".to_string())]
        );
    }
}
