//! `:config` — the personal config, edited in place and saved whole
//! (ADR 0037).
//!
//! The modal edits a DRAFT of the user file. Theme, layout and context are
//! previewed on the review behind it as they change, and `esc` puts back what
//! was there. Keys and the agent are not previewed: a key that moved under the
//! reader's hands while they were still choosing it would be a trap, and the
//! agent has already run. `ctrl-s` writes the draft and applies all of it.
//!
//! **The modal's own keys are fixed.** It is where a broken `[keys]` table is
//! repaired, so it must not answer to the table it is repairing.

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use differential_engine::config::{
    Agent, Config, DEFAULT_TIMEOUT_SECS, DiffLayout, EditorCommand, KeysConfig, ReviewConfig,
    ThemeName,
};
use differential_engine::store::OsConfigSource;
use tui_input::backend::crossterm::to_input_request;
use tui_input::{Input, InputRequest};

use ratatui::widgets::ListState;

use crate::keymap::{self, KeymapError};

use super::draw::config_modal_area;
use super::*;

/// One editable line of the modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Agent,
    Timeout,
    Theme,
    Diff,
    Context,
    ContextStep,
    Editor,
    Key(Action),
}

impl Field {
    /// Every row, in the order the modal draws them.
    pub fn all() -> Vec<Field> {
        let mut rows = vec![
            Field::Agent,
            Field::Timeout,
            Field::Theme,
            Field::Diff,
            Field::Context,
            Field::ContextStep,
            Field::Editor,
        ];
        rows.extend(Action::ALL.iter().copied().map(Field::Key));
        rows
    }

    /// The section a row sits under, named as the file names its table.
    pub fn section(self) -> &'static str {
        match self {
            Field::Agent | Field::Timeout => "[grouping]",
            Field::Theme | Field::Diff | Field::Context | Field::ContextStep | Field::Editor => {
                "[review]"
            }
            Field::Key(_) => "[keys]",
        }
    }

    /// The row's name, as the file spells its key.
    pub fn label(self) -> &'static str {
        match self {
            Field::Agent => "agent",
            Field::Timeout => "timeout_secs",
            Field::Theme => "theme",
            Field::Diff => "diff",
            Field::Context => "context",
            Field::ContextStep => "context_step",
            Field::Editor => "editor",
            Field::Key(a) => a.key(),
        }
    }

    /// A multiple-choice row's choices, by the names the file writes; empty
    /// for a row that is typed. Each opens a list rather than stepping blind.
    pub fn choices(self) -> Vec<&'static str> {
        match self {
            Field::Agent => Agent::ALL.iter().map(|a| a.key()).collect(),
            Field::Theme => ThemeName::ALL.iter().map(|t| t.key()).collect(),
            Field::Diff => DiffLayout::ALL.iter().map(|d| d.key()).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether `enter` edits the row as text. The rest cycle.
    fn typed(self) -> bool {
        matches!(
            self,
            Field::Timeout | Field::Context | Field::ContextStep | Field::Editor | Field::Key(_)
        )
    }
}

/// The list a multiple-choice row opens: whose it is, which choice is on,
/// and the one to go back to. Every step through it sets the row, so a theme
/// or a layout is worn as it is passed — the list IS the preview.
///
/// The selection is ratatui's own `ListState`, which the `List` it is drawn
/// with reads: the widget owns the highlight and the scroll, and this owns
/// only what a step means.
pub struct Dropdown {
    pub field: Field,
    /// Over [`Field::choices`]; always `Some` while the list is open.
    pub state: ListState,
    before: usize,
}

impl Dropdown {
    /// The index into the row's choices the list is on.
    pub fn selected(&self) -> usize {
        self.state.selected().unwrap_or(0)
    }

    /// Step to `i`, clamped to the list.
    fn select(&mut self, i: usize) {
        let last = self.field.choices().len().saturating_sub(1);
        self.state.select(Some(i.min(last)));
    }

    /// The choice `esc` goes back to: the one on when the list opened.
    pub fn before(&self) -> usize {
        self.before
    }
}

