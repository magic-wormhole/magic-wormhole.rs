use std::sync::LazyLock;

use color_eyre::eyre;
use dialoguer::{Completion, Input};
use magic_wormhole::Wordlist;

static WORDLIST: LazyLock<Wordlist> = LazyLock::new(|| Wordlist::default_wordlist(2));

struct CustomCompletion {}

impl CustomCompletion {
    pub fn default() -> Self {
        CustomCompletion {}
    }
}

impl Completion for CustomCompletion {
    fn get(&self, input: &str) -> Option<String> {
        WORDLIST
            .get_completions(input)
            .map(|list| list.first().cloned())
            .flatten()
    }
}

pub fn enter_code() -> eyre::Result<String> {
    let custom_completion = CustomCompletion::default();

    Input::new()
        .with_prompt("Wormhole Code")
        .completion_with(&custom_completion)
        .interact_text()
        .map(|code: String| code.trim().to_string())
        .map_err(From::from)
}
