use rand::{rngs::OsRng, seq::SliceRandom};
use serde_json::{self, Value};
use std::fmt;

#[derive(PartialEq)]
pub struct Wordlist {
    num_words: usize,
    words: Vec<Vec<String>>,
}

impl fmt::Debug for Wordlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Wordlist ( {}, lots of words...)", self.num_words)
    }
}

impl Wordlist {
    #[cfg(test)]
    pub fn new(num_words: usize, words: Vec<Vec<String>>) -> Wordlist {
        Wordlist { num_words, words }
    }

    #[allow(dead_code)] // TODO make this API public one day
    pub fn get_completions(&self, prefix: &str) -> Vec<String> {
        let count_dashes = prefix.matches('-').count();
        let mut completions = Vec::new();
        let words = &self.words[count_dashes % self.words.len()];

        let last_partial_word = prefix.split('-').last();
        let lp = if let Some(w) = last_partial_word {
            w.len()
        } else {
            0
        };

        for word in words {
            let mut suffix: String = prefix.to_owned();
            if word.starts_with(last_partial_word.unwrap()) {
                if lp == 0 {
                    suffix.push_str(&word);
                } else {
                    let p = prefix.len() - lp;
                    suffix.truncate(p as usize);
                    suffix.push_str(&word);
                }

                if count_dashes + 1 < self.num_words {
                    suffix.push_str("-");
                }

                completions.push(suffix);
            }
        }
        completions.sort();
        completions
    }

    pub fn choose_words(&self) -> String {
        let mut rng = OsRng;
        let components: Vec<String>;
        components = self
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
    fn test_completion() {
        let words: Vec<Vec<String>> = vec![
            vecstrings("purple green yellow"),
            vecstrings("sausages seltzer snobol"),
        ];

        let w = Wordlist::new(2, words);
        assert_eq!(w.get_completions(""), vec!["green-", "purple-", "yellow-"]);
        assert_eq!(w.get_completions("pur"), vec!["purple-"]);
        assert_eq!(w.get_completions("blu"), Vec::<String>::new());
        assert_eq!(w.get_completions("purple-sa"), vec!["purple-sausages"]);
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
    fn test_default_completions() {
        let w = default_wordlist(2);
        let c = w.get_completions("ar");
        assert_eq!(c.len(), 2);
        assert!(c.contains(&String::from("article-")));
        assert!(c.contains(&String::from("armistice-")));

        let c = w.get_completions("armis");
        assert_eq!(c.len(), 1);
        assert!(c.contains(&String::from("armistice-")));

        let c = w.get_completions("armistice-");
        assert_eq!(c.len(), 256);

        let c = w.get_completions("armistice-ba");
        assert_eq!(
            c,
            vec![
                "armistice-baboon",
                "armistice-backfield",
                "armistice-backward",
                "armistice-banjo",
            ]
        );

        let w = default_wordlist(3);
        let c = w.get_completions("armistice-ba");
        assert_eq!(
            c,
            vec![
                "armistice-baboon-",
                "armistice-backfield-",
                "armistice-backward-",
                "armistice-banjo-",
            ]
        );

        let w = default_wordlist(4);
        let c = w.get_completions("armistice-baboon");
        assert_eq!(c, vec!["armistice-baboon-"]);
    }
}
