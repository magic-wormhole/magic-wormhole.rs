use super::events::{Code, Events, Nameplate};
use std::sync::Arc;
// we process these
use super::events::InputEvent;
// we emit these
use super::api::InputHelperError;
use super::events::CodeEvent::{
    FinishedInput as C_FinishedInput, GotNameplate as C_GotNameplate,
};
use super::events::ListerEvent::Refresh as L_Refresh;
use super::events::Wordlist;
use super::timing::{new_timelog, now};

pub struct InputMachine {
    state: Option<State>,
    start_time: f64,
}

#[derive(Debug)]
enum State {
    Idle,
    WantNameplateNoNameplates,
    WantNameplateYesNameplates(Vec<Nameplate>),
    WantCodeNoWordlist(Nameplate),
    WantCodeYesWordlist(Nameplate, Arc<Wordlist>),
    Done,
}

fn choose_words(
    nameplate: Nameplate,
    words: String,
    start_time: f64,
) -> (State, Events) {
    let mut t = new_timelog("input code", Some(start_time));
    t.detail("waiting", "user");
    t.finish(None);
    let code = Code(format!("{}-{}", nameplate, words));
    (State::Done, events![t, C_FinishedInput(code)])
}

impl InputMachine {
    pub fn new() -> InputMachine {
        InputMachine {
            state: Some(State::Idle),
            start_time: 0.0,
        }
    }

    pub fn process(&mut self, event: InputEvent) -> Events {
        use InputEvent::*;
        use State::*;
        let old_state = self.state.take().unwrap();
        let mut actions = Events::new();
        self.state = Some(match old_state {
            Idle => match event {
                Start => {
                    self.start_time = now();
                    actions.push(L_Refresh);
                    WantNameplateNoNameplates
                }
                GotWordlist(_) => old_state,
                _ => panic!(),
            },
            WantNameplateNoNameplates => match event {
                RefreshNameplates => {
                    actions.push(L_Refresh);
                    old_state
                }
                GotNameplates(nameplates) => {
                    WantNameplateYesNameplates(nameplates)
                }
                ChooseNameplate(nameplate) => {
                    actions.push(C_GotNameplate(nameplate.clone()));
                    WantCodeNoWordlist(nameplate)
                }
                _ => panic!(),
            },
            WantNameplateYesNameplates(..) => match event {
                RefreshNameplates => {
                    actions.push(L_Refresh);
                    old_state
                }
                GotNameplates(new_nameplates) => {
                    WantNameplateYesNameplates(new_nameplates)
                }
                ChooseNameplate(nameplate) => {
                    actions.push(C_GotNameplate(nameplate.clone()));
                    WantCodeNoWordlist(nameplate)
                }
                _ => panic!(),
            },
            WantCodeNoWordlist(nameplate) => match event {
                RefreshNameplates => {
                    panic!("already chose nameplate, stop refreshing")
                }
                GotNameplates(_) => WantCodeNoWordlist(nameplate),
                ChooseWords(words) => {
                    let (new_state, new_actions) =
                        choose_words(nameplate, words, self.start_time);
                    for a in new_actions {
                        actions.push(a);
                    }
                    new_state
                }
                GotWordlist(wordlist) => {
                    WantCodeYesWordlist(nameplate, wordlist)
                }
                _ => panic!(),
            },
            WantCodeYesWordlist(nameplate, wordlist) => match event {
                RefreshNameplates => {
                    panic!("already chose nameplate, stop refreshing")
                }
                GotNameplates(_) => WantCodeYesWordlist(nameplate, wordlist),
                GotWordlist(_) => WantCodeYesWordlist(nameplate, wordlist),
                ChooseWords(words) => {
                    let (new_state, new_actions) =
                        choose_words(nameplate, words, self.start_time);
                    for a in new_actions {
                        actions.push(a);
                    }
                    new_state
                }
                _ => panic!(),
            },
            Done => match event {
                RefreshNameplates => {
                    panic!("already chose nameplate, stop refreshing")
                }
                GotNameplates(_) => old_state,
                GotWordlist(_) => old_state,
                _ => panic!(),
            },
        });
        actions
    }

