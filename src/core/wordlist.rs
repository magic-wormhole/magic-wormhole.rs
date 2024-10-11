///! Wordlist generation and wormhole code utilities
use rand::{rngs::OsRng, seq::SliceRandom};
use serde_json::{self, Value};
use std::fmt;

/// Represents a list of words used to generate and complete wormhole codes.
/// A wormhole code is a sequence of words used for secure communication or identification.
#[derive(PartialEq)]
pub struct Wordlist {
    /// Number of words in a wormhole code
    num_words: usize,
    /// Odd and even wordlist
    words: Vec<Vec<String>>,
}

impl fmt::Debug for Wordlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wordlist ( {}, lots of words...)", self.num_words)
    }
}

impl Wordlist {
    #[cfg(test)]
    #[doc(hidden)]
    pub fn new(num_words: usize, words: Vec<Vec<String>>) -> Wordlist {
        Wordlist { num_words, words }
    }

    /// Completes a wormhole code
    ///
    /// Completion can be done either with fuzzy search (approximate string matching)
    /// or simple `starts_with` matching.
    pub fn get_completions(&self, prefix: &str) -> Vec<String> {
        let words = self.get_wordlist(prefix);

        let (prefix_without_last, last_partial) = prefix.rsplit_once('-').unwrap_or(("", prefix));

        let matches = if cfg!(feature = "fuzzy-complete") {
            self.fuzzy_complete(last_partial, words)
        } else {
            self.normal_complete(last_partial, words)
        };

        matches
            .into_iter()
            .map(|word| {
                let mut completion = String::new();
                completion.push_str(prefix_without_last);
                if !prefix_without_last.is_empty() {
                    completion.push('-');
                }
                completion.push_str(&word);
                completion
            })
            .collect()
    }

    fn get_wordlist(&self, prefix: &str) -> &Vec<String> {
        let count_dashes = prefix.matches('-').count();
        &self.words[count_dashes % self.words.len()]
    }