/// The modal's state: the draft, what it started from, and where the reader
/// is in it.
pub struct ConfigEdit {
    pub draft: UserConfig,
    /// The config as the modal opened on it: what `esc` puts back, and what
    /// "changed" is measured against.
    pub original: UserConfig,
    pub selected: usize,
    pub scroll: usize,
    /// The selected row, being typed. `None` while moving.
    pub editing: Option<Input>,
    /// The theme list, open over the theme row. Every step through it wears
    /// the theme it lands on, so the list IS the preview.
    pub dropdown: Option<Dropdown>,
    /// What is wrong with the draft, in the words the CLI would print. Save
    /// is refused while this is not empty.
    pub problems: Vec<String>,
    /// Why the value just typed was refused. In the modal, not the status
    /// row: an error cut off at the edge of one line is an error half read.
    pub error: Option<String>,
}

impl ConfigEdit {
    pub fn field(&self) -> Field {
        Field::all()[self.selected]
    }

    /// A row's value as the modal writes it, and whether it is the default.
    pub fn value(&self, field: Field) -> (String, bool) {
        let d = &self.draft;
        match field {
            Field::Agent => match d.grouping.agent {
                Some(a) => (a.key().to_string(), false),
                None => (Agent::default().key().to_string(), true),
            },
            Field::Timeout => match d.grouping.timeout_secs {
                Some(t) => (t.to_string(), false),
                None => (DEFAULT_TIMEOUT_SECS.to_string(), true),
            },
            Field::Theme => (
                d.review.theme.key().to_string(),
                d.review.theme == ThemeName::default(),
            ),
            Field::Diff => (
                d.review.diff.key().to_string(),
                d.review.diff == DiffLayout::default(),
            ),
            Field::Context => {
                let def = ReviewConfig::default().context;
                (d.review.context.to_string(), d.review.context == def)
            }
            Field::ContextStep => {
                let def = ReviewConfig::default().context_step;
                (
                    d.review.context_step.to_string(),
                    d.review.context_step == def,
                )
            }
            Field::Editor => match &d.review.editor {
                Some(text) => (text.clone(), false),
                // Empty rather than the words "$VISUAL, then $EDITOR": this
                // string is also what `enter` puts in the box, and prose
                // typed back would not parse. The row's own note says it.
                None => (String::new(), true),
            },
            Field::Key(a) => match d.keys.0.get(&a) {
                Some(keys) => (KeysConfig::render_list(keys), false),
                None => (KeysConfig::render_list(&keymap::defaults(a)), true),
            },
        }
    }

    /// Which of a multiple-choice row's choices the draft holds.
    fn choice(&self, field: Field) -> usize {
        let d = &self.draft;
        match field {
            Field::Agent => {
                let a = d.grouping.agent.unwrap_or_default();
                Agent::ALL.iter().position(|x| *x == a)
            }
            Field::Theme => ThemeName::ALL.iter().position(|x| *x == d.review.theme),
            Field::Diff => DiffLayout::ALL.iter().position(|x| *x == d.review.diff),
            _ => None,
        }
        .unwrap_or(0)
    }

    /// Set a multiple-choice row to its choice `i`.
    fn set_choice(&mut self, field: Field, i: usize) {
        let d = &mut self.draft;
        match field {
            Field::Agent => {
                let a = Agent::ALL[i];
                // The default is written as no key at all, as a step writes it.
                d.grouping.agent = (a != Agent::default()).then_some(a);
            }
            Field::Theme => d.review.theme = ThemeName::ALL[i],
            Field::Diff => d.review.diff = DiffLayout::ALL[i],
            _ => {}
        }
    }

    /// The draft differs from what the modal opened on.
    pub fn dirty(&self) -> bool {
        self.draft != self.original
    }

    /// Check the draft's keys the way `dfr review` does before it opens.
    fn validate(&mut self) {
        self.problems = match Keymap::new(&self.draft.keys) {
            Ok(_) => Vec::new(),
            Err(KeymapError(problems)) => problems.iter().map(ToString::to_string).collect(),
        };
    }

