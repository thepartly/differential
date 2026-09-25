//! One table of keys, read twice: by the footer and by `?`.
//!
//! The footer used to name ten keys and the help modal named them again, in a
//! different order and a different wording — a wall the reader stops seeing,
//! and two lists to keep in step. Both now ask [`App::area`] where the reader
//! is standing and read the acts for that place, so the short list on the
//! footer is a subset of the long one in the modal by construction.
//!
//! # Adding a place
//!
//! The table is declarative on purpose: a place is a list of rows, and every
//! consumer is derived from it. Four steps, and nothing else to keep in step:
//!
//! 1. Add a variant to [`Area`], and its words to [`Area::title`].
//! 2. Say when the reader is in it, in [`App::area_of`].
//! 3. Write its rows in [`App::acts_of`] — `on` for the one to three a
//!    footer shows, `quiet` for the rest. A row names ACTIONS, and its keys
//!    are read from the keymap; a new action also needs its default keys in
//!    `keymap::DEFAULTS`.
//!
//! A row's short words go on ONE footer, never two: the window's own footer
//! for a pane, and the modal's own footer for a modal ([`App::modal_hints`]).
//! A modal names its keys in the box the reader is looking at, so the window
//! footer under it holds the pills and `? help` and nothing else.
//! 4. Add the place to `spec/tui.md` and to this crate's README.
//!
//! What a footer button presses is the first key the row names, read from
//! the same keymap as the handler, so a row cannot name one key and press
//! another — and a key the reader rebound is the key the row names.
//!
//! Built at draw time from the model, so `draw` stays a pure function of it
//! and nothing here is state.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::keymap::{Action, Binding, Keymap, Screen};

use super::search::Reading;
use super::text::{Hint, Ink, joined};
use super::{App, Focus, Mode, ViewMode};

/// Where the reader is standing. Everything the two lists differ by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// The left pane: the reading plan, or the file tree.
    Plan,
    /// The diff pane, on an ordinary row.
    Diff,
    /// The diff pane, with a line selection open.
    Selecting,
    /// The diff pane, with the symbol float open. Not modal: `j`/`k` still
    /// move the cursor, and moving it is one of the ways out.
    Peeking,
    /// The diff pane, on a review thread the forge holds.
    Thread {
        own: bool,
        resolved: bool,
    },
    FileList,
    Findings,
    /// `/` — the search box. Every printable key types into it.
    Search,
    /// Writing a finding.
    Composer,
    /// A question only `y` answers: a publish, a comment's deletion.
    Question,
    /// Help itself, and a notice. Any key closes them, and that is the
    /// whole list.
    Reading,
}

impl Area {
    /// A modal takes every key it is given, so the keys that work in the
    /// review behind it do not work here — and naming them would be a lie.
    fn modal(self) -> bool {
        !matches!(
            self,
            Area::Plan | Area::Diff | Area::Selecting | Area::Peeking | Area::Thread { .. }
        )
    }

    /// What the help modal calls this place.
    fn title(self) -> &'static str {
        match self {
            Area::Plan => "the plan pane",
            Area::Diff => "the diff pane",
            Area::Selecting => "a line selection",
            Area::Peeking => "a symbol's declaration",
            Area::Thread { own: false, .. } => "a review thread",
            Area::Thread { own: true, .. } => "your own comment",
            Area::FileList => "the file list",
            Area::Findings => "the findings list",
            Area::Search => "the search",
            Area::Composer => "writing a finding",
            Area::Question => "the question",
            Area::Reading => "here",
        }
    }
}

/// One row of keys and what they do here: as the footer says it, and as `?`
/// does.
///
/// `footer` is the short words, and `None` keeps the row out of the footer —
/// a footer of everything is the wall this change is undoing.
pub struct Act {
    /// Every key the row's actions answer to, as `?` writes them.
    pub key: String,
    /// The first of each, as the footer writes them: a footer is short.
    pub short: String,
    /// What a click on the row's footer button presses: the first key of its
    /// first action, so a row cannot name one key and press another.
    pub presses: Vec<KeyEvent>,
    pub footer: Option<String>,
    pub help: String,
}

/// The key column of a row, before its words are added.
struct Keys {
    full: String,
    short: String,
    presses: Vec<KeyEvent>,
}

impl Keys {
    /// Keys that are not the reader's to bind: the composer's, the search
    /// box's, a question's. Written once, pressed as given.
    fn fixed(text: &str, presses: Vec<KeyEvent>) -> Keys {
        Keys {
            full: text.to_string(),
            short: text.to_string(),
            presses,
        }
    }