    #[allow(unused)]
    fn fuzzy_complete(&self, partial: &str, words: &[String]) -> Vec<String> {
        // We use Jaro-Winkler algorithm because it emphasizes the beginning of a word
        use fuzzt::algorithms::JaroWinkler;

        let words = words.iter().map(|w| w.as_str()).collect::<Vec<&str>>();

        fuzzt::get_top_n(partial, &words, None, None, None, Some(&JaroWinkler))
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[allow(unused)]
    fn normal_complete(&self, partial: &str, words: &[String]) -> Vec<String> {
        words
            .iter()
            .filter(|word| word.starts_with(partial))
            .cloned()
            .collect()
    }

    /// Choose wormhole code word
    pub fn choose_words(&self) -> String {
        let mut rng = OsRng;
        let components: Vec<String> = self
            .words
            .iter()
            .cycle()
            .take(self.num_words)
            .map(|words| words.choose(&mut rng).unwrap().to_string())
            .collect();
        components.join("-")
    }
}

fn load_pgpwords() -> Vec<Vec<String>> {
    let raw_words_value: Value = serde_json::from_str(include_str!("pgpwords.json")).unwrap();
    let raw_words = raw_words_value.as_object().unwrap();
    let mut even_words: Vec<String> = Vec::with_capacity(256);
    even_words.resize(256, String::from(""));
    let mut odd_words: Vec<String> = Vec::with_capacity(256);
    odd_words.resize(256, String::from(""));
    for (index_str, values) in raw_words.iter() {
        let index = u8::from_str_radix(index_str, 16).unwrap() as usize;
        even_words[index] = values
            .get(1)
            .unwrap()
            .as_str()
            .unwrap()
            .to_lowercase()
            .to_string();
        odd_words[index] = values
            .get(0)
            .unwrap()
            .as_str()
            .unwrap()
            .to_lowercase()
            .to_string();
    }

    vec![even_words, odd_words]
}

/// Construct Wordlist struct with given number of words in a wormhole code
pub fn default_wordlist(num_words: usize) -> Wordlist {
    Wordlist {
        num_words,
        words: load_pgpwords(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_load_words() {
        let w = load_pgpwords();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0][0], "adroitness");
        assert_eq!(w[1][0], "aardvark");
        assert_eq!(w[0][255], "yucatan");
        assert_eq!(w[1][255], "zulu");
    }

    #[test]
    fn test_default_wordlist() {
        let d = default_wordlist(2);
        assert_eq!(d.words.len(), 2);
        assert_eq!(d.words[0][0], "adroitness");
        assert_eq!(d.words[1][0], "aardvark");
        assert_eq!(d.words[0][255], "yucatan");
        assert_eq!(d.words[1][255], "zulu");
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
    fn test_choose_words() {
        let few_words: Vec<Vec<String>> = vec![vecstrings("purple"), vecstrings("sausages")];

        let w = Wordlist::new(2, few_words.clone());
        assert_eq!(w.choose_words(), "purple-sausages");
        let w = Wordlist::new(3, few_words.clone());
        assert_eq!(w.choose_words(), "purple-sausages-purple");
        let w = Wordlist::new(4, few_words);
        assert_eq!(w.choose_words(), "purple-sausages-purple-sausages");
    }

    #[test]
    fn test_choose_more_words() {
        let more_words: Vec<Vec<String>> =
            vec![vecstrings("purple yellow"), vecstrings("sausages")];

        let expected2 = vecstrings("purple-sausages yellow-sausages");
        let expected3: Vec<String> = vec![
            "purple-sausages-purple",
            "yellow-sausages-purple",
            "purple-sausages-yellow",
            "yellow-sausages-yellow",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        let w = Wordlist::new(2, more_words.clone());
        for _ in 0..20 {
            assert!(expected2.contains(&w.choose_words()));
        }

        let w = Wordlist::new(3, more_words);
        for _ in 0..20 {
            assert!(expected3.contains(&w.choose_words()));
        }
    }

    #[test]
    #[cfg(feature = "fuzzy-complete")]
    fn test_wormhole_code_fuzzy_completions() {
        let list = default_wordlist(2);

        assert_eq!(list.get_completions("22"), Vec::<String>::new());
        assert_eq!(list.get_completions("22-"), Vec::<String>::new());

        // Invalid wormhole code check
        assert_eq!(list.get_completions("trj"), Vec::<String>::new());

        assert_eq!(
            list.get_completions("22-chisel"),
            ["22-chisel", "22-chairlift", "22-christmas"]
        );

        assert_eq!(
            list.get_completions("22-chle"),
            ["22-chisel", "22-chatter", "22-checkup"]
        );

        assert_eq!(list.get_completions("22-chisel-tba"), ["22-chisel-tobacco"]);
    }

    #[test]
    #[cfg(feature = "fuzzy-complete")]
    fn test_completion_fuzzy() {
        let wl = default_wordlist(2);
        let list = wl.get_wordlist("22-");

        assert_eq!(wl.fuzzy_complete("chck", list), ["checkup", "choking"]);
        assert_eq!(wl.fuzzy_complete("checkp", list), ["checkup"]);
        assert_eq!(
            wl.fuzzy_complete("checkup", list),
            ["checkup", "lockup", "cleanup"]
        );
    }

    #[test]
    fn test_completion_normal() {
        let wl = default_wordlist(2);
        let list = wl.get_wordlist("22-");

        assert_eq!(wl.normal_complete("che", list), ["checkup"]);
    }

    #[test]
    fn test_full_wormhole_completion() {
        let wl = default_wordlist(2);

        assert_eq!(wl.get_completions("22-chec").first().unwrap(), "22-checkup");
        assert_eq!(
            wl.get_completions("22-checkup-t").first().unwrap(),
            "22-checkup-tobacco"
        );
    }
}
