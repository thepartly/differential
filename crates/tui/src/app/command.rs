//! `:` — one line that names a command, as vim's does (ADR 0037).
//!
//! Every place the reviewer can take the reader to from anywhere has a name
//! here as well as a key: a reader who has forgotten `F` can still type
//! `:findings`, and the config modal, which has no key, is reached this way.
//!
//! One table, [`COMMANDS`], read by the dispatch, by the completion popup
//! and by `?`, so a command cannot be runnable without being listed or listed
//! without running.
//!
//! **Completion is shown, not guessed.** While the reader types a name, the
//! commands it could still become are listed above the line with what each
//! does, and the rest of the first is drawn dim after the caret. `tab` and the
//! arrows walk the list, filling the line as they go; `→` at the end takes the
//! dim rest. Nothing runs that the line does not say.
//!
//! The line's own keys are fixed, as the search box's are: every printable
//! key types into it.

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent};
use ratatui::widgets::ListState;
use tui_input::backend::crossterm::to_input_request;
use tui_input::{Input, InputRequest};

use super::*;

/// The line being typed, after the `:`.
#[derive(Default)]
pub struct CommandLine {
    pub input: Input,
    /// What the reader TYPED, which the candidates are matched against. The
    /// line itself changes as `tab` walks them, and matching against that
    /// would narrow the list to the one just filled in.
    typed: String,
    /// Which candidate `tab` or an arrow last filled the line with, as the
    /// `ListState` the popup's `List` is drawn from. Nothing selected until
    /// one does, and again after any edit.
    pub pick: ListState,
}

impl CommandLine {
    /// The commands the typed name could still become, in table order. None
    /// once the line has an argument: past the first space the text is the
    /// reader's own.
    pub fn candidates(&self) -> Vec<&'static Command> {
        if self.typed.contains(' ') {
            return Vec::new();
        }
        let word = self.typed.as_str();
        COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(word) || c.aliases.iter().any(|a| a.starts_with(word)))
            .collect()
    }

    /// The rest of the first candidate's name, drawn dim after the caret —
    /// only while the caret is at the end of what was typed and nothing has
    /// been picked, since then it is exactly what `→` would add.
    pub fn ghost(&self) -> Option<&'static str> {
        let value = self.input.value();
        if self.pick.selected().is_some()
            || value != self.typed
            || self.input.cursor() != value.chars().count()
        {
            return None;
        }
        let first = self.candidates().into_iter().next()?;
        first
            .name
            .strip_prefix(value)
            .filter(|rest| !rest.is_empty() && !value.is_empty())
    }

    /// Walk the candidates by `by`, filling the line with the one reached.
    fn walk(&mut self, by: isize) {
        let found = self.candidates();
        if found.is_empty() {
            return;
        }
        let n = found.len() as isize;
        let next = match self.pick.selected() {
            None if by > 0 => 0,
            None => n - 1,
            Some(p) => (p as isize + by).rem_euclid(n),
        } as usize;
        self.pick.select(Some(next));
        self.input = Input::new(found[next].name.to_string());
    }

    /// The reader changed the text: it is what they typed now.
    fn edited(&mut self) {
        self.typed = self.input.value().to_string();
        self.pick.select(None);
    }
}

/// One command: the name completion fills in, the others it answers to, and
/// the words the popup gives it.
pub struct Command {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub what: &'static str,
}

/// Every command, in the order `?` lists them and `tab` offers them.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "config",
        aliases: &[],
        what: "edit your config — theme, layout, context, keys, agent",
    },
    Command {
        name: "help",
        aliases: &[],
        what: "the keys, where you are",
    },
    Command {
        name: "findings",
        aliases: &[],
        what: "every finding and thread, in one list",
    },
    Command {
        name: "search",
        aliases: &[],
        what: "find a word in any changed file · :search <text>",
    },
    Command {
        name: "files",
        aliases: &[],
        what: "the files in the diff pane, in a list",
    },
    Command {
        name: "publish",
        aliases: &[],
        what: "publish the open findings (asks first)",
    },
    Command {
        name: "refetch",
        aliases: &[],
        what: "fetch the review threads again",
    },
    Command {
        name: "copy",
        aliases: &[],
        what: "copy the open findings",
    },
    Command {
        name: "quit",
        aliases: &["q"],
        what: "quit — state is saved on every change",
    },
];

/// The command `word` names, by its name or an alias.
fn find(word: &str) -> Option<&'static Command> {
    COMMANDS
        .iter()
        .find(|c| c.name == word || c.aliases.contains(&word))
}

impl App {
    /// Open the line. From a list it takes the list's place: every command
    /// goes somewhere else, and `esc` goes back to the review.
    pub(super) fn open_command(&mut self) {
        self.visual = None;
        self.mode = Mode::Command(CommandLine::default());
    }

    fn command_line(&mut self) -> Option<&mut CommandLine> {
        match &mut self.mode {
            Mode::Command(line) => Some(line),
            _ => None,
        }
    }