    fn footer(self, short: &str, help: &str) -> Act {
        Act {
            key: self.full,
            short: self.short,
            presses: self.presses,
            footer: Some(short.to_string()),
            help: help.to_string(),
        }
    }

    fn quiet(self, help: &str) -> Act {
        Act {
            key: self.full,
            short: self.short,
            presses: self.presses,
            footer: None,
            help: help.to_string(),
        }
    }
}

/// A bare key, pressed once.
fn bare(code: KeyCode) -> Vec<KeyEvent> {
    vec![KeyEvent::new(code, KeyModifiers::NONE)]
}

/// A bare character, pressed once.
fn press(c: char) -> Vec<KeyEvent> {
    bare(KeyCode::Char(c))
}

/// One row as a footer button. The words are the row's, and so are the keys
/// a click presses.
fn hint(act: &Act) -> Hint {
    Hint::button(
        &format!("{} ", act.short),
        act.footer.as_deref().unwrap_or_default(),
        act.presses.clone(),
    )
}

/// A titled run of acts in the help modal. The second one is under a rule.
pub struct HelpSection {
    pub title: &'static str,
    pub acts: Vec<Act>,
}

impl Keymap {
    /// The key column for a row of one action, or of a pair read as one —
    /// `down`/`up` is `j/k`. The pair's keys are zipped, so its alternates
    /// read as pairs too: `j/k · down/up`. `None` when the reader unbound
    /// every one of them, and the row goes: a row naming no key is a row
    /// that says to press nothing.
    fn keys(&self, screen: Screen, actions: &[Action]) -> Option<Keys> {
        let bound: Vec<&[Binding]> = actions.iter().map(|a| self.bindings(screen, *a)).collect();
        let longest = bound.iter().map(|b| b.len()).max().unwrap_or(0);
        if longest == 0 {
            return None;
        }
        let pair = |i: usize| -> String {
            let keys: Vec<String> = bound
                .iter()
                .filter_map(|b| b.get(i).map(ToString::to_string))
                .collect();
            keys.join("/")
        };
        let full: Vec<String> = (0..longest).map(pair).collect();
        let presses = bound
            .iter()
            .find_map(|b| b.first())
            .map(Binding::presses)
            .unwrap_or_default();
        Some(Keys {
            full: full.join(" · "),
            short: pair(0),
            presses,
        })
    }
}

impl Area {
    /// Whose keys this place reads: the review's, a list's, or none the
    /// reader binds.
    fn screen(self) -> Option<Screen> {
        match self {
            Area::Plan | Area::Diff | Area::Selecting | Area::Peeking | Area::Thread { .. } => {
                Some(Screen::Review)
            }
            Area::FileList => Some(Screen::FileList),
            Area::Findings => Some(Screen::Findings),
            Area::Search | Area::Composer | Area::Question | Area::Reading => None,
        }
    }
}

