use events::{Events, Wordlist};
use wordlist::PGPWordlist;
// We process these events
use events::InputHelperEvent::{self, ChooseNameplate, ChooseWords,
                               GotNameplates, GotWordlist, RefreshNameplates};
// We emit the following events
use events::InputEvent::{ChooseNameplate as I_ChooseNameplate,
                         ChooseWords as I_ChooseWords,
                         RefreshNameplates as I_RefreshNameplates};

pub struct InputHelper {
    _all_nameplates: Option<Vec<String>>,
    _wordlist: Option<Wordlist>,
}

impl InputHelper {
    pub fn new() -> Self {
        InputHelper {
            _all_nameplates: None,
            _wordlist: None,
        }
    }

    pub fn process(&mut self, event: InputHelperEvent) -> Events {
        match event {
            RefreshNameplates => events![I_RefreshNameplates],
            GotNameplates(nameplates) => {
                self._all_nameplates = Some(nameplates);
                events![]
            }
            GotWordlist(wordlist) => {
                self._wordlist = Some(wordlist);
                events![]
            }
            ChooseWords(words) => events![I_ChooseWords(words)],
            ChooseNameplate(nameplate) => events![I_ChooseNameplate(nameplate)],
        }
    }

    fn get_word_completions(&self, prefix: &str) -> HashSet<String> {
        let wordlist = PGPWordlist::new();
        wordlist.get_completions(prefix, 2)
    }

    pub fn get_completions(&self, prefix: &str) -> (Events, Vec<String>) {
        // If we find '-' then there is a nameplate already entered
        let got_nameplate = prefix.find('-').is_some();

        if got_nameplate {
            let ns: Vec<&str> = prefix.splitn(1, '-').collect();
            let nameplate = ns[0];
            let words = ns.join("");

            // We have already the nameplate hence we need to emit event telling
            // input machine about nameplate
            let completions: Vec<String> =
                self.get_word_completions(words).iter().collect();
            (
                events![I_ChooseNameplate(nameplate.to_string())],
                completions.iter().map(|w| nameplate + "-" + w).collect(),
            )
        } else {
            let completions: Vec<String> = self._all_nameplates
                .iter()
                .filter(|n| n.starts_with(prefix))
                .map(|n| n + "-")
                .collect();
            (events![], completions)
        }
    }
}