    /// The line, while it is open.
    pub fn command(&self) -> Option<&CommandLine> {
        match &self.mode {
            Mode::Command(line) => Some(line),
            _ => None,
        }
    }

    /// A key on the line: `enter` runs it, `esc` drops it, `tab` and the
    /// arrows walk the candidates, `→` at the end takes the dim rest, and
    /// everything else edits.
    pub(super) fn command_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(line) = self.command_line() else {
            return Vec::new();
        };
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                let text = line.input.value().to_string();
                self.mode = Mode::Normal;
                return self.run_command(&text);
            }
            KeyCode::Tab | KeyCode::Down => line.walk(1),
            KeyCode::BackTab | KeyCode::Up => line.walk(-1),
            KeyCode::Right if line.ghost().is_some() => {
                let name = line.candidates()[0].name;
                line.input = Input::new(name.to_string());
                line.edited();
            }
            // `backspace` on an empty line leaves it, as vim's does: the `:`
            // is the last character there is to take back.
            KeyCode::Backspace if line.input.value().is_empty() => self.mode = Mode::Normal,
            _ => {
                if let Some(req) = to_input_request(&CrosstermEvent::Key(key)) {
                    let before = line.input.value().to_string();
                    line.input.handle(req);
                    if line.input.value() != before {
                        line.edited();
                    }
                }
            }
        }
        Vec::new()
    }

    /// Text pasted onto the line: its first line, at the caret.
    pub(super) fn command_paste(&mut self, text: &str) {
        if let Some(line) = self.command_line() {
            for c in text.lines().next().unwrap_or_default().chars() {
                line.input.handle(InputRequest::InsertChar(c));
            }
            line.edited();
        }
    }

    /// Run `text` — `findings`, `search foo`, `q`. An empty line does
    /// nothing; a name nobody has says so and points at where they are
    /// listed.
    pub(super) fn run_command(&mut self, text: &str) -> Vec<Effect> {
        let text = text.trim();
        let (word, arg) = text.split_once(' ').unwrap_or((text, ""));
        let arg = arg.trim();
        if word.is_empty() {
            return Vec::new();
        }
        let Some(command) = find(word) else {
            self.status = format!("no command :{word} · :help lists them");
            return Vec::new();
        };
        match command.name {
            "config" => self.open_config(),
            "help" => self.open_help(),
            "findings" => self.open_findings(),
            "search" => {
                self.open_search();
                if !arg.is_empty() {
                    self.search_paste(arg);
                }
            }
            "files" => self.open_file_list(),
            "publish" => self.offer_publish(),
            "refetch" => self.start_fetch(),
            // The clipboard is the loop's to touch, as it is for `y`.
            "copy" => return vec![Effect::CopySummary(self.findings_summary())],
            "quit" => {
                self.save_cursor();
                return vec![Effect::Quit];
            }
            other => unreachable!("{other} is in COMMANDS and not handled"),
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases_are_unique() {
        let mut all: Vec<&str> = COMMANDS
            .iter()
            .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
            .collect();
        let before = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), before, "two commands share a name");
    }

    fn typed(text: &str) -> CommandLine {
        let mut line = CommandLine {
            input: Input::new(text.to_string()),
            ..CommandLine::default()
        };
        line.edited();
        line
    }

    fn names(line: &CommandLine) -> Vec<&'static str> {
        line.candidates().iter().map(|c| c.name).collect()
    }

    #[test]
    fn the_candidates_are_what_the_typed_name_could_become() {
        assert_eq!(
            names(&typed("")).len(),
            COMMANDS.len(),
            "empty lists them all"
        );
        assert_eq!(names(&typed("f")), ["findings", "files"]);
        assert_eq!(names(&typed("fil")), ["files"]);
        assert_eq!(names(&typed("q")), ["quit"], "an alias matches its command");
        assert!(names(&typed("x")).is_empty());
        assert!(
            names(&typed("search foo")).is_empty(),
            "an argument is not a name"
        );
        assert!(find("q").is_some_and(|c| c.name == "quit"));
    }

    #[test]
    fn the_ghost_is_the_rest_of_the_first_candidate() {
        assert_eq!(typed("fi").ghost(), Some("ndings"));
        assert_eq!(typed("files").ghost(), None, "nothing left to add");
        assert_eq!(typed("").ghost(), None, "no name begun, no guess");
    }

    #[test]
    fn walking_fills_the_line_and_keeps_the_typed_list() {
        let mut line = typed("f");
        line.walk(1);
        assert_eq!(line.input.value(), "findings");
        line.walk(1);
        assert_eq!(
            line.input.value(),
            "files",
            "still walking what `f` matched"
        );
        line.walk(1);
        assert_eq!(line.input.value(), "findings", "and round again");
        line.walk(-1);
        assert_eq!(line.input.value(), "files");
        assert_eq!(line.ghost(), None, "a picked name has no ghost");
    }
}