    /// `h`/`l`: the next value of a cycling row, or a number stepped by one.
    fn step(&mut self, forward: bool) {
        fn cycle<T: Copy + PartialEq>(all: &[T], now: T, forward: bool) -> T {
            let i = all.iter().position(|x| *x == now).unwrap_or(0);
            let n = all.len();
            all[if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            }]
        }
        let field = self.field();
        let d = &mut self.draft;
        match field {
            Field::Agent => {
                let next = cycle(Agent::ALL, d.grouping.agent.unwrap_or_default(), forward);
                // The default is written as no key at all, so a file that never
                // named an agent does not start naming one by being cycled past.
                d.grouping.agent = (next != Agent::default()).then_some(next);
            }
            Field::Timeout => {
                let now = d.grouping.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
                // A minute: seconds are not how anyone thinks about a timeout
                // this long, and `enter` types an exact number.
                let next = match forward {
                    true => now.saturating_add(60),
                    false => now.saturating_sub(60).max(60),
                };
                d.grouping.timeout_secs = (next != DEFAULT_TIMEOUT_SECS).then_some(next);
            }
            Field::Theme => d.review.theme = cycle(ThemeName::ALL, d.review.theme, forward),
            Field::Diff => d.review.diff = cycle(DiffLayout::ALL, d.review.diff, forward),
            Field::Context => {
                d.review.context = match forward {
                    true => d.review.context.saturating_add(1),
                    false => d.review.context.saturating_sub(1),
                }
            }
            Field::ContextStep => {
                d.review.context_step = match forward {
                    true => d.review.context_step.saturating_add(1),
                    false => d.review.context_step.saturating_sub(1).max(1),
                }
            }
            // Neither has a next value; `enter` types one.
            Field::Editor | Field::Key(_) => {}
        }
    }

    /// `r`: the row back to its default — which, in the file, is its absence.
    fn reset(&mut self) {
        let field = self.field();
        let d = &mut self.draft;
        let def = ReviewConfig::default();
        match field {
            Field::Agent => d.grouping.agent = None,
            Field::Timeout => d.grouping.timeout_secs = None,
            Field::Theme => d.review.theme = def.theme,
            Field::Diff => d.review.diff = def.diff,
            Field::Context => d.review.context = def.context,
            Field::ContextStep => d.review.context_step = def.context_step,
            Field::Editor => d.review.editor = None,
            Field::Key(a) => {
                d.keys.0.remove(&a);
            }
        }
    }

    /// `enter` on a typed row: what it is typed as, the value it has now.
    fn begin_edit(&mut self) {
        let (text, _) = self.value(self.field());
        self.editing = Some(Input::new(text));
    }

    /// `enter` while typing: the text as the row's new value, or why not. A
    /// refused value keeps the box open on what was typed, so a slip costs a
    /// character and not the line.
    fn commit_edit(&mut self) -> Result<(), String> {
        let Some(input) = &self.editing else {
            return Ok(());
        };
        let text = input.value().trim().to_string();
        let number = |what: &str| -> Result<usize, String> {
            text.parse::<usize>()
                .map_err(|_| format!("{what} is a whole number, not {text:?}"))
        };
        let field = self.field();
        let d = &mut self.draft;
        match field {
            Field::Timeout => {
                let t = number("timeout_secs")? as u64;
                if t == 0 {
                    return Err("timeout_secs must be more than 0".into());
                }
                d.grouping.timeout_secs = (t != DEFAULT_TIMEOUT_SECS).then_some(t);
            }
            Field::Context => d.review.context = number("context")?,
            Field::ContextStep => {
                let n = number("context_step")?;
                if n == 0 {
                    return Err("context_step must be at least 1".into());
                }
                d.review.context_step = n;
            }
            // Empty is unset, which means the environment. Anything else has
            // to be a command this crate could actually run, so it is parsed
            // here rather than at the next press of `e`.
            Field::Editor => {
                d.review.editor = match text.is_empty() {
                    true => None,
                    false => {
                        EditorCommand::parse(&text, "editor").map_err(|e| e.to_string())?;
                        Some(text)
                    }
                };
            }
            Field::Key(a) => {
                let keys = KeysConfig::parse_list(&text).map_err(|e| {
                    format!("{}: {e} — write a list, as [\"j\", \"down\"]", a.key())
                })?;
                // The defaults, typed back, are the default: no override.
                match keys == keymap::defaults(a) {
                    true => d.keys.0.remove(&a),
                    false => d.keys.0.insert(a, keys),
                };
            }
            Field::Agent | Field::Theme | Field::Diff => {}
        }
        self.editing = None;
        Ok(())
    }

    /// Move the selection, keeping it inside `rows` visible lines.
    fn select(&mut self, to: usize, rows: usize) {
        let last = Field::all().len() - 1;
        self.selected = to.min(last);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if rows > 0 && self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }
}

