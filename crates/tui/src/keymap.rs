//! Which key does what: the reviewer's actions, their default keys, and the
//! reader's `[keys]` laid over them (ADR 0036).
//!
//! One table serves the dispatch, the footer and `?`. A key the handler
//! answers to and a key the help names are read from the same place, so the
//! two cannot disagree — which they could while the help table held its keys
//! as display strings and the handler held them as `match` arms.
//!
//! [`Keymap::new`] is the whole check, and it is a library call on purpose:
//! it touches no terminal and no file, so anything that wants to know whether
//! a `[keys]` table is good can ask without opening a reviewer.

use std::fmt;

use crokey::KeyCombination;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub use differential_engine::config::{Action, KeysConfig};

/// Where a key is looked up. Two actions may share a key only in different
/// screens: `f` opens the file list in the review and closes it in the list.
///
/// Coarser than the help's areas on purpose. Within the review a key means
/// one action whichever pane has focus, and the action decides what it does
/// there — so `l` is taken in the plan pane too, although it only moves the
/// diff. A key that meant one thing on one row and another on the next would
/// be a key nobody could rebind with confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Screen {
    /// Both panes of the review proper.
    Review,
    /// The file list over the diff pane.
    FileList,
    /// The findings list.
    Findings,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::Review, Screen::FileList, Screen::Findings];

    /// What an error message calls this screen.
    pub fn title(self) -> &'static str {
        match self {
            Screen::Review => "the review",
            Screen::FileList => "the file list",
            Screen::Findings => "the findings list",
        }
    }
}

/// The default keys, and — by which screens each row names — where each
/// action works at all. An action the reader rebinds keeps its screens and
/// takes the reader's keys in every one of them.
///
/// `close` is two rows because it is two keys in a list and one in the
/// review, where `q` quits. `files` and `findings` work in their own lists
/// because the key that opened a list is the key a hand reaches for to shut
/// it.
const DEFAULTS: &[(Action, &[Screen], &[&str])] = {
    use Screen::*;
    const ALL: &[Screen] = &[Review, FileList, Findings];
    &[
        (Action::ToggleFocus, &[Review], &["tab"]),
        (Action::Open, ALL, &["enter"]),
        (Action::Close, ALL, &["esc"]),
        (Action::Down, ALL, &["j", "down"]),
        (Action::Up, ALL, &["k", "up"]),
        (Action::NextGroup, &[Review], &["J", "}"]),
        (Action::PrevGroup, &[Review], &["K", "{"]),
        (Action::HalfPageDown, &[Review], &["ctrl-d"]),
        (Action::HalfPageUp, &[Review], &["ctrl-u"]),
        (Action::Top, &[Review], &["g"]),
        (Action::Bottom, &[Review], &["G"]),
        (Action::NextHunk, &[Review], &["n"]),
        (Action::PrevHunk, &[Review], &["N"]),
        (Action::ToggleSplit, &[Review], &["s"]),
        (Action::ToggleWrap, &[Review], &["w"]),
        (Action::ShiftRight, &[Review], &["l", "right"]),
        (Action::ShiftLeft, &[Review], &["h", "left"]),
        (Action::ShiftReset, &[Review], &["0"]),
        // `+` is shifted `=` on most keyboards, so both are one key to a hand.
        (Action::GrowDiff, &[Review], &["alt-=", "alt-+"]),
        (Action::ShrinkDiff, &[Review], &["alt--"]),
        (Action::Fold, &[Review], &["z"]),
        (Action::Files, &[Review, FileList], &["f"]),
        (Action::Findings, &[Review, Findings], &["F"]),
        (Action::Search, ALL, &["/"]),
        (Action::ToggleReviewed, &[Review], &["space"]),
        (Action::Select, &[Review], &["v"]),
        (Action::Comment, &[Review], &["c"]),
        (Action::Delete, &[Review, Findings], &["d d"]),
        (Action::ClearNotes, &[Findings], &["D"]),
        (Action::Copy, &[Review, Findings], &["y"]),
        (Action::Reply, &[Review], &["r"]),
        (Action::Resolve, &[Review], &["x"]),
        (Action::Refetch, &[Review], &["R"]),
        (Action::Publish, &[Review, Findings], &["P"]),
    ]
};

