use events::Events;
use std::sync::Arc;
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
    WantCodeHaveWordlist(String, Arc<Wordlist>), // (nameplate, wordlist)
    Done,
}

impl Input {
    pub fn new() -> Input {
        Input {
            state: State::Idle,
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        // switch on event first, to avoid a conflict between the match()'s
        // mutable borrow of self.state and the borrow needed by per-state
        // dispatch functions.

        match event {
            Start => self.start(),
            ChooseNameplate(nameplate) => self.choose_nameplate(nameplate),
            ChooseWords(words) => self.choose_words(words),
            GotNameplates(nameplates) => self.got_nameplates(nameplates),
            GotWordlist(wordlist) => self.got_wordlist(wordlist),
            RefreshNameplates => self.refresh_nameplates(),
        }
    }

    fn start(&mut self) -> Events {
        use self::State::*;
        match self.state {
            Idle => {
                self.state = WantNameplateNoNameplates;
                events![L_Refresh]
            },
            _ => panic!("already started"),
        }
    }

    fn choose_nameplate(&mut self, nameplate: String) -> Events {
        use self::State::*;
        match self.state {
            Idle => panic!("too soon"),
            WantNameplateNoNameplates | WantNameplateHaveNameplates(..) => {
                self.state = WantCodeNoWordlist(nameplate.clone());
                events![C_GotNameplate(nameplate.clone())]
            },
            _ => panic!("already set nameplate"),
        }
    }

    fn choose_words(&mut self, words: String) -> Events {
        use self::State::*;
        let mut newstate: Option<State> = None;
        let events = match self.state {
            Idle => panic!("too soon"),
            WantCodeNoWordlist(ref nameplate) | WantCodeHaveWordlist(ref nameplate, _) => {
                let code = format!("{}-{}", nameplate, words);
                newstate = Some(Done);
                events![C_FinishedInput(code)]
            },
            Done => events![], // REMOVE
            _ => panic!("already set nameplate"),
        };
        if newstate.is_some() {
            self.state = newstate.unwrap();
        }
        events
    }

    fn got_nameplates(&mut self, nameplates: Vec<String>) -> Events {
        use self::State::*;
        match self.state {
            Idle => panic!("this shouldn't happen, I think"),
            WantNameplateNoNameplates => {
                self.state = WantNameplateHaveNameplates(nameplates);
                events![]
            },
            WantNameplateHaveNameplates(..) => {
                self.state = WantNameplateHaveNameplates(nameplates);
                events![]
            },
            _ => events![],
        }
    }

    fn got_wordlist(&mut self, wordlist: Arc<Wordlist>) -> Events {
        use self::State::*;
        #[allow(unused_assignments)]
        let mut newstate: Option<State> = None;
        let events = match self.state {
            // TODO: is it possible for the wordlist to arrive before we set
            // the nameplate?
            Idle | WantNameplateNoNameplates | WantNameplateHaveNameplates(..) => panic!("I should be prepared for this, but I'm not"),
            WantCodeNoWordlist(ref nameplate) => {
                newstate = Some(WantCodeHaveWordlist(nameplate.clone(), wordlist));
                events![]
            },
            _ => panic!("wordlist already set"),
        };
        if newstate.is_some() {
            self.state = newstate.unwrap();
        }
        events
    }

    fn refresh_nameplates(&mut self) -> Events {
        use self::State::*;
        match self.state {
            Idle => panic!("too early, I think"),
            WantNameplateNoNameplates | WantNameplateHaveNameplates(..) =>
                events![L_Refresh],
            _ => panic!("already chose nameplate, stop refreshing"),
        }
    }


    // InputHelper functions

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

    // TODO: remove this, the helper should remember whether it's called
    // choose_nameplate yet or not instead of asking the core
    pub fn committed_nameplate(&self) -> Option<&str> {
        use self::State::*;
        match self.state {
            WantCodeHaveWordlist(ref nameplate, _)
            | WantCodeNoWordlist(ref nameplate) => Some(nameplate),
            _ => None,
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
        let wordlist = Arc::new(Wordlist::new(2, words));
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