impl App {
    /// `:config` — open the modal on the config as it is now.
    pub(super) fn open_config(&mut self) {
        let user = self.opts.user_config.clone();
        let mut edit = ConfigEdit {
            draft: user.clone(),
            original: user,
            selected: 0,
            scroll: 0,
            editing: None,
            dropdown: None,
            problems: Vec::new(),
            error: None,
        };
        edit.validate();
        self.visual = None;
        self.mode = Mode::Config(Box::new(edit));
    }

    pub fn config_edit(&self) -> Option<&ConfigEdit> {
        match &self.mode {
            Mode::Config(edit) => Some(edit),
            _ => None,
        }
    }

    fn config_edit_mut(&mut self) -> Option<&mut ConfigEdit> {
        match &mut self.mode {
            Mode::Config(edit) => Some(edit),
            _ => None,
        }
    }

    /// Put `review` on the screen: the palette, the context and the default
    /// layout. What the modal previews, what `esc` restores and what a save
    /// keeps all come through here, so the three cannot disagree.
    fn show_review(&mut self, review: &ReviewConfig) {
        let theme_changed = self.opts.theme != review.theme;
        self.opts.theme = review.theme;
        self.opts.context = review.context;
        self.opts.context_step = review.context_step;
        // A default: a layout this review recorded with `s` still wins, and
        // the modal's diff row says so.
        self.opts.split_diff = review.diff.is_split();
        // Not a preview of anything on screen — `e` is the next press that
        // could use it — but it goes through this one function for the reason
        // everything else does: the draft, the original and the saved config
        // must not be able to disagree. Clearing the row means the
        // environment, which the application layer handed over for exactly
        // this (ADR 0038).
        self.opts.editor = match &review.editor {
            Some(text) => EditorCommand::parse(text, "[review].editor").ok(),
            None => self.opts.editor_env.clone(),
        };
        match theme_changed {
            true => self.set_theme(Theme::named(review.theme)),
            false => self.rebuild_rows(),
        }
    }

    /// Whether this review recorded its own layout, which a default cannot
    /// move.
    pub fn layout_is_recorded(&self) -> bool {
        self.session.split_diff().is_some()
    }

    /// The rows the modal has room to show: what is left once the notes
    /// under the list have the lines they wrap to.
    pub(super) fn config_rows(&self) -> usize {
        let notes = self.config_edit().map_or(0, |e| {
            let lines = self.config_notes(e).len();
            if lines == 0 { 0 } else { lines + 1 }
        });
        config_list_rows(self.viewport.body_rows, self.config_top())
            .saturating_sub(notes)
            .max(3)
    }

    /// Lines above the list: the header, wrapped, and a blank under it.
    pub(super) fn config_top(&self) -> usize {
        self.config_header().len() + 1
    }

    /// The modal's headline: the file a save writes. Wrapped, a long path
    /// included — a home directory is exactly the path that does not fit.
    pub fn config_header(&self) -> Vec<String> {
        match &self.opts.user_config_path {
            Some(p) => self.config_wrap(&p.display().to_string()),
            None => self.config_wrap("no config directory · saving is off"),
        }
    }

    /// `text` wrapped to the modal's inner width, a word broken only when
    /// it is longer than a line (a path is one word).
    fn config_wrap(&self, text: &str) -> Vec<String> {
        let inner = usize::from(config_modal_area(self.panes().body).width.saturating_sub(2));
        let options = textwrap::Options::new(inner.saturating_sub(1).max(10))
            .initial_indent(" ")
            .subsequent_indent("   ")
            .break_words(true);
        textwrap::wrap(text, &options)
            .into_iter()
            .map(|line| line.into_owned())
            .collect()
    }

    /// What is wrong, wrapped to the modal's width: the value just refused,
    /// then every problem with the draft's keys. Every line is read to its
    /// end, which is the whole point of listing them.
    pub fn config_notes(&self, edit: &ConfigEdit) -> Vec<String> {
        edit.error
            .iter()
            .chain(&edit.problems)
            .flat_map(|note| self.config_wrap(note))
            .collect()
    }

