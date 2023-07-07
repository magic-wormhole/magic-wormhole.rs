use rand::{rngs::OsRng, seq::SliceRandom};
use serde_json::{self, Value};

pub struct PgpWordList {
    pub words: Vec<Vec<String>>,
    pub num_words: usize,
}

impl PgpWordList {
    pub fn new(num_words: usize, words: Vec<Vec<String>>) -> Self {
        Self { words, num_words }
    }
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
    let raw_words_value: serde_json::Value =
        serde_json::from_str(include_str!("pgpwords.json")).unwrap();
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

pub fn default_wordlist(num_words: usize) -> PgpWordList {
    PgpWordList {
        num_words,
        words: load_pgpwords(),
    }
}

pub fn vecstrings(all: &str) -> Vec<String> {
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
    fn test_choose_words() {
        let few_words: Vec<Vec<String>> = vec![vecstrings("purple"), vecstrings("sausages")];

        let w = PgpWordList::new(2, few_words.clone());
        assert_eq!(w.choose_words(), "purple-sausages");
        let w = PgpWordList::new(3, few_words.clone());
        assert_eq!(w.choose_words(), "purple-sausages-purple");
        let w = PgpWordList::new(4, few_words);
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

        let w = PgpWordList::new(2, more_words.clone());
        for _ in 0..20 {
            assert!(expected2.contains(&w.choose_words()));
        }

        let w = PgpWordList::new(3, more_words);
        for _ in 0..20 {
            assert!(expected3.contains(&w.choose_words()));
        }
    }
}
