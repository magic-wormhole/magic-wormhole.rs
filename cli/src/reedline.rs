use std::{borrow::Cow, process::exit};

use color_eyre::eyre;
use magic_wormhole::core::wordlist::{default_wordlist, Wordlist};
use nu_ansi_term::{Color, Style};
use reedline::{
    Highlighter, Hinter, History, Prompt, PromptEditMode, PromptHistorySearch, Reedline, Signal,
    StyledText,
};

struct CodePrompt {}

impl CodePrompt {
    fn default() -> Self {
        CodePrompt {}
    }
}

impl Prompt for CodePrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("Wormhole Code: ")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    // Optionally override provided methods
    // fn get_prompt_color(&self) -> Color { ... }
    // fn get_prompt_multiline_color(&self) -> Color { ... }
    // fn get_indicator_color(&self) -> Color { ... }
    // fn get_prompt_right_color(&self) -> Color { ... }
    // fn right_prompt_on_last_line(&self) -> bool { ... }
}

pub struct CodeHinter {
    wordlist: Wordlist,
}

impl CodeHinter {
    fn default() -> Self {
        CodeHinter {
            wordlist: default_wordlist(2),
        }
    }
}

impl Hinter for CodeHinter {
    fn handle(
        &mut self,
        _line: &str,
        _pos: usize,
        _history: &dyn History,
        _use_ansi_coloring: bool,
        _cwd: &str,
    ) -> String {
        "".to_string()
    }

    fn complete_hint(&self) -> String {
        "".to_string()
    }

    fn next_hint_token(&self) -> String {
        "".to_string()
    }
}

struct CodeHighliter {
    wordlist: Wordlist,
}

impl CodeHighliter {
    fn default() -> Self {
        CodeHighliter {
            wordlist: default_wordlist(2),
        }
    }

    fn is_valid_code(&self, code: &str) -> bool {
        let words: Vec<&str> = code.split('-').collect();

        // if the first element in code is not a valid number
        if words.first().and_then(|w| w.parse::<u8>().ok()).is_none() {
            return false;
        }

        // check all words for validity
        words.iter().skip(1).all(|&word| {
            self.wordlist
                .words
                .iter()
                .flatten()
                .any(|valid_word| valid_word == word)
        })
    }
}

impl Highlighter for CodeHighliter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let invalid = Style::new().fg(Color::White);
        let valid = Style::new().fg(Color::Green);

        let style = match self.is_valid_code(line) {
            true => valid,
            false => invalid,
        };

        let mut t = StyledText::new();
        t.push((style, line.to_string()));
        t
    }
}

pub fn enter_code() -> eyre::Result<String> {
    let mut line_editor = Reedline::create().with_highlighter(Box::new(CodeHighliter::default()));
    let prompt = CodePrompt::default();

    loop {
        let sig = line_editor.read_line(&prompt);
        match sig {
            Ok(Signal::Success(buffer)) => return Ok(buffer),
            Ok(Signal::CtrlD) | Ok(Signal::CtrlC) => exit(0),
            _ => {},
        }
    }
}
