use serde_json::{self, from_str, Value};
use std::collections::{HashMap, HashSet};

use util::{bytes_to_hexstr, random_bytes};

#[derive(Debug, PartialEq, Clone)]
pub struct PGPWordlist {
    _byte_even_words: HashMap<String, String>,
    _byte_odd_words: HashMap<String, String>,
}

impl PGPWordlist {
    pub fn new() -> Self {
        let raw_words: Value =
            serde_json::from_str(include_str!("pgpwords.json")).unwrap();
        let map_obj = raw_words.as_object().unwrap();
        let even_words = map_obj
            .iter()
            .map(|item| {
                let (k, v): (&String, &Value) = item;
                let both_words: Vec<String> = v.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                (k.to_string(), both_words[0].as_str().to_string())
            })
            .collect::<HashMap<String, String>>();
        let odd_words = map_obj
            .iter()
            .map(|item| {
                let (k, v): (&String, &Value) = item;
                let both_words: Vec<String> = v.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect();
                (k.to_string(), both_words[1].as_str().to_string())
            })
            .collect::<HashMap<String, String>>();

        PGPWordlist {
            _byte_even_words: even_words,
            _byte_odd_words: odd_words,
        }
    }

    pub fn get_completions(
        &self,
        prefix: &str,
        num_words: usize,
    ) -> HashSet<String> {
        let count_dashes = prefix.matches('-').count();
        let words;
        let mut completions: HashSet<String> = HashSet::new();

        if count_dashes % 2 == 0 {
            words = self._byte_odd_words
                .values()
                .map(String::to_string)
                .collect::<Vec<String>>();
        } else {
            words = self._byte_even_words
                .values()
                .map(String::to_string)
                .collect::<Vec<String>>();
        }

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
                    suffix.split_off(p as usize);
                    suffix.push_str(&word);
                }

                if count_dashes + 1 < num_words {
                    suffix.push_str("-");
                }

                completions.insert(suffix);
            }
        }

        completions
    }

    pub fn choose_words(&self, length: u8) -> String {
        let mut rnd: [u8; 1] = [0; 1];
        let mut words: Vec<String> = Vec::new();
        for i in 0..length {
            random_bytes(&mut rnd);
            let key = bytes_to_hexstr(&rnd).to_uppercase();
            if i % 2 == 0 {
                let word = self._byte_odd_words[&key].as_str();
                words.push(word.to_string());
            } else {
                let word = self._byte_even_words[&key].as_str();
                words.push(word.to_string());
            }
        }

        words.join("-")
    }
}

#[cfg(test)]
mod test {
    use super::PGPWordlist;
    use std::collections::HashSet;

    #[test]
    fn test_completions() {
        let w = PGPWordlist::new();
        let c = w.get_completions("ar", 2);
        assert_eq!(c.len(), 2);
        assert_eq!(
            c,
            vec!["article-".to_string(), "armistice-".to_string()]
                .iter()
                .collect::<HashSet<_>>()
        );
    }
}