/// Keys no action may take, and why. Each is a way out or a way to the
/// answer, and a reader who is lost is exactly the reader a moved one fails:
///
/// - `ctrl-c` quits from everywhere, the composer included.
/// - `q` quits the review, and closes a list back to it.
/// - `?` opens the keys of where the reader is standing — the one place a
///   reader who rebound something can find out what they did.
pub const RESERVED: &[(&str, &str)] = &[
    ("ctrl-c", "it quits from everywhere"),
    ("q", "it quits the review and closes a list"),
    ("?", "it opens help from everywhere"),
];

/// The reserved key `binding` is, and why, if it is one.
pub fn reserved(binding: &Binding) -> Option<(&'static str, &'static str)> {
    RESERVED
        .iter()
        .copied()
        .find(|(key, _)| Binding::parse(key).as_ref() == Ok(binding))
}

/// One binding: the presses that make it, in order. Almost always one; `dd`
/// is two.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binding(Vec<KeyCombination>);

impl Binding {
    /// Parse a binding as `[keys]` writes it: one key (`"j"`, `"ctrl-d"`,
    /// `"alt-="`, `"enter"`), or several separated by spaces (`"d d"`).
    ///
    /// A letter is its case: `"J"` is shift-j, as the reader sees it on the
    /// screen and as the terminal reports it.
    pub fn parse(text: &str) -> Result<Binding, String> {
        let presses = text
            .split_whitespace()
            .map(parse_press)
            .collect::<Result<Vec<_>, _>>()?;
        if presses.is_empty() {
            return Err("an empty key".into());
        }
        Ok(Binding(presses))
    }

    /// The events a click on a footer button naming this binding sends,
    /// through the same handler a hand would reach.
    pub fn presses(&self) -> Vec<KeyEvent> {
        self.0.iter().map(|&k| k.into()).collect()
    }

    fn starts_with(&self, other: &Binding) -> bool {
        self.0.starts_with(&other.0)
    }
}

/// `dd` rather than `d d`: a run of plain characters reads as the vim it is.
/// Anything with a named key or a modifier keeps its spaces.
impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plain = self.0.len() > 1
            && self.0.iter().all(|k| {
                k.modifiers.is_empty() && matches!(k.codes.first(), KeyCode::Char(c) if *c != ' ')
            });
        let sep = if plain { "" } else { " " };
        let words: Vec<String> = self.0.iter().map(press_text).collect();
        f.write_str(&words.join(sep))
    }
}

/// One press, parsed by crokey and put in the one shape a lookup compares.
///
/// crokey lowercases the whole string before it reads it, so `"J"` would
/// come back as `j` — and an uppercase letter is how every reader writes a
/// shifted one. The letter is said as `shift-` first, which crokey keeps.
fn parse_press(text: &str) -> Result<KeyCombination, String> {
    let shifted;
    let text = match text.char_indices().next_back() {
        Some((i, c)) if c.is_ascii_uppercase() && (i == 0 || text[..i].ends_with('-')) => {
            shifted = format!("{}shift-{}", &text[..i], c.to_ascii_lowercase());
            shifted.as_str()
        }
        _ => text,
    };
    let key = crokey::parse(text).map_err(|_| {
        let run = text.chars().count() > 1 && !text.contains('-');
        match run {
            true => format!("{text:?} is not a key; a sequence is written with spaces, as \"d d\""),
            false => format!("{text:?} is not a key"),
        }
    })?;
    // A combination of two keys at once (`ctrl-a-b`) is something a
    // terminal cannot report: it sends one key at a time.
    if !key.is_ansi_compatible() {
        return Err(format!(
            "{text:?} presses keys together, and a terminal cannot report that"
        ));
    }
    Ok(canon(key))
}

/// The one shape a key is compared in. A character carries its own shift —
/// `J`, `?`, `+` — so the modifier is dropped from every character, where
/// terminals disagree about sending it. A named key keeps it: shift-tab is
/// not tab.
fn canon(key: KeyCombination) -> KeyCombination {
    let mut key = key.normalized();
    if matches!(key.codes.first(), KeyCode::Char(_)) {
        key.modifiers.remove(KeyModifiers::SHIFT);
    }
    key
}

