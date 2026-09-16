//! `/` — a word, found anywhere in the changed files.
//!
//! The reviewer could move to a group, a file or a hunk, and not to a NAME.
//! A reader who remembers what a thing is called and not where it lives had
//! to leave the tool for `grep` and come back with a line number.
//!
//! **What is searched is the files as they are now, not the diff.** Every
//! changed file's head side, unchanged lines included, which is where most of
//! the names a reader is hunting for actually are. Two things follow from it
//! and are stated in `spec/tui.md` because a reader will meet them: a line the
//! change DELETED is not in the file any more and so is not found, and a
//! deleted file holds nothing at all.
//!
//! The corpus is [`RowFactory`](crate::rows::RowFactory)'s own blob cache,
//! filled once before the reviewer opens (`crate::review`). So a scan reads
//! memory and never a process, and `/` answers on the keystroke.
//!
//! Enumeration stays total (ADR 0005, 0012): no extension filter and no path
//! exclusion. A lockfile is searched like anything else and ranks last
//! because its group does, which is a view's ordering and not a filter.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use regex::Regex;

use crate::rows::{RowKind, SnippetLine};
use crate::vendor::text_utils::split_pairs_at_ranges;

use super::*;

/// Hits a scan collects before it stops counting.
///
/// A three-letter query over a big branch matches tens of thousands of lines,
/// and a list nobody can reach the end of costs the same to build as one they
/// can. The header says `2000+ found` so the number on screen is never a lie.
pub(super) const MOST_HITS: usize = 2000;

/// How far `enter` will pull a file open to reach a line outside every window.
///
/// Every revealed line becomes a row and goes through syntect, and a generated
/// file can put a match tens of thousands of lines from the nearest hunk.
/// Past this the jump lands on the boundary row instead and the footer says
/// how far the line still is.
pub(super) const MOST_REVEALED: u32 = 4000;

/// One line of one file that matches the query.
///
/// One per LINE, not per match: a line with four hits is one thing to go and
/// read. The preview marks every hit on the lines it shows.
#[derive(Clone)]
pub struct Occurrence {
    pub path: String,
    /// Head-side line number, counting from 1.
    pub line: u32,
    /// The first hit's byte range within the line as the pane draws it.
    pub start: usize,
    pub end: usize,
    /// The group that owns this line, as a position in the reading plan.
    ///
    /// The group of the hunk holding the line, or — for a line inside no hunk
    /// — the first group by plan order that owns any hunk in this file. `None`
    /// where no group touches the file at all, which is a file whose change
    /// carries no hunks: a mode change, a rename of untouched content.
    pub group: Option<usize>,
    /// `g2 focus C37` — the group, its tier, and the shape class of the hunk
    /// holding this line.
    ///
    /// The class is the answer to "have I read this already?", which is the
    /// question a reader looking at four hits in four files is really asking:
    /// four hits in one class are one shape, and the plan may well have
    /// deferred three of them for exactly that reason. A line inside NO hunk
    /// has no class, and the gap is the statement — the badge that names a
    /// class is the badge on a line the change wrote.
    ///
    /// Empty where no group owns the file at all.
    pub badge: String,
    /// Is the line inside a hunk this change wrote?
    pub in_hunk: bool,
}

/// Which way the query is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Every character stands for itself. `a.c` finds `a.c`, not `abc`.
    Literal,
    /// The query is a pattern. `ctrl-r` is what asks for this.
    Pattern,
}

impl Reading {
    fn flip(self) -> Self {
        match self {
            Reading::Literal => Reading::Pattern,
            Reading::Pattern => Reading::Literal,
        }
    }

    /// What the query row is led by, so the mode is on screen and not in the
    /// reader's memory.
    pub(super) fn sigil(self) -> &'static str {
        match self {
            Reading::Literal => " /",
            Reading::Pattern => " /~",
        }
    }
}