    /// A key in the modal. Fixed keys, deliberately (see the module doc).
    ///
    /// Then the selection is kept in view against the rows left NOW: a note
    /// that appeared under the list takes rows from it, and the row the
    /// reader is on must not be one of the ones that went.
    pub(super) fn config_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let effects = self.config_key_inner(key);
        let rows = self.config_rows();
        if let Some(edit) = self.config_edit_mut() {
            edit.select(edit.selected, rows);
        }
        effects
    }

    fn config_key_inner(&mut self, key: KeyEvent) -> Vec<Effect> {
        let rows = self.config_rows();
        let Some(edit) = self.config_edit_mut() else {
            return Vec::new();
        };
        if let Some(list) = &mut edit.dropdown {
            let last = list.field.choices().len().saturating_sub(1);
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => list.select(list.selected() + 1),
                KeyCode::Char('k') | KeyCode::Up => list.select(list.selected().saturating_sub(1)),
                KeyCode::Char('g') | KeyCode::Home => list.select(0),
                KeyCode::Char('G') | KeyCode::End => list.select(last),
                KeyCode::Enter | KeyCode::Char(' ') => edit.dropdown = None,
                KeyCode::Esc | KeyCode::Char('q') => {
                    let (field, before) = (list.field, list.before);
                    edit.dropdown = None;
                    edit.set_choice(field, before);
                }
                _ => return Vec::new(),
            }
            if let Some((field, at)) = edit.dropdown.as_ref().map(|l| (l.field, l.selected())) {
                edit.set_choice(field, at);
            }
            let review = edit.draft.review.clone();
            self.show_review(&review);
            return Vec::new();
        }
        if let Some(input) = &mut edit.editing {
            match key.code {
                KeyCode::Esc => {
                    edit.editing = None;
                    edit.error = None;
                }
                KeyCode::Enter => match edit.commit_edit() {
                    Ok(()) => {
                        edit.error = None;
                        edit.validate();
                        let review = edit.draft.review.clone();
                        self.show_review(&review);
                    }
                    Err(why) => edit.error = Some(why),
                },
                _ => {
                    if let Some(req) = to_input_request(&CrosstermEvent::Key(key)) {
                        input.handle(req);
                    }
                }
            }
            return Vec::new();
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('s') if ctrl => {
                self.save_config();
                return Vec::new();
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.discard_config();
                return Vec::new();
            }
            KeyCode::Char('j') | KeyCode::Down => edit.select(edit.selected + 1, rows),
            KeyCode::Char('k') | KeyCode::Up => edit.select(edit.selected.saturating_sub(1), rows),
            KeyCode::Char('g') | KeyCode::Home => edit.select(0, rows),
            KeyCode::Char('G') | KeyCode::End => edit.select(usize::MAX, rows),
            KeyCode::Char('l') | KeyCode::Right => edit.step(true),
            KeyCode::Char('h') | KeyCode::Left => edit.step(false),
            // A multiple-choice row opens its list rather than stepping
            // blind: a choice is made by seeing the choices.
            KeyCode::Enter | KeyCode::Char(' ') if !edit.field().choices().is_empty() => {
                let field = edit.field();
                let at = edit.choice(field);
                edit.dropdown = Some(Dropdown {
                    field,
                    state: ListState::default().with_selected(Some(at)),
                    before: at,
                });
            }
            KeyCode::Enter | KeyCode::Char(' ') => match edit.field().typed() {
                true => edit.begin_edit(),
                false => edit.step(true),
            },
            KeyCode::Char('r') => edit.reset(),
            _ => return Vec::new(),
        }
        // Whatever the key changed, the draft is checked again and the review
        // behind the modal shows it.
        edit.validate();
        let review = edit.draft.review.clone();
        self.show_review(&review);
        Vec::new()
    }

    /// Text pasted into a row being typed.
    pub(super) fn config_paste(&mut self, text: &str) {
        if let Some(input) = self.config_edit_mut().and_then(|e| e.editing.as_mut()) {
            for c in text.lines().next().unwrap_or_default().chars() {
                input.handle(InputRequest::InsertChar(c));
            }
        }
    }

    /// The wheel and a click: a notch or a click moves the selection, and a
    /// click on the selected row is `enter`. Outside the box nothing happens
    /// — a draft is not something a stray click should throw away.
    pub(super) fn config_mouse(&mut self, line: Option<usize>, click: bool, step: isize) {
        let rows = self.config_rows();
        let top = self.config_top();
        let Some(edit) = self.config_edit_mut() else {
            return;
        };
        if edit.editing.is_some() {
            return;
        }
        // Over the theme list, the wheel walks it and a click keeps it.
        if edit.dropdown.is_some() {
            let code = match (step, click) {
                (1.., _) => KeyCode::Down,
                (..0, _) => KeyCode::Up,
                (0, true) => KeyCode::Enter,
                (0, false) => return,
            };
            let _ = self.config_key(KeyEvent::new(code, KeyModifiers::NONE));
            return;
        }
        if step != 0 {
            edit.select(edit.selected.saturating_add_signed(step), rows);
            return;
        }
        if click && let Some(hit) = line.and_then(|l| config_row_at(edit, l, top)) {
            if hit == edit.selected {
                let _ = self.config_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            } else {
                edit.select(hit, rows);
            }
        }
    }

    /// `esc`: drop the draft and put the screen back as it was.
    fn discard_config(&mut self) {
        let Mode::Config(edit) = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        self.show_review(&edit.original.review);
        self.status = match edit.dirty() {
            true => "config unchanged · nothing was saved".into(),
            false => String::new(),
        };
    }

    /// `ctrl-s`: write the draft over the user file and apply all of it.
    fn save_config(&mut self) {
        let Some(edit) = self.config_edit() else {
            return;
        };
        if !edit.problems.is_empty() {
            // The problems themselves are in the modal, wrapped: the status
            // row has one line and would cut them off.
            self.status = "not saved · fix the problems listed first".into();
            return;
        }
        let Some(path) = self.opts.user_config_path.clone() else {
            self.status = "not saved · no config directory to write to".into();
            return;
        };
        let draft = edit.draft.clone();
        let agent_changed = draft.grouping != edit.original.grouping;
        let keymap = Keymap::new(&draft.keys).expect("validated with the draft");
        if let Err(e) = Config::save_user(&OsConfigSource, &path, &draft) {
            self.status = format!("not saved · {e}");
            return;
        }
        self.mode = Mode::Normal;
        self.opts.keymap = keymap;
        self.pending.clear();
        self.opts.user_config = draft.clone();
        self.show_review(&draft.review);
        let mut status = format!("saved {}", path.display());
        if agent_changed {
            status.push_str(" · the agent applies from the next grouping run");
        }
        self.status = status;
    }
}