/// A press as the help and the footer write it, which is how `[keys]` spells
/// it too, so a reader can copy a key off the screen into the file.
///
/// Hand-written rather than crokey's formatter: that one writes `Ctrl-`,
/// `Hyphen` and `Down`, and none of them is what a reader types here.
fn press_text(key: &KeyCombination) -> String {
    let code = *key.codes.first();
    let mut text = String::new();
    for (m, name) in [
        (KeyModifiers::CONTROL, "ctrl-"),
        (KeyModifiers::ALT, "alt-"),
        (KeyModifiers::SUPER, "cmd-"),
    ] {
        if key.modifiers.contains(m) {
            text.push_str(name);
        }
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) && code != KeyCode::BackTab {
        text.push_str("shift-");
    }
    match code {
        KeyCode::Char(' ') => text.push_str("space"),
        KeyCode::Char(c) => text.push(c),
        KeyCode::BackTab => text.push_str("shift-tab"),
        KeyCode::PageUp => text.push_str("pageup"),
        KeyCode::PageDown => text.push_str("pagedown"),
        KeyCode::F(n) => text.push_str(&format!("f{n}")),
        other => text.push_str(&format!("{other:?}").to_lowercase()),
    }
    text
}

/// What a key means, given the presses before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// A whole binding: do this.
    Act(Action),
    /// The start of a binding: keep these presses and wait for the next.
    Pending(Vec<KeyCombination>),
    /// Nothing here.
    Nothing,
}

/// One thing wrong with a `[keys]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyProblem {
    /// A key string that is not a key.
    Unparsed { action: Action, why: String },
    /// A key no action may take.
    Reserved {
        action: Action,
        key: String,
        why: &'static str,
    },
    /// Two actions answer to one key in one screen — or one's key is the
    /// start of the other's, so the longer one could never be pressed.
    Clash {
        screen: Screen,
        first: (Action, String),
        second: (Action, String),
    },
}

impl fmt::Display for KeyProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyProblem::Unparsed { action, why } => write!(f, "{}: {why}", action.key()),
            KeyProblem::Reserved { action, key, why } => {
                write!(f, "{}: {key:?} is reserved — {why}", action.key())
            }
            KeyProblem::Clash {
                screen,
                first: (a, ka),
                second: (b, kb),
            } if ka == kb => write!(
                f,
                "{ka:?} is bound to both {} and {} in {}",
                a.key(),
                b.key(),
                screen.title()
            ),
            KeyProblem::Clash {
                screen,
                first: (a, ka),
                second: (b, kb),
            } => write!(
                f,
                "{ka:?} ({}) starts {kb:?} ({}) in {}, so {kb:?} could never be pressed",
                a.key(),
                b.key(),
                screen.title()
            ),
        }
    }
}