/// The query's matcher, with smart case.
///
/// A regex either way (design rule 5 — the boring, widely-used crate). Escaped
/// for a literal, and taken as written for a pattern. The crate compiles a
/// literal to a memchr search, and it reports byte ranges in the HAYSTACK — so
/// a fold that changes a string's length cannot put a mark in the wrong column.
///
/// **Smart case**: an all-lowercase query ignores case, and one uppercase
/// letter in it means the reader typed the case they meant. It holds for a
/// pattern too, where `\w` and `\W` are the reader's own business.
///
/// `None` means there is nothing to search for: an empty query, or — only ever
/// in [`Reading::Pattern`] — a pattern that does not compile. The box tells
/// those two apart so it can say `bad pattern` rather than `no occurrence`.
pub(super) fn matcher(query: &str, reading: Reading) -> Option<Regex> {
    if query.is_empty() {
        return None;
    }
    let folded = !query.chars().any(char::is_uppercase);
    let source = match reading {
        Reading::Literal => regex::escape(query),
        Reading::Pattern => query.to_string(),
    };
    regex::RegexBuilder::new(&source)
        .case_insensitive(folded)
        .build()
        .ok()
}

/// Ink for a hit: the palette's `highlight` accent as a FILL, with the ground
/// reversed out of it.
///
/// A fill rather than the underline the symbol float marks with, because the
/// two say different things. A symbol mark says "there is something here to
/// ask about" on a row the reader is already reading, and has to stay out of
/// the way; this says "this is the thing you went looking for", on a line the
/// reader has not read yet, and being got out of the way is the one thing it
/// must not be.
///
/// It can be a background here where `draw::mark_symbols` could not be one:
/// nothing in this box goes through `Theme::step_band`, which dispatches on
/// colour VALUES, so there is no tint for a new one to be confused with.
fn hit_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.highlight_fg)
        .bg(theme.highlight_bg)
        .add_modifier(Modifier::BOLD)
}

/// Mark every hit on one already-highlighted line.
fn mark_hits(pairs: &[(Style, String)], re: &Regex, theme: &Theme) -> Vec<(Style, String)> {
    let text: String = pairs.iter().map(|(_, t)| t.as_str()).collect();
    let at: Vec<(usize, usize)> = re.find_iter(&text).map(|m| (m.start(), m.end())).collect();
    if at.is_empty() {
        return pairs.to_vec();
    }
    split_pairs_at_ranges(pairs, at, hit_style(theme))
}

impl App {
    /// Open the search, on the query the reader last used.
    ///
    /// The query survives a close, and so does which hit was selected: a
    /// reader who has just jumped to one wants the next, and re-typing the
    /// word to get it is the tool asking them to repeat themselves.
    pub(super) fn open_search(&mut self) {
        self.visual = None;
        let query = self.last_query.clone();
        let reading = self.last_reading;
        let entries = self.search_scan(&query, reading);
        let selected = self.last_hit.min(entries.len().saturating_sub(1));
        let scroll = text::follow(selected, 0, self.search_list_rows());
        self.mode = Mode::Search {
            picked: !query.is_empty(),
            query,
            reading,
            entries,
            selected,
            scroll,
            preview: Vec::new(),
        };
        self.refresh_preview();
    }

    /// How the open box is reading its query. `Literal` when none is open,
    /// which is what it opens as.
    pub fn search_reading(&self) -> Reading {
        match &self.mode {
            Mode::Search { reading, .. } => *reading,
            _ => Reading::Literal,
        }
    }

    /// Whether the box is holding a pattern it could not compile. Only ever
    /// true in [`Reading::Pattern`]: an escaped literal always compiles.
    pub(super) fn search_pattern_is_bad(&self) -> bool {
        match &self.mode {
            Mode::Search { query, reading, .. } => {
                !query.is_empty() && matcher(query, *reading).is_none()
            }
            _ => false,
        }
    }

    /// The first occurrence the list is showing.
    pub(super) fn search_scroll(&self) -> usize {
        match &self.mode {
            Mode::Search { scroll, .. } => *scroll,
            _ => 0,
        }
    }

    /// Close it, keeping the query and the hit for the next `/`.
    pub(super) fn close_search(&mut self) {
        if let Mode::Search {
            query,
            reading,
            selected,
            ..
        } = &self.mode
        {
            self.last_query = query.clone();
            self.last_reading = *reading;
            self.last_hit = *selected;
        }
        self.mode = Mode::Normal;
    }