/// How many rows of the list fit, given the body's height and the `top`
/// lines above the list, before any notes: the frame, one line per section
/// rule, and the footer with a blank above it.
pub fn config_list_rows(body_rows: usize, top: usize) -> usize {
    body_rows.saturating_sub(2 + top + 3 + 2).max(3)
}

/// One line of the list, as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLine {
    /// The table the rows under it belong to: `[review]`.
    Section(&'static str),
    /// A setting, by its index in [`Field::all`].
    Row(usize),
}

/// The list's lines from the scroll position on: a section header before the
/// first row of each table, and at most `rows` settings.
pub fn config_lines(edit: &ConfigEdit, rows: usize) -> Vec<ConfigLine> {
    let mut out = Vec::new();
    let mut section = "";
    for (i, f) in Field::all().iter().enumerate().skip(edit.scroll).take(rows) {
        if f.section() != section {
            section = f.section();
            out.push(ConfigLine::Section(section));
        }
        out.push(ConfigLine::Row(i));
    }
    out
}

/// The field on list line `line`, counted from the list's first line.
fn config_row_at(edit: &ConfigEdit, line: usize, top: usize) -> Option<usize> {
    // The list starts under the header and a blank.
    let line = line.checked_sub(top)?;
    match config_lines(edit, usize::MAX).get(line)? {
        ConfigLine::Row(i) => Some(*i),
        ConfigLine::Section(_) => None,
    }
}
