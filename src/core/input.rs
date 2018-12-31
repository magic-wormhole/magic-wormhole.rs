use super::events::{Code, Events, Nameplate};
use std::sync::Arc;
// we process these
use super::events::InputEvent::{
    self, ChooseNameplate, ChooseWords, GotNameplates, GotWordlist,
    RefreshNameplates, Start,
};
// we emit these
use super::api::InputHelperError;
use super::events::CodeEvent::{
    FinishedInput as C_FinishedInput, GotNameplate as C_GotNameplate,
};
use super::events::ListerEvent::Refresh as L_Refresh;
use super::events::Wordlist;
use super::timing::{new_timelog, now};

pub struct InputMachine {
    state: State,
    wordlist: Option<Arc<Wordlist>>,
    nameplates: Option<Vec<Nameplate>>,
    start_time: f64,
}

#[derive(Debug)]
enum State {
    Idle,
    WantNameplate,
    WantCode(Nameplate), // nameplate
    Done,
}

impl InputMachine {
    pub fn new() -> InputMachine {
        InputMachine {
            state: State::Idle,
            wordlist: None,
            nameplates: None,
            start_time: 0.0,
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        // switch on event first, to avoid a conflict between the match()'s
        // mutable borrow of self.state and the borrow needed by per-state
        // dispatch functions.

        match event {
            Start => self.start(),
            ChooseNameplate(nameplate) => self.choose_nameplate(&nameplate),
            ChooseWords(words) => self.choose_words(&words),
            GotNameplates(nameplates) => self.got_nameplates(nameplates),
            GotWordlist(wordlist) => self.got_wordlist(wordlist),
            RefreshNameplates => self.refresh_nameplates(),
        }
    }

    fn start(&mut self) -> Events {
        use self::State::*;
        match self.state {
            Idle => {
                self.state = WantNameplate;
                self.start_time = now();
                events![L_Refresh]
            }
            _ => panic!("already started"),
        }
    }

    fn choose_nameplate(&mut self, nameplate: &Nameplate) -> Events {
        use self::State::*;
        match self.state {
            Idle => panic!("too soon"),
            WantNameplate => {
                self.state = WantCode(nameplate.clone());
                events![C_GotNameplate(nameplate.clone())]
            }
            _ => panic!("already set nameplate"),
        }
    }

    fn choose_words(&mut self, words: &str) -> Events {
        use self::State::*;
        let mut newstate: Option<State> = None;
        let events = match self.state {
            Idle => panic!("too soon"),
            WantCode(ref nameplate) => {
                let mut t = new_timelog(
                    "input code",
                    Some(self.start_time),
                    Some(("waiting", "user")),
                );
                t.finish(None, None);
                let code = Code(format!("{}-{}", *nameplate, words));
                newstate = Some(Done);
                events![t, C_FinishedInput(code)]
            }
            Done => events![], // REMOVE
            _ => panic!("already set nameplate"),
        };
        if newstate.is_some() {
            self.state = newstate.unwrap();
        }
        events
    }

    fn got_nameplates(&mut self, nameplates: Vec<Nameplate>) -> Events {
        self.nameplates = Some(nameplates);
        events![]
    }

    fn got_wordlist(&mut self, wordlist: Arc<Wordlist>) -> Events {
        self.wordlist = Some(wordlist);
        events![]
    }

    fn refresh_nameplates(&mut self) -> Events {
        use self::State::*;
        match self.state {
            Idle => panic!("too early, I think"),
            WantNameplate => events![L_Refresh],
            _ => panic!("already chose nameplate, stop refreshing"),
        }
    }

    // InputHelper functions

    pub fn get_nameplate_completions(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::InputHelperError::*;
        use self::State::*;
        match self.state {
            Idle => Err(Inactive),
            WantNameplate => match self.nameplates {
                None => Ok(Vec::new()),
                Some(ref nameplates) => {
                    let mut completions = Vec::<String>::new();
                    for n in nameplates {
                        if n.starts_with(prefix) {
                            completions.push(n.to_string() + "-");
                        }
                    }
                    completions.sort();
                    Ok(completions)
                }
            },
            WantCode(..) => Err(AlreadyChoseNameplate),
            Done => Err(AlreadyChoseNameplate),
        }
    }

    pub fn get_word_completions(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::InputHelperError::*;
        use self::State::*;
        match self.state {
            Idle => Err(Inactive),
            WantNameplate => Err(MustChooseNameplateFirst),
            WantCode(..) => {
                match self.wordlist {
                    Some(ref wordlist) => Ok(wordlist.get_completions(prefix)),
                    None => Ok(Vec::new()), // no wordlist, no completions
                }
            }
            Done => Err(AlreadyChoseWords),
        }
    }

    // TODO: remove this, the helper should remember whether it's called
    // choose_nameplate yet or not instead of asking the core
    pub fn committed_nameplate(&self) -> Option<&Nameplate> {
        use self::State::*;
        match self.state {
            WantCode(ref nameplate) => Some(nameplate),
            _ => None,
        }
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::super::test::filt;
    use super::*;

    #[test]
    fn test() {
        // in this test, we'll pretend the user charges on through before any
        // nameplates or wordlists ever arrive
        let mut i = InputMachine::new();

        let actions = filt(i.process(Start));
        assert_eq!(actions, events![L_Refresh]);
        let actions =
            filt(i.process(ChooseNameplate(Nameplate("4".to_string()))));
        assert_eq!(
            actions,
            events![C_GotNameplate(Nameplate("4".to_string()))]
        );
        let actions =
            filt(i.process(ChooseWords("purple-sausages".to_string())));
        assert_eq!(
            actions,
            events![C_FinishedInput(Code("4-purple-sausages".to_string()))]
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

    fn vecnameplates(all: &str) -> Vec<Nameplate> {
        all.split_whitespace()
            .map(|s| {
                if s == "." {
                    "".to_string()
                } else {
                    s.to_string()
                }
            })
            .map(|s| Nameplate(s))
            .collect()
    }

    #[test]
    fn test_completions() {
        let mut i = InputMachine::new();
        // you aren't allowed to call these before w.input_code()
        assert_eq!(
            i.get_nameplate_completions(""),
            Err(InputHelperError::Inactive)
        );
        assert_eq!(i.get_word_completions(""), Err(InputHelperError::Inactive));

        let actions = filt(i.process(Start));
        assert_eq!(actions, events![L_Refresh]);
        // we haven't received any nameplates yet, so completions are empty
        assert_eq!(i.get_nameplate_completions("").unwrap(), vecstrings(""));
        // and it's too early to make word compltions
        assert_eq!(
            i.get_word_completions(""),
            Err(InputHelperError::MustChooseNameplateFirst)
        );

        // now we pretend that we've received a set of active nameplates
        let actions =
            filt(i.process(GotNameplates(vecnameplates("4 48 5 49"))));
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
        let actions =
            filt(i.process(ChooseNameplate(Nameplate("4".to_string()))));
        assert_eq!(
            actions,
            events![C_GotNameplate(Nameplate("4".to_string()))]
        );

        // now it's too late to complete the nameplate
        assert_eq!(
            i.get_nameplate_completions(""),
            Err(InputHelperError::AlreadyChoseNameplate)
        );

        // wordlist hasn't been received yet, so completions are empty
        assert_eq!(i.get_word_completions("pur").unwrap(), vecstrings(""));

        // receive the wordlist for this nameplate
        let words = vec![
            vecstrings("purple green yellow"),
            vecstrings("sausages seltzer snobol"),
        ];
        let wordlist = Arc::new(Wordlist::new(2, words));
        let actions = filt(i.process(GotWordlist(wordlist)));
        assert_eq!(actions, events![]);

        assert_eq!(
            i.get_word_completions("pur").unwrap(),
            vecstrings("purple-")
        );

        let actions =
            filt(i.process(ChooseWords("purple-sausages".to_string())));
        assert_eq!(
            actions,
            events![C_FinishedInput(Code("4-purple-sausages".to_string()))]
        );
    }
}