    /// Rows the occurrence list is drawn in — the number it scrolls against
    /// too, from one function, for the reason `text::findings_rows` gives.
    pub(super) fn search_list_rows(&self) -> usize {
        text::search_list_rows(self.viewport.body_rows)
    }

    /// Every line of every changed file that matches, best first.
    ///
    /// Read-only: the corpus is already in memory, so this runs on the
    /// keystroke that changed the query.
    fn search_scan(&self, query: &str, reading: Reading) -> Vec<Occurrence> {
        let Some(re) = matcher(query, reading) else {
            return Vec::new();
        };
        let plan = self.session.plan();
        let doc = self.session.doc();
        // Plan order is the position in `groups`, which the projection already
        // holds in reading order.
        let rank: HashMap<&str, usize> = plan
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.id.as_str(), i))
            .collect();
        // What the reader is looking at, which outranks everything else.
        let here_path = self.file_at_cursor().map(|i| self.files()[i].path.clone());
        let here_group = (self.view_mode == ViewMode::Groups).then_some(self.selected_group);

        let mut out: Vec<(SortKey, Occurrence)> = Vec::new();
        for f in &doc.files {
            // A blob of bytes is not text, and lossy-decoding one produces
            // matches nobody can go and read. This is classification, never
            // enumeration: the file is still in the document and still in
            // every count.
            if f.binary || f.submodule.is_some() {
                continue;
            }
            let Some(lines) = self.factory.cached_head_lines(&f.path) else {
                continue;
            };
            // The file's hunks once, with the group each one landed in.
            let hunks: Vec<(u32, u32, Option<usize>, String)> = plan
                .files
                .iter()
                .find(|v| v.path == f.path)
                .map(|v| {
                    v.hunks
                        .iter()
                        .map(|h| {
                            let e = &doc.hunks[h.index()];
                            let g = plan
                                .group_of_hunk(*h)
                                .and_then(|g| rank.get(g.id.as_str()).copied());
                            (e.new_start, e.new_start + e.new_count, g, e.class.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            // A line in no hunk belongs to the first group that reads this
            // file — "the first plan it is contained by".
            let file_group = hunks.iter().filter_map(|(_, _, g, _)| *g).min();

            for (i, text) in lines.iter().enumerate() {
                let Some(m) = re.find(text) else { continue };
                let line = i as u32 + 1;
                let owner = hunks
                    .iter()
                    .find(|(lo, hi, _, _)| *hi > *lo && line >= *lo && line < *hi);
                let group = owner.and_then(|(_, _, g, _)| *g).or(file_group);
                let badge = group
                    .and_then(|g| plan.groups.get(g))
                    .map(|g| match owner {
                        Some((_, _, _, class)) => {
                            format!("{} {} {class}", g.id, plan.tier_name(g))
                        }
                        None => format!("{} {}", g.id, plan.tier_name(g)),
                    })
                    .unwrap_or_default();
                let here = here_path.as_deref() == Some(f.path.as_str())
                    || (group.is_some() && group == here_group);
                out.push((
                    SortKey {
                        elsewhere: !here,
                        outside: owner.is_none(),
                        group: group.unwrap_or(usize::MAX),
                        path: f.path.clone(),
                        line,
                    },
                    Occurrence {
                        path: f.path.clone(),
                        line,
                        start: m.start(),
                        end: m.end(),
                        group,
                        badge,
                        in_hunk: owner.is_some(),
                    },
                ));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out.truncate(MOST_HITS);
        out.into_iter().map(|(_, o)| o).collect()
    }

    /// Rebuild the preview for whichever hit is selected.
    ///
    /// Reading a blob and running syntect both want `&mut self`, and `draw` is
    /// a pure function of the model — so the preview is model state, exactly
    /// as the symbol float's body is.
    fn refresh_preview(&mut self) {
        let Mode::Search {
            query,
            reading,
            entries,
            selected,
            ..
        } = &self.mode
        else {
            return;
        };
        let Some(re) = matcher(query, *reading) else {
            if let Mode::Search { preview, .. } = &mut self.mode {
                preview.clear();
            }
            return;
        };
        let Some(occ) = entries.get(*selected) else {
            if let Mode::Search { preview, .. } = &mut self.mode {
                preview.clear();
            }
            return;
        };
        let (path, line) = (occ.path.clone(), occ.line);
        let rows = text::search_preview_rows(self.viewport.body_rows);
        // The hit sits in the middle of what is shown, so the reader sees what
        // leads to it as well as what follows.
        let first = line.saturating_sub((rows as u32) / 2).max(1);
        let added = self.added_ranges(&path);
        let (body, _) = self.factory.declaration(
            &self.theme,
            &path,
            first,
            first + rows as u32 - 1,
            rows,
            &added,
        );
        let theme = self.theme.clone();
        let body: Vec<SnippetLine> = body
            .into_iter()
            .map(|l| SnippetLine {
                pairs: mark_hits(&l.pairs, &re, &theme),
                ..l
            })
            .collect();
        if let Mode::Search { preview, .. } = &mut self.mode {
            *preview = body;
        }
    }

    /// A key inside the box. Every printable one types, which is why the list
    /// moves on the arrows and `?` is not help here.
    pub(super) fn search_key(&mut self, key: KeyEvent) {
        let rows = self.search_list_rows();
        let Mode::Search {
            query,
            reading,
            entries,
            selected,
            scroll,
            picked,
            ..
        } = &mut self.mode
        else {
            return;
        };
        // A selected query behaves as one in any text field: the next thing
        // typed replaces it, and anything else drops the selection and leaves
        // the word alone.
        let was_picked = std::mem::take(picked);
        let mut edited = false;
        let mut moved = false;
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.close_search();
                return;
            }
            (KeyCode::Enter, _) => {
                let Some(occ) = entries.get(*selected) else {
                    self.status = "nothing to jump to".into();
                    return;
                };
                let occ = occ.clone();
                self.close_search();
                self.jump_to_occurrence(&occ);
                return;
            }
            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                text::step_list(selected, scroll, entries.len(), rows, true);
                moved = true;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                text::step_list(selected, scroll, entries.len(), rows, false);
                moved = true;
            }
            // The toggle, not a second box: the query a reader typed as a
            // literal is usually most of the pattern they now want.
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                *reading = reading.flip();
                edited = true;
            }
            (KeyCode::Backspace, _) => {
                if was_picked {
                    query.clear();
                } else {
                    query.pop();
                }
                edited = true;
            }
            // The two line-editing chords a one-line box owes its reader. A
            // caret and its movement keys would cost the arrows, and the
            // arrows are how the list moves.
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                query.clear();
                edited = true;
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                let kept = query.trim_end_matches(|c: char| !c.is_alphanumeric());
                let cut = kept.trim_end_matches(char::is_alphanumeric).len();
                query.truncate(cut);
                edited = true;
            }
            (KeyCode::Char(c), m) if m.is_empty() || m == KeyModifiers::SHIFT => {
                if was_picked {
                    query.clear();
                }
                query.push(c);
                edited = true;
            }
            _ => {}
        }
        if edited {
            self.rerun_search();
        }
        if edited || moved {
            self.refresh_preview();
        }
    }

    /// The query changed: scan again and start at the best hit.
    fn rerun_search(&mut self) {
        let Mode::Search { query, reading, .. } = &self.mode else {
            return;
        };
        let (query, reading) = (query.clone(), *reading);
        let found = self.search_scan(&query, reading);
        if let Mode::Search {
            entries,
            selected,
            scroll,
            ..
        } = &mut self.mode
        {
            *entries = found;
            *selected = 0;
            *scroll = 0;
        }
    }

    /// Text pasted into the query box.
    pub(super) fn search_paste(&mut self, text: &str) {
        let Mode::Search { query, picked, .. } = &mut self.mode else {
            return;
        };
        // A paste replaces a selected query, as typing does.
        if std::mem::take(picked) {
            query.clear();
        }
        // One line: the box is one row, and the rest of a multi-line paste
        // would be typed into a field nobody can see.
        query.push_str(text.lines().next().unwrap_or_default());
        self.rerun_search();
        self.refresh_preview();
    }

    /// Select the `hit`-th occurrence, or jump to it if it is already the one.
    pub(super) fn search_click(&mut self, hit: usize) {
        let rows = self.search_list_rows();
        let Mode::Search {
            entries,
            selected,
            scroll,
            ..
        } = &mut self.mode
        else {
            return;
        };
        if hit >= entries.len() {
            return;
        }
        if hit == *selected {
            let occ = entries[hit].clone();
            self.close_search();
            self.jump_to_occurrence(&occ);
            return;
        }
        *selected = hit;
        *scroll = text::follow(hit, *scroll, rows);
        self.refresh_preview();
    }

    /// The wheel over the list.
    pub(super) fn search_wheel(&mut self, down: bool) {
        let rows = self.search_list_rows();
        if let Mode::Search {
            entries,
            selected,
            scroll,
            ..
        } = &mut self.mode
        {
            text::step_list(selected, scroll, entries.len(), rows, down);
        }
        self.refresh_preview();
    }

    /// Put the cursor on the matched line, wherever in the review it lives.
    ///
    /// The same navigation `jump_to_finding` makes, and for the same reason: a
    /// row exists only in the view that is BUILT, so reaching one is a
    /// navigation and never a row index. Three things can be in the way and
    /// each is opened in turn — the wrong group selected, a fold over the
    /// hunk, and a window that does not reach the line.
    pub(super) fn jump_to_occurrence(&mut self, occ: &Occurrence) {
        let (path, line) = (occ.path.clone(), occ.line);
        match self.view_mode {
            ViewMode::Groups => match occ.group {
                Some(g) => self.select_entry(g),
                None => {
                    // No group owns the file, so the reading plan has no row
                    // for it at all. Only the file view can show this one.
                    self.status = format!("{path} carries no hunks · f opens the file view");
                    return;
                }
            },
            ViewMode::Files => match self.reveal_path(&path) {
                Some(row) => self.select_entry(row),
                None => {
                    self.status = format!("{path} is not in this review");
                    return;
                }
            },
        }
        // A folded skim remainder or a folded noise group hides the hunk
        // itself. Opening it is the reader overriding a default deliberately,
        // on a line they asked for by name (ADR 0006).
        if self.row_of_line(&path, line).is_none() {
            self.toggle_group_fold();
        }
        // Still nothing: the line is real but outside every window the pane
        // shows, which is most of a file.
        if self.row_of_line(&path, line).is_none() {
            self.reveal_line(&path, line);
        }
        match self.row_of_line(&path, line) {
            Some(row) => self.land_on(row),
            None => self.land_near(&path, line),
        }
    }

    /// The row showing head-side line `line` of `path`, if this view has one.
    ///
    /// A diff row does not name its file; the header above it does, which is
    /// why this walks rather than searching.
    fn row_of_line(&self, path: &str, line: u32) -> Option<usize> {
        let mut here = false;
        for (i, r) in self.rows.iter().enumerate() {
            if let RowKind::FileHeader(p) = &r.kind {
                here = p == path;
            }
            if here && r.line.as_ref().is_some_and(|l| l.holds("new", line)) {
                return Some(i);
            }
        }
        None
    }

    /// Pull a file open far enough to show `line`.
    ///
    /// Grows the gap of the nearest hunk TOWARDS the line, so nothing has to
    /// be crossed: no hunk lies inside the gap that is being opened, by the
    /// definition of nearest. Capped, because every revealed line is a row and
    /// goes through syntect.
    fn reveal_line(&mut self, path: &str, line: u32) {
        let mut best: Option<(usize, bool, u32)> = None;
        for (i, h) in self.session.doc().hunks.iter().enumerate() {
            if h.file != path {
                continue;
            }
            // A pure deletion has no new-side lines; it still sits AFTER the
            // new-side line its header names, which is all this needs.
            let top = h.new_start.max(1);
            let bot = top + h.new_count.max(1);
            let (down, away) = if bot <= line {
                (true, line + 1 - bot)
            } else if line < top {
                (false, top - line)
            } else {
                // Already inside a hunk: no gap reaches this line, and the
                // row's absence means the hunk is not in this view.
                return;
            };
            if best.is_none_or(|(_, _, d)| away < d) {
                best = Some((i, down, away));
            }
        }
        let Some((hunk, down, away)) = best else {
            return;
        };
        if away > MOST_REVEALED {
            self.status =
                format!("{path}:{line} is {away} lines from the nearest hunk · z opens more");
            return;
        }
        let e = self.expanded.entry(hunk).or_default();
        if down {
            e.down = e.down.max(away as usize);
        } else {
            e.up = e.up.max(away as usize);
        }
        self.rebuild_rows();
    }

    /// The line could not be reached: land on the file and say so, rather
    /// than leave the cursor where the reader was not looking.
    fn land_near(&mut self, path: &str, line: u32) {
        let row = self
            .rows
            .iter()
            .position(|r| matches!(&r.kind, RowKind::FileHeader(p) if p == path));
        match row {
            Some(row) => {
                self.land_on(row);
                if self.status.is_empty() {
                    self.status = format!("{path}:{line} is not shown here · z opens more");
                }
            }
            None => {
                if self.status.is_empty() {
                    self.status = format!("could not reach {path}:{line}");
                }
            }
        }
    }
}

