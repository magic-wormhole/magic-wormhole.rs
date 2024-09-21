use std::{borrow::Cow, process::exit};

use color_eyre::eyre::{self, bail};
use lazy_static::lazy_static;
use magic_wormhole::core::wordlist::{default_wordlist, Wordlist};
use nu_ansi_term::{Color, Style};
use reedline::{
    default_emacs_keybindings, ColumnarMenu, Completer, DefaultCompleter, Emacs, Highlighter,
    Hinter, History, KeyCode, KeyModifiers, MenuBuilder, Prompt, PromptEditMode,
    PromptHistorySearch, Reedline, ReedlineEvent, ReedlineMenu, Signal, StyledText, Suggestion,
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

    fn get_prompt_color(&self) -> reedline::Color {
        reedline::Color::Grey
    }
}

pub struct CodeHinter {}

impl CodeHinter {
    fn default() -> Self {
        CodeHinter {}
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
        _line.to_string()
    }

    fn complete_hint(&self) -> String {
        "accepted".to_string()
    }

    fn next_hint_token(&self) -> String {
        "test".to_string()
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
        let current_part = parts.last().unwrap_or(&"");
        let current_part_start = line[..pos].rfind('-').map(|i| i + 1).unwrap_or(0);

        let mut suggestions = Vec::new();

        if parts.len() == 1 {
            return suggestions;
        } else {
            // Completing a word
            for word_list in WORDLIST.words.iter() {
                for word in word_list.iter() {
                    if word.starts_with(current_part) {
                        suggestions.push(Suggestion {
                            value: word.to_string(),
                            description: None,
                            extra: None,
                            span: reedline::Span {
                                start: current_part_start,
                                end: pos,
                            },
                            append_whitespace: false,
                            style: None,
                        });
                    }
                }
            }
        }

        suggestions
    }
}

struct CodeHighliter {}

impl CodeHighliter {
    fn default() -> Self {
        CodeHighliter {}
    }

    fn is_valid_code(&self, code: &str) -> bool {
        let parts: Vec<&str> = code.split('-').collect();

        // if the first element in code is not a valid number
        if !parts
            .first()
            .and_then(|c| c.parse::<usize>().ok())
            .is_some_and(|c| (0..1000).contains(&c))
        {
            return false;
        }

        // check all words for validity
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
        let invalid = Style::new().fg(Color::Red);
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
            _ => {},
        }
    }
}