/// Getting about, in one place. Every one of these works from either pane,
/// so a reader hunting for "how do I move" reads one run of rows rather than
/// finding `j/k` under the place they are in and `g/G` three sections later.
///
/// `j/k` is the exception that proves it: it is the pane's, so it says what
/// it does in THIS pane. In a selection it is the selection's, and it stays
/// up in that place's own rows.
fn moving(keys: &Keymap, area: Area) -> Vec<Act> {
    let row =
        |actions: &[Action], help: &str| keys.keys(Screen::Review, actions).map(|k| k.quiet(help));
    let pane = match area {
        Area::Selecting => None,
        Area::Plan => row(&[Action::Down, Action::Up], "switch group"),
        _ => row(&[Action::Down, Action::Up], "move over rows"),
    };
    [
        pane,
        row(
            &[Action::NextGroup, Action::PrevGroup],
            "next / previous group",
        ),
        row(
            &[Action::NextHunk, Action::PrevHunk],
            "next / previous hunk",
        ),
        row(&[Action::HalfPageDown, Action::HalfPageUp], "half page"),
        row(&[Action::Top, Action::Bottom], "top / bottom"),
        row(&[Action::ToggleFocus], "switch pane focus"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The keys that mean the same thing wherever the reader stands in the
/// review. They are why the footer can be three keys long: a key that is
/// always there does not need saying on every row.
fn everywhere(keys: &Keymap) -> Vec<Act> {
    let row =
        |actions: &[Action], help: &str| keys.keys(Screen::Review, actions).map(|k| k.quiet(help));
    // `?` and `q` are fixed, whatever `[keys]` says (`keymap::RESERVED`).
    let help = Keys::fixed("?", press('?'));
    let quit = Keys::fixed("q · ctrl-c", press('q'));
    [
        row(&[Action::ToggleSplit], "unified / split diff"),
        row(&[Action::ToggleWrap], "soft wrap long lines"),
        row(
            &[Action::ShiftLeft, Action::ShiftRight],
            "shift the diff sideways",
        ),
        row(&[Action::ShiftReset], "back to the left edge"),
        row(
            &[Action::GrowDiff, Action::ShrinkDiff],
            "widen / narrow the diff pane",
        ),
        row(&[Action::Search], "find a word in any changed file"),
        row(&[Action::Findings], "every finding and thread, in one list"),
        row(&[Action::Copy], "copy the open findings"),
        row(&[Action::Publish], "publish the open findings (asks first)"),
        row(&[Action::Refetch], "fetch the review threads again"),
        Some(help.quiet("these keys")),
        Some(quit.quiet("quit — state is saved on every change")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

impl App {
    /// Where the reader is standing.
    pub fn area(&self) -> Area {
        self.area_of(&self.mode)
    }

    /// The area help is being read ABOUT: `?` was pressed somewhere, and that
    /// somewhere is what the modal answers for. The footer's own area stays
    /// `Reading` while the modal is up, which is what empties it.
    fn help_area(&self) -> Area {
        match &self.mode {
            Mode::Help(from) => self.area_of(from),
            other => self.area_of(other),
        }
    }

    fn area_of(&self, mode: &Mode) -> Area {
        match mode {
            Mode::Help(_) | Mode::Notice { .. } => Area::Reading,
            Mode::FileList { .. } => Area::FileList,
            Mode::Findings { .. } => Area::Findings,
            Mode::Search(_) => Area::Search,
            Mode::Editing { .. } => Area::Composer,
            Mode::Publish { .. } | Mode::DeleteComment { .. } => Area::Question,
            Mode::Normal => match self.focus {
                Focus::Groups => Area::Plan,
                // A selection is what the next key acts on, so it outranks
                // the row under the cursor.
                Focus::Detail if self.visual.is_some() => Area::Selecting,
                Focus::Detail if self.peek.is_some() => Area::Peeking,
                Focus::Detail => match self.thread_at_cursor() {
                    Some(t) => Area::Thread {
                        own: self.own_comment_at_cursor().is_some(),
                        resolved: t.resolved,
                    },
                    None => Area::Diff,
                },
            },
        }
    }

    /// Whether `?` is help's key here. In the composer it is a character,
    /// and in a `y` question every key but `y` is the answer no.
    pub(super) fn help_opens(&self) -> bool {
        match &self.mode {
            Mode::Normal | Mode::FileList { .. } => true,
            // A box the query owns takes `?` as a character, exactly as the
            // composer does — and a reader may well be searching for one.
            Mode::Search(_) => false,
            // While `D` waits for its answer, the next key IS the answer.
            Mode::Findings { confirming, .. } => !confirming,
            _ => false,
        }
    }

    /// Open help over where the reader is, keeping that place to come back to.
    pub(super) fn open_help(&mut self) {
        let from = std::mem::replace(&mut self.mode, Mode::Normal);
        self.mode = Mode::Help(Box::new(from));
    }

    /// The acts of the place the reader is standing in.
    pub fn acts(&self) -> Vec<Act> {
        self.acts_of(self.area())
    }

    /// The table. One row per key, the footer's one to three first: the
    /// footer takes them in this order and stops when the row runs out.
    fn acts_of(&self, area: Area) -> Vec<Act> {
        let keys = self.keymap();
        let screen = area.screen().unwrap_or(Screen::Review);
        let on = |action: Action, short: &str, help: &str| {
            keys.keys(screen, &[action]).map(|k| k.footer(short, help))
        };
        let quiet =
            |actions: &[Action], help: &str| keys.keys(screen, actions).map(|k| k.quiet(help));
        let rows: Vec<Option<Act>> = match area {
            // A label says what the key WILL do, not which view is already
            // on: a key named for where the reader already is reads as a key
            // that does nothing.
            Area::Plan => vec![
                on(
                    Action::Open,
                    "open",
                    "open the group or file in the diff pane",
                ),
                on(
                    Action::ToggleReviewed,
                    "reviewed",
                    "mark the whole group or file reviewed",
                ),
                match self.view_mode {
                    ViewMode::Files => on(Action::Files, "plan", "back to the reading plan"),
                    ViewMode::Groups => {
                        on(Action::Files, "tree", "the file tree instead of the plan")
                    }
                },
                quiet(
                    &[Action::Fold],
                    "unfold a skim remainder, a noise group or a directory",
                ),
            ],
            Area::Diff => vec![
                on(
                    Action::Comment,
                    "note",
                    "write a finding on this line, or on the hunk",
                ),
                on(
                    Action::ToggleReviewed,
                    "reviewed",
                    "mark this hunk's class reviewed",
                ),
                on(Action::Select, "select", "start a line selection here"),
                quiet(&[Action::Delete], "delete the finding under the cursor"),
                quiet(
                    &[Action::Fold],
                    "on a symbol: what declares it · on a boundary: more of the \
                     file, or the hunk it names",
                ),
                quiet(
                    &[Action::Files],
                    &match keys.first(Screen::FileList, Action::Open) {
                        Some(k) => format!("the file list ({k} jumps)"),
                        None => "the file list".to_string(),
                    },
                ),
            ],
            Area::Peeking => vec![
                on(Action::Fold, "next", "the next symbol on this line"),
                on(Action::Close, "close", "close the float"),
                quiet(
                    &[Action::Down, Action::Up],
                    "move on — the float closes with the cursor",
                ),
            ],
            Area::Selecting => vec![
                keys.keys(screen, &[Action::Down, Action::Up])
                    .map(|k| k.footer("extend", "extend the selection, up to a context boundary")),
                on(
                    Action::Comment,
                    "note",
                    "write one finding over the selected lines",
                ),
                on(Action::Close, "drop", "drop the selection"),
                quiet(&[Action::Select], "drops it too"),
            ],
            // A thread is the forge's. What the reader may do to it depends
            // on whether the comment under the cursor is theirs.
            Area::Thread { own, resolved } => [
                vec![
                    on(Action::Reply, "reply", "draft a reply under this thread"),
                    match resolved {
                        true => on(
                            Action::Resolve,
                            "reopen",
                            "reopen the thread on the forge, at once",
                        ),
                        false => on(
                            Action::Resolve,
                            "resolve",
                            "resolve the thread on the forge, at once",
                        ),
                    },
                ],
                match own {
                    true => vec![
                        on(Action::Comment, "edit", "rewrite your comment on the forge"),
                        on(
                            Action::Delete,
                            "delete",
                            "delete it there and here (asks first)",
                        ),
                    ],
                    // Two keys that do nothing here, named together so the
                    // reader who reaches for either finds out why.
                    false => {
                        let named: Vec<String> = [Action::Comment, Action::Delete]
                            .iter()
                            .filter_map(|a| keys.first(screen, *a).map(ToString::to_string))
                            .collect();
                        let help = match keys.says(screen, Action::Reply, "replies to it") {
                            Some(reply) => format!("not yours — {reply}"),
                            None => "not yours".to_string(),
                        };
                        vec![
                            (!named.is_empty())
                                .then(|| Keys::fixed(&named.join(" · "), Vec::new()).quiet(&help)),
                        ]
                    }
                },
                vec![quiet(&[Action::Fold], "open or close a resolved thread")],
            ]
            .into_iter()
            .flatten()
            .collect(),
            // These two draw their own footer from these same rows, so the
            // list a reader sees under the box is this list.
            Area::FileList => vec![
                on(Action::Open, "jump", "jump to the file"),
                keys.keys(screen, &[Action::Close]).map(|k| {
                    let also = keys.first(screen, Action::Files);
                    let help = match also {
                        Some(f) => format!("close the list · so do q and {f}"),
                        None => "close the list · so does q".to_string(),
                    };
                    k.footer("close", &help)
                }),
                quiet(&[Action::Down, Action::Up], "move over the files"),
            ],
            // The arrows, not `j`/`k`: every printable key types into the
            // query, which is the price of a box you can search a path in.
            Area::Search => vec![
                Some(
                    Keys::fixed("enter", bare(KeyCode::Enter))
                        .footer("open", "jump to the occurrence"),
                ),
                // On the footer, and it is the one key here that HAS to be:
                // `?` types in this box, so the help modal cannot be opened
                // from it and the footer is the only place a reader finds
                // this. A label says what the key WILL do, not which reading
                // is already on — the pill on the query row says that.
                Some({
                    let ctrl_r = Keys::fixed(
                        "ctrl-r",
                        vec![KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)],
                    );
                    match self.search_reading() {
                        Reading::Literal => {
                            ctrl_r.footer("regexp", "read the query as a regular expression")
                        }
                        Reading::Regexp => {
                            ctrl_r.footer("literal", "read the query as a literal again")
                        }
                    }
                }),
                Some(Keys::fixed("esc", bare(KeyCode::Esc)).footer("close", "close the search")),
                // No quiet rows, and they could not be read if there were.
                // This is the one place `?` is a character rather than help,
                // so `help_area` is never `Search` and nothing would ever
                // draw them. The box's other keys are written down in
                // `spec/tui.md` and this crate's README, which is where a
                // reader who cannot press `?` goes.
            ],
            Area::Findings => vec![
                on(Action::Open, "jump", "jump to the note or thread"),
                on(Action::Delete, "delete", "delete the selected note"),
                on(
                    Action::ClearNotes,
                    "clear local",
                    "clear the notes not on the request (asks first)",
                ),
                on(Action::Copy, "copy", "copy the open findings"),
                on(
                    Action::Publish,
                    "publish",
                    "publish the open findings (asks first)",
                ),
                keys.keys(screen, &[Action::Close]).map(|k| {
                    let help = match keys.first(screen, Action::Findings) {
                        Some(f) => format!("close the list · so do q and {f}"),
                        None => "close the list · so does q".to_string(),
                    };
                    k.footer("close", &help)
                }),
                quiet(&[Action::Down, Action::Up], "move over the list"),
            ],
            // The composer and the questions keep footers of their own: one
            // names a chord no single key sends, and the others are a
            // question rather than a list of keys. None of their keys is the
            // reader's to bind (ADR 0036).
            Area::Composer => vec![
                Some(Keys::fixed("enter · ctrl-s", Vec::new()).quiet("save the finding")),
                Some(
                    Keys::fixed("shift+enter", Vec::new())
                        .quiet("a new line · or a trailing \\ before enter"),
                ),
                Some(Keys::fixed("esc", Vec::new()).quiet("discard it")),
            ],
            Area::Question => vec![
                Some(Keys::fixed("y", Vec::new()).quiet("yes, and only y does")),
                Some(Keys::fixed("any other key", Vec::new()).quiet("no — nothing happens")),
            ],
            Area::Reading => vec![Some(Keys::fixed("any key", Vec::new()).quiet("close this"))],
        };
        rows.into_iter().flatten().collect()
    }

    /// The window footer's right edge: this place's short words, then
    /// `? help`, one hint per row and no separators — how many of them fit is
    /// the footer's arithmetic, and it drops them from the left.
    ///
    /// A modal's own words are on the modal, so under one this is `? help`
    /// alone. Empty where `?` cannot be pressed — a box the caret owns takes
    /// `?` as a character, and a `y` question takes it as its no.
    pub fn footer_hints(&self) -> Vec<Hint> {
        if !self.help_opens() {
            return Vec::new();
        }
        let own = match self.area().modal() {
            true => Vec::new(),
            false => self.modal_hints(),
        };
        let help = hint(&Keys::fixed("?", press('?')).footer("help", "these keys"));
        own.into_iter().chain([help]).collect()
    }

    /// A modal's own footer, exactly as drawn: this place's short words with
    /// a separator between them. The hit test reads the same list, so a
    /// click lands on the button the reader sees.
    pub fn modal_footer(&self) -> Vec<Hint> {
        // A lead of two columns, so the first key does not sit against the
        // border. It presses nothing: it is padding, and a click on padding
        // is a click on nothing.
        let mut hints = vec![Hint::note("  ", Ink::Dim)];
        hints.extend(joined(&self.modal_hints()));
        hints
    }

    /// The short words of this place, as buttons: what a modal draws on its
    /// own footer, and what the window footer draws when no modal is up.
    pub fn modal_hints(&self) -> Vec<Hint> {
        self.acts()
            .iter()
            .filter(|a| a.footer.is_some())
            .map(hint)
            .collect()
    }

    /// What `?` shows: this place's keys, and — outside a modal — the keys
    /// that work anywhere in the review, under a rule.
    pub fn help_sections(&self) -> Vec<HelpSection> {
        let area = self.help_area();
        let mut sections = vec![HelpSection {
            title: area.title(),
            acts: self.acts_of(area),
        }];
        if !area.modal() {
            // Acting, then moving: the reader who opens `?` is usually asking
            // what they can DO here, and the movement keys are the ones they
            // already know. Last is where a reference belongs.
            sections.push(HelpSection {
                title: "anywhere",
                acts: everywhere(self.keymap()),
            });
            sections.push(HelpSection {
                title: "moving",
                acts: moving(self.keymap(), area),
            });
        }
        sections
    }
}
