use std::borrow::Cow;

use color_eyre::eyre::{self, bail};
use fuzzt::{algorithms::JaroWinkler, get_top_n, processors::NullStringProcessor};
use lazy_static::lazy_static;
use magic_wormhole::core::wordlist::{default_wordlist, Wordlist};
use nu_ansi_term::{Color, Style};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, Emacs, Highlighter, KeyCode, KeyModifiers,
    MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, Reedline, ReedlineEvent,
    ReedlineMenu, Signal, Span, StyledText, Suggestion,
};

lazy_static! {
    static ref WORDLIST: Wordlist = default_wordlist(2);
}

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

    // Not needed
    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    // Not needed
    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    // Not needed
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("... ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        _history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Grey
    }
}

struct CodeCompleter {}

impl CodeCompleter {
    fn default() -> Self {
        CodeCompleter {}
    }
}

impl Completer for CodeCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let parts: Vec<&str> = line.split('-').collect();

        // Skip autocomplete for the channel number (first part)
        if parts.len() <= 1 {
            return Vec::new();
        }

        // Find the start and end of the current word
        let current_word_start = line[..pos].rfind('-').map(|i| i + 1).unwrap_or(0);
        let current_word_end = line[pos..]
            .find('-')
            .map(|i| i + pos)
            .unwrap_or_else(|| line.len());

        let current_part = &line[current_word_start..current_word_end];

        // Flatten the word list
        let all_words: Vec<&str> = WORDLIST
            .words
            .iter()
            .flatten()
            .map(|s| s.as_str())
            .collect();

        // Use fuzzy matching to find the best matches
        let matches = get_top_n(
            current_part,
            &all_words,
            Some(0.8),
            Some(5),
            Some(&NullStringProcessor),
            Some(&JaroWinkler),
        );

        matches
            .into_iter()
            .map(|word| {
                let suggestion = word.to_string();

                // Incase suggestion word length is larger then the current typed part
                // Otherwise we get index out of range error in Span
                let span_end = if suggestion.len() >= current_part.len() {
                    current_word_end
                } else {
                    current_word_start + suggestion.len()
                };

                Suggestion {
                    value: suggestion,
                    description: None,
                    extra: None,
                    span: Span {
                        start: current_word_start,
                        end: span_end,
                    },
                    append_whitespace: false,
                    style: None,
                }
            })
            .collect()
    }
}

struct CodeHighliter {}

impl CodeHighliter {
    fn default() -> Self {
        CodeHighliter {}
    }

    fn is_valid_code(&self, code: &str) -> bool {
        let parts: Vec<&str> = code.split('-').collect();

        // If the first element in code is not a valid number
        if !parts
            .first()
            .and_then(|c| c.parse::<usize>().ok())
            .is_some_and(|c| (0..1000).contains(&c))
        {
            return false;
        }

        // Check all words for validity
        parts.iter().skip(1).all(|&word| {
            WORDLIST
                .words
                .iter()
                .flatten()
                .any(|valid_word| valid_word == word)
        })
    }
}

impl Highlighter for CodeHighliter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        let invalid = Style::new().fg(Color::Red).bold();
        let valid = Style::new().fg(Color::Green).bold();

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
    // Set up the required keybindings
    let mut keybindings = default_emacs_keybindings();

    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));

    let mut line_editor = Reedline::create()
        .with_completer(Box::new(CodeCompleter::default()))
        .with_highlighter(Box::new(CodeHighliter::default()))
        .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
        .with_quick_completions(true)
        .with_partial_completions(true)
        .with_edit_mode(edit_mode);

    let prompt = CodePrompt::default();

    loop {
        let sig = line_editor.read_line(&prompt);
        match sig {
            Ok(Signal::Success(buffer)) => return Ok(buffer),
            // TODO: fix temporary work around
            Ok(Signal::CtrlC) => bail!("Ctrl-C received"),
            Ok(Signal::CtrlD) => bail!("Ctrl-D received"),
            Err(e) => bail!(e),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_tab_compeltion_complete_word() {
        let mut completer = CodeCompleter::default();
        let input = "22-trombonist";
        let cursor_pos = input.len();

        let suggestions = completer.complete(&input, cursor_pos);

        assert_eq!(suggestions.len(), 3);

        assert_eq!(suggestions.first().unwrap().value, "trombonist");
    }

    #[test]
    fn test_tab_compeltion_partial_word() {
        let mut completer = CodeCompleter::default();
        let input = "22-trmbn";
        let cursor_pos = input.len();

        let suggestions = completer.complete(&input, cursor_pos);

        assert_eq!(suggestions.first().unwrap().value, "trombonist");
    }

    #[test]
    fn test_tab_compeltion_partial_in_middle() {
        let mut completer = CodeCompleter::default();
        let input = "22-trbis-zulu";
        let cursor_pos = input.len() - "-zulu".len();

        let suggestions = completer.complete(&input, cursor_pos);

        assert_eq!(suggestions.first().unwrap().value, "trombonist");
    }
}