    // InputHelper functions

    pub fn get_nameplate_completions(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::InputHelperError::*;
        use self::State::*;
        match self.state {
            Some(ref s) => match s {
                Idle => Err(Inactive),
                WantNameplateNoNameplates => Ok(Vec::new()),
                WantNameplateYesNameplates(ref nameplates) => {
                    let mut completions = Vec::<String>::new();
                    for n in nameplates {
                        if n.starts_with(prefix) {
                            completions.push(n.to_string() + "-");
                        }
                    }
                    completions.sort();
                    Ok(completions)
                }
                WantCodeNoWordlist(..) => Err(AlreadyChoseNameplate),
                WantCodeYesWordlist(..) => Err(AlreadyChoseNameplate),
                Done => Err(AlreadyChoseNameplate),
            },
            None => panic!(),
        }
    }

    pub fn get_word_completions(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, InputHelperError> {
        use self::InputHelperError::*;
        use self::State::*;
        match self.state {
            Some(ref s) => match s {
                Idle => Err(Inactive),
                WantNameplateNoNameplates | WantNameplateYesNameplates(..) => {
                    Err(MustChooseNameplateFirst)
                }
                WantCodeNoWordlist(..) => Ok(Vec::new()),
                WantCodeYesWordlist(_, ref wordlist) => {
                    Ok(wordlist.get_completions(prefix))
                }
                Done => Err(AlreadyChoseWords),
            },
            None => panic!(),
        }
    }

    // TODO: remove this, the helper should remember whether it's called
    // choose_nameplate yet or not instead of asking the core
    pub fn committed_nameplate(&self) -> Option<Nameplate> {
        use self::State::*;
        match self.state {
            Some(ref s) => match s {
                WantCodeNoWordlist(ref nameplate) => Some(nameplate.clone()),
                WantCodeYesWordlist(ref nameplate, _) => {
                    Some(nameplate.clone())
                }
                _ => None,
            },
            None => panic!(),
        }
    }
}

#[cfg_attr(tarpaulin, skip)]
#[cfg(test)]
mod test {
    use super::super::test::filt;
    use super::InputEvent::*;
    use super::*;

    #[test]
    fn test() {
        // in this test, we'll pretend the user charges on through before any
        // nameplates or wordlists ever arrive
        let mut i = InputMachine::new();

        let actions = filt(i.process(Start));
        assert_eq!(actions, events![L_Refresh]);
        let actions =
            filt(i.process(ChooseNameplate(Nameplate(String::from("4")))));
        assert_eq!(
            actions,
            events![C_GotNameplate(Nameplate(String::from("4")))]
        );
        let actions =
            filt(i.process(ChooseWords(String::from("purple-sausages"))));
        assert_eq!(
            actions,
            events![C_FinishedInput(Code(String::from("4-purple-sausages")))]
        );
    }

    fn vecstrings(all: &str) -> Vec<String> {
        all.split_whitespace()
            .map(|s| {
                if s == "." {
                    String::from("")
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
        expected = vec![String::from("4"), String::from("5")];
        assert_eq!(vecstrings("4 5"), expected);
        expected = vec![String::from("4"), String::from("")];
        assert_eq!(vecstrings("4 ."), expected);
    }

    fn vecnameplates(all: &str) -> Vec<Nameplate> {
        all.split_whitespace()
            .map(|s| {
                if s == "." {
                    Nameplate::new("")
                } else {
                    Nameplate::new(s)
                }
            })
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
            filt(i.process(ChooseNameplate(Nameplate(String::from("4")))));
        assert_eq!(
            actions,
            events![C_GotNameplate(Nameplate(String::from("4")))]
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
            filt(i.process(ChooseWords(String::from("purple-sausages"))));
        assert_eq!(
            actions,
            events![C_FinishedInput(Code(String::from("4-purple-sausages")))]
        );
    }
}