/// What ranks one hit above another, in order of what it answers.
///
/// 1. **Where the reader already is.** The file under the diff cursor, or the
///    group they have open. A hit in front of them is the one they meant.
/// 2. **Inside a hunk.** The change is what this tool is for; a match in code
///    the branch did not touch is context, and context comes second.
/// 3. **Plan order.** The projection holds groups in reading order, so a
///    group's position IS its rank — and a rank fixes a tier, which is why
///    the tier is on every row and in none of this arithmetic.
/// 4. **Path and line**, so equal hits have one order and not an arbitrary one.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SortKey {
    elsewhere: bool,
    outside: bool,
    group: usize,
    path: String,
    line: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lowercase_query_ignores_case_and_an_uppercase_one_does_not() {
        let re = matcher("readingsplit", Reading::Literal).unwrap();
        assert!(re.is_match("ReadingSplit"), "all lowercase folds");
        let re = matcher("ReadingSplit", Reading::Literal).unwrap();
        assert!(re.is_match("ReadingSplit"));
        assert!(!re.is_match("readingsplit"), "an uppercase letter is meant");
        // And it holds for a pattern, where the reader owns the metacharacters
        // and not the case rule.
        let re = matcher("reading.plit", Reading::Pattern).unwrap();
        assert!(re.is_match("ReadingSplit"));
    }

    #[test]
    fn a_literal_query_is_never_a_pattern() {
        let re = matcher("a.c", Reading::Literal).unwrap();
        assert!(re.is_match("a.c"));
        assert!(!re.is_match("abc"), ". is a dot, not any character");
    }

    #[test]
    fn a_pattern_query_is_one() {
        let re = matcher("a.c", Reading::Pattern).unwrap();
        assert!(re.is_match("abc"), ". is any character now");
        assert!(re.is_match("a.c"));
        assert!(
            matcher("a(", Reading::Pattern).is_none(),
            "a pattern that does not compile finds nothing"
        );
        assert!(
            matcher("a(", Reading::Literal).is_some(),
            "and the same characters are a fine literal"
        );
    }

    #[test]
    fn an_empty_query_matches_nothing_at_all() {
        assert!(matcher("", Reading::Literal).is_none());
        assert!(matcher("", Reading::Pattern).is_none());
    }

    #[test]
    fn the_sort_puts_here_then_hunks_then_plan_order() {
        let key = |elsewhere, outside, group, line| SortKey {
            elsewhere,
            outside,
            group,
            path: "a.rs".into(),
            line,
        };
        let mut keys = [
            key(true, false, 0, 1),
            key(false, true, 9, 1),
            key(false, false, 3, 2),
            key(false, false, 3, 1),
        ];
        keys.sort();
        assert_eq!(
            keys.iter()
                .map(|k| (k.elsewhere, k.outside, k.group, k.line))
                .collect::<Vec<_>>(),
            vec![
                (false, false, 3, 1),
                (false, false, 3, 2),
                (false, true, 9, 1),
                (true, false, 0, 1),
            ]
        );
    }
}