/// Every problem with a `[keys]` table, not just the first: a reader fixing
/// a file should not have to run the reviewer once per mistake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapError(pub Vec<KeyProblem>);

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_slice() {
            [one] => write!(f, "[keys]: {one}"),
            many => {
                write!(f, "[keys]: {} problems", many.len())?;
                for p in many {
                    write!(f, "\n  {p}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for KeymapError {}

/// Every action's keys in every screen it works in.
#[derive(Debug, Clone)]
pub struct Keymap {
    /// `(screen, action, keys)`, in the order of [`DEFAULTS`], so the help
    /// lists a binding the way the table does.
    bound: Vec<(Screen, Action, Vec<Binding>)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap::new(&KeysConfig::default()).expect("the default keys have no problems")
    }
}

impl Keymap {
    /// The defaults with the reader's `[keys]` laid over them, or every
    /// reason that cannot be done.
    ///
    /// An action the table names takes exactly the keys it gives, in every
    /// screen the action works in; `[]` unbinds it. A clash is an error that
    /// names both actions, never a quiet winner: a reader who bound `j` to
    /// something else and still sees it move has been told nothing.
    pub fn new(config: &KeysConfig) -> Result<Keymap, KeymapError> {
        let mut problems = Vec::new();
        let mut parsed = |action: Action, texts: &[&str]| -> Vec<Binding> {
            let mut out: Vec<Binding> = Vec::new();
            for text in texts {
                match Binding::parse(text) {
                    Ok(b) => match reserved(&b) {
                        Some((_, why)) => problems.push(KeyProblem::Reserved {
                            action,
                            key: text.to_string(),
                            why,
                        }),
                        None if !out.contains(&b) => out.push(b),
                        None => {}
                    },
                    Err(why) => problems.push(KeyProblem::Unparsed { action, why }),
                }
            }
            out
        };
        let mut bound = Vec::new();
        for (action, screens, defaults) in DEFAULTS {
            let own: Option<Vec<&str>> = config
                .0
                .get(action)
                .map(|keys| keys.iter().map(String::as_str).collect());
            let keys = parsed(*action, own.as_deref().unwrap_or(defaults));
            bound.extend(screens.iter().map(|s| (*s, *action, keys.clone())));
        }
        // An action on two rows of the table is parsed twice, and its
        // problems are the same problems. Kept once each, in order, wherever
        // the rows sit — `dedup` alone would only merge neighbours.
        let mut seen = Vec::new();
        problems.retain(|p| {
            let new = !seen.contains(p);
            if new {
                seen.push(p.clone());
            }
            new
        });
        for screen in Screen::ALL {
            let here: Vec<(Action, &Binding)> = bound
                .iter()
                .filter(|(s, ..)| *s == screen)
                .flat_map(|(_, a, keys)| keys.iter().map(move |k| (*a, k)))
                .collect();
            for (i, (a, ka)) in here.iter().enumerate() {
                for (b, kb) in &here[i + 1..] {
                    if a == b {
                        continue;
                    }
                    let (first, second) = match (kb.starts_with(ka), ka.starts_with(kb)) {
                        (true, _) => ((*a, ka), (*b, kb)),
                        (_, true) => ((*b, kb), (*a, ka)),
                        _ => continue,
                    };
                    problems.push(KeyProblem::Clash {
                        screen,
                        first: (first.0, first.1.to_string()),
                        second: (second.0, second.1.to_string()),
                    });
                }
            }
        }
        match problems.is_empty() {
            true => Ok(Keymap { bound }),
            false => Err(KeymapError(problems)),
        }
    }

    /// The keys `action` answers to in `screen`, first the one to show.
    /// Empty where it does not work, or where the reader unbound it.
    pub fn bindings(&self, screen: Screen, action: Action) -> &[Binding] {
        self.bound
            .iter()
            .find(|(s, a, _)| *s == screen && *a == action)
            .map(|(.., keys)| keys.as_slice())
            .unwrap_or_default()
    }

    /// The key to name for `action` in `screen`, if it has one.
    pub fn first(&self, screen: Screen, action: Action) -> Option<&Binding> {
        self.bindings(screen, action).first()
    }

    /// `"<key> <what>"` for a hint in a message — `"w turns it off"` — or
    /// `None` when the reader unbound the key, so a message never tells them
    /// to press something that does nothing.
    pub fn says(&self, screen: Screen, action: Action, what: &str) -> Option<String> {
        self.first(screen, action).map(|k| format!("{k} {what}"))
    }

    /// What `key` does in `screen`, after the presses in `pending`.
    ///
    /// A press that neither finishes nor continues a sequence is read again
    /// on its own: `d` then `j` is a `j`, as it always was.
    pub fn lookup(&self, screen: Screen, pending: &[KeyCombination], key: KeyEvent) -> Lookup {
        let mut seq = pending.to_vec();
        seq.push(canon(key.into()));
        let mut prefix = false;
        for (s, action, keys) in &self.bound {
            if *s != screen {
                continue;
            }
            for k in keys {
                if k.0 == seq {
                    return Lookup::Act(*action);
                }
                prefix |= k.0.starts_with(&seq);
            }
        }
        match (prefix, pending.is_empty()) {
            (true, _) => Lookup::Pending(seq),
            (false, true) => Lookup::Nothing,
            (false, false) => self.lookup(screen, &[], key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(pairs: &[(Action, &[&str])]) -> KeysConfig {
        KeysConfig(
            pairs
                .iter()
                .map(|(a, ks)| (*a, ks.iter().map(|k| k.to_string()).collect()))
                .collect(),
        )
    }

    fn press(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    fn ch(c: char) -> KeyEvent {
        press(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn act(map: &Keymap, screen: Screen, key: KeyEvent) -> Lookup {
        map.lookup(screen, &[], key)
    }

    #[test]
    fn the_defaults_have_no_problems() {
        Keymap::new(&KeysConfig::default()).unwrap();
    }

    #[test]
    fn every_action_has_a_default_somewhere() {
        // An action with no row would parse in `[keys]` and do nothing
        // anywhere, which is a knob that looks like it works.
        for a in Action::ALL {
            assert!(
                DEFAULTS.iter().any(|(d, ..)| *d == a),
                "{} has no default row",
                a.key()
            );
        }
    }

    #[test]
    fn the_defaults_answer_as_the_reviewer_always_did() {
        let map = Keymap::default();
        let r = Screen::Review;
        let cases = [
            (ch('j'), Action::Down),
            (press(KeyCode::Down, KeyModifiers::NONE), Action::Down),
            (ch('J'), Action::NextGroup),
            // The terminal's spelling of the same press.
            (
                press(KeyCode::Char('J'), KeyModifiers::SHIFT),
                Action::NextGroup,
            ),
            (
                press(KeyCode::Char('j'), KeyModifiers::SHIFT),
                Action::NextGroup,
            ),
            (ch('}'), Action::NextGroup),
            (
                press(KeyCode::Char('d'), KeyModifiers::CONTROL),
                Action::HalfPageDown,
            ),
            (
                press(KeyCode::Char('='), KeyModifiers::ALT),
                Action::GrowDiff,
            ),
            // `+` with its shift reported, as the keyboard enhancements do.
            (
                press(KeyCode::Char('+'), KeyModifiers::ALT | KeyModifiers::SHIFT),
                Action::GrowDiff,
            ),
            (
                press(KeyCode::Char('-'), KeyModifiers::ALT),
                Action::ShrinkDiff,
            ),
            (ch(' '), Action::ToggleReviewed),
            (press(KeyCode::Tab, KeyModifiers::NONE), Action::ToggleFocus),
        ];
        for (key, want) in cases {
            assert_eq!(act(&map, r, key), Lookup::Act(want), "{key:?}");
        }
        // A modifier nobody bound is not the bare key.
        assert_eq!(
            act(&map, r, press(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Lookup::Nothing
        );
        // One key, a different action per screen.
        assert_eq!(act(&map, r, ch('f')), Lookup::Act(Action::Files));
        assert_eq!(
            act(&map, Screen::FileList, ch('f')),
            Lookup::Act(Action::Files)
        );
        // `q` and `?` are no action's: the reviewer answers them itself.
        assert_eq!(act(&map, r, ch('q')), Lookup::Nothing);
        assert_eq!(act(&map, r, ch('?')), Lookup::Nothing);
    }

    #[test]
    fn a_sequence_waits_and_a_stray_press_is_read_on_its_own() {
        let map = Keymap::default();
        let r = Screen::Review;
        let Lookup::Pending(d) = act(&map, r, ch('d')) else {
            panic!("d starts dd");
        };
        assert_eq!(map.lookup(r, &d, ch('d')), Lookup::Act(Action::Delete));
        assert_eq!(map.lookup(r, &d, ch('j')), Lookup::Act(Action::Down));
        assert_eq!(map.lookup(r, &d, ch('!')), Lookup::Nothing);
    }

    #[test]
    fn a_bound_action_takes_exactly_the_keys_given() {
        let map = Keymap::new(&keys(&[(Action::NextGroup, &["ctrl-j"])])).unwrap();
        let r = Screen::Review;
        let ctrl_j = press(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert_eq!(act(&map, r, ctrl_j), Lookup::Act(Action::NextGroup));
        assert_eq!(
            act(&map, r, ch('J')),
            Lookup::Nothing,
            "the default is gone"
        );
        assert_eq!(
            act(&map, r, ch('}')),
            Lookup::Nothing,
            "every default is gone"
        );
        assert_eq!(
            map.first(r, Action::NextGroup).unwrap().to_string(),
            "ctrl-j"
        );
    }

    #[test]
    fn a_rebinding_holds_in_every_screen_the_action_works_in() {
        let map = Keymap::new(&keys(&[(Action::Down, &["e"])])).unwrap();
        for screen in Screen::ALL {
            assert_eq!(act(&map, screen, ch('e')), Lookup::Act(Action::Down));
            assert_eq!(act(&map, screen, ch('j')), Lookup::Nothing);
        }
    }

    #[test]
    fn an_empty_list_unbinds() {
        let map = Keymap::new(&keys(&[(Action::Publish, &[])])).unwrap();
        assert_eq!(act(&map, Screen::Review, ch('P')), Lookup::Nothing);
        assert!(map.first(Screen::Review, Action::Publish).is_none());
        assert_eq!(map.says(Screen::Review, Action::Publish, "publishes"), None);
    }

    #[test]
    fn a_clash_names_both_actions_and_the_screen() {
        let err = Keymap::new(&keys(&[(Action::Delete, &["j"])])).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("down") && text.contains("delete"), "{text}");
        assert!(text.contains("the review"), "{text}");
        // Delete works in the findings list too, and `j` moves there as well.
        assert!(text.contains("the findings list"), "{text}");
    }

    #[test]
    fn a_key_that_starts_another_is_a_clash() {
        let err = Keymap::new(&keys(&[(Action::Fold, &["d"])])).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("could never be pressed"), "{text}");
        assert!(text.contains("fold") && text.contains("delete"), "{text}");
    }

    #[test]
    fn screens_that_do_not_meet_do_not_clash() {
        // `D` clears the findings list's notes, and the review has no `D`.
        Keymap::new(&keys(&[(Action::Refetch, &["D"])])).unwrap();
    }

    #[test]
    fn the_reserved_keys_are_nobodys() {
        for (key, why) in RESERVED {
            let err = Keymap::new(&keys(&[(Action::Copy, &[key])])).unwrap_err();
            assert!(matches!(err.0[..], [KeyProblem::Reserved { .. }]), "{err}");
            assert!(err.to_string().contains(why), "{err}");
        }
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let err = Keymap::new(&keys(&[
            (Action::Copy, &["ctrl-c"]),
            (Action::Top, &["nope"]),
            (Action::Reply, &["j"]),
        ]))
        .unwrap_err();
        assert_eq!(err.0.len(), 3, "{err}");
        assert!(err.to_string().starts_with("[keys]: 3 problems"), "{err}");
    }

    #[test]
    fn keys_parse_as_a_reader_writes_them() {
        let text = |s: &str| Binding::parse(s).unwrap().to_string();
        assert_eq!(text("J"), "J");
        assert_eq!(text("shift-j"), "J");
        assert_eq!(text("ctrl-d"), "ctrl-d");
        assert_eq!(text("alt--"), "alt--");
        assert_eq!(text("alt-="), "alt-=");
        assert_eq!(text("Enter"), "enter");
        assert_eq!(text("ESC"), "esc");
        assert_eq!(text("space"), "space");
        assert_eq!(text("down"), "down");
        assert_eq!(text("f5"), "f5");
        assert_eq!(text("d d"), "dd");
        assert_eq!(text("g  g"), "gg");
        assert_eq!(text("ctrl-x k"), "ctrl-x k");
        assert!(Binding::parse("").is_err());
        assert!(Binding::parse("ctrl-a-b").is_err(), "two keys at once");
        let err = Binding::parse("dd").unwrap_err();
        assert!(err.contains("\"d d\""), "the error says how: {err}");
    }

    #[test]
    fn a_binding_presses_what_it_names() {
        let map = Keymap::default();
        for screen in Screen::ALL {
            for a in Action::ALL {
                for b in map.bindings(screen, a) {
                    let mut pending = Vec::new();
                    let mut last = Lookup::Nothing;
                    for key in b.presses() {
                        last = map.lookup(screen, &pending, key);
                        if let Lookup::Pending(p) = &last {
                            pending = p.clone();
                        }
                    }
                    assert_eq!(last, Lookup::Act(a), "{b} in {}", screen.title());
                }
            }
        }
    }
}
