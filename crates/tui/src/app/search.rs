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

use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use std::ops::Range;

use regex::Regex;
use tui_input::backend::crossterm::to_input_request;
use tui_input::{Input, InputRequest};

use crate::rows::{RowKind, SnippetLine};
use crate::text_utils::split_pairs_at_ranges;

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
    /// The FIRST hit's byte range within the line as the pane draws it. One
    /// range rather than two numbers, because nothing wants one without the
    /// other: it is what the preview shifts sideways to keep on screen.
    pub hit: Range<usize>,
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
    /// Is this where the reader already is — the file under the diff cursor,
    /// or the group they have open? Ranking only; the row does not say it,
    /// because the row is at the top and that IS saying it.
    here: bool,
    /// Is the line inside a hunk this change wrote? Ranking only, for the same
    /// reason: the badge names a shape class exactly when this is true.
    in_hunk: bool,
}

/// What ranks one hit above another, in the order it answers.
///
/// 1. **Where the reader already is.** The file under the diff cursor, or the
///    group they have open. A hit in front of them is the one they meant.
/// 2. **Inside a hunk.** The change is what this tool is for; a match in code
///    the branch did not touch is context, and context comes second.
/// 3. **Plan order.** The projection holds groups in reading order, so a
///    group's position IS its rank — and a rank fixes a tier, which is why
///    the tier is on every row and in none of this arithmetic.
/// 4. **Path and line**, so equal hits have one order and not an arbitrary one.
///
/// A key by reference rather than a struct of owned copies: every field here
/// is already on the occurrence, and the struct that held them was a second
/// place for the same facts to be got wrong.
/// One hunk of the file being scanned, as the scan needs it: the head-side
/// lines it covers, the group it landed in, and its shape class. A tuple of
/// four said none of those, and every use of it had to count commas.
struct Hunk {
    top: u32,
    bot: u32,
    group: Option<usize>,
    class: String,
}

impl Hunk {
    /// Does this hunk cover head-side line `line`? A hunk with no new-side
    /// lines — a pure deletion — covers none.
    fn holds(&self, line: u32) -> bool {
        self.bot > self.top && line >= self.top && line < self.bot
    }
}

fn rank(o: &Occurrence) -> (bool, bool, usize, &str, u32) {
    (
        !o.here,
        !o.in_hunk,
        o.group.unwrap_or(usize::MAX),
        &o.path,
        o.line,
    )
}

/// Which way the query is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Every character stands for itself. `a.c` finds `a.c`, not `abc`.
    Literal,
    /// The query is a regular expression. `ctrl-r` is what asks for this.
    Regexp,
}

impl Reading {
    fn flip(self) -> Self {
        match self {
            Reading::Literal => Reading::Regexp,
            Reading::Regexp => Reading::Literal,
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
/// regexp too, where `\w` and `\W` are the reader's own business.
///
/// `None` means there is nothing to search for: an empty query, or — only ever
/// in [`Reading::Regexp`] — an expression that does not compile. The box tells
/// those two apart so it can say `bad regexp` rather than `no occurrence`.
pub(super) fn matcher(query: &str, reading: Reading) -> Option<Regex> {
    if query.is_empty() {
        return None;
    }
    let folded = !query.chars().any(char::is_uppercase);
    let source = match reading {
        Reading::Literal => regex::escape(query),
        Reading::Regexp => query.to_string(),
    };
    regex::RegexBuilder::new(&source)
        .case_insensitive(folded)
        .build()
        .ok()
}

/// The box's own state.
///
/// One type rather than seven fields on the mode variant: nine functions each
/// opened by naming a different subset of them, the draw took eight arguments
/// to be handed the same thing, and three one-line accessors existed only to
/// peek at one field of it.
///
/// **The boundary.** This owns what the box IS — the query, how it is read,
/// what it found, where the cursor is in that. It owns none of what the box
/// reads or does: scanning wants the document and the blob cache, and jumping
/// wants the rows, so both live on [`App`] and take this as an argument. That
/// is why nothing here needs a repository and every method on it is pure.
pub struct Search {
    /// The query and its caret.
    ///
    /// `tui_input`, not a `String` and an index of our own: a one-line field
    /// owes its reader a caret, `←`/`→`, `home`/`end`, word delete and a row
    /// that scrolls to follow the caret — and every one of those is a place to
    /// get a character boundary wrong (design rule 5). It is STATE only, which
    /// is why it fits: the query row is composed of spans around it.
    pub input: Input,
    pub reading: Reading,
    /// The query came back from the last `/` and is SELECTED: the next
    /// character typed replaces the whole of it, as it would in any text
    /// field. Reopening on the old word is what a reader wants when they are
    /// walking its hits; it is in the way when they are not, and this is the
    /// one state that serves both without a second key.
    ///
    /// Ours, not the input's: `tui_input` has no selection, and this is the
    /// only one the box needs.
    pub picked: bool,
    /// The compiled query, rebuilt by [`Search::retype`] and nowhere else.
    ///
    /// Held rather than compiled where it is wanted: the scan, the preview
    /// and the header each want it, and each compiling its own was three
    /// compiles per keystroke and three places for the case rule to drift.
    re: Option<Regex>,
    pub entries: Vec<Occurrence>,
    pub selected: usize,
    /// First visible occurrence. The list is longer than any box.
    pub scroll: usize,
    pub preview: Vec<SnippetLine>,
}

impl Search {
    fn new(query: String, reading: Reading) -> Self {
        let mut s = Search {
            picked: !query.is_empty(),
            input: Input::new(query),
            reading,
            re: None,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            preview: Vec::new(),
        };
        s.retype();
        s
    }

    pub fn query(&self) -> &str {
        self.input.value()
    }

    /// The query or its reading changed: compile it again.
    ///
    /// Refilling `entries` is the caller's, because only [`App`] can see the
    /// corpus. The two always happen together, and `App::rescan` is the one
    /// place they do.
    fn retype(&mut self) {
        self.re = matcher(self.query(), self.reading);
    }

    /// Whether the box is holding an expression it could not compile. Only
    /// ever true in [`Reading::Regexp`]: an escaped literal always compiles.
    pub fn bad_regexp(&self) -> bool {
        !self.query().is_empty() && self.re.is_none()
    }

    /// The occurrence the reader is standing on.
    pub fn hit(&self) -> Option<&Occurrence> {
        self.entries.get(self.selected)
    }

    /// Move the selection one step, and keep it in a window `rows` tall.
    fn step(&mut self, rows: usize, down: bool) {
        text::step_list(
            &mut self.selected,
            &mut self.scroll,
            self.entries.len(),
            rows,
            down,
        );
    }

    /// Put the selection on `hit`, and bring it into the same window.
    fn select(&mut self, hit: usize, rows: usize) {
        self.selected = hit;
        self.scroll = text::follow(hit, self.scroll, rows);
    }

    /// Hand a key to the query, and say whether the text changed.
    ///
    /// A key that WRITES replaces a selected query; a key that only moves the
    /// caret leaves it standing. Which of the two a key is, [`writes`] says.
    fn edit(&mut self, key: KeyEvent) -> bool {
        let Some(req) = to_input_request(&CrosstermEvent::Key(key)) else {
            return false;
        };
        // Taken only by a key that WRITES. A caret move leaves the selection
        // standing, which is the whole point of it: a reader who came back to
        // a word and looked along it has not said they are done with it.
        if writes(req) && std::mem::take(&mut self.picked) {
            self.input = Input::default();
        }
        let before = self.input.value().to_string();
        self.input.handle(req);
        self.input.value() != before
    }
}

/// Does this request change the text, rather than only move the caret?
///
/// A selected query is replaced by the first key that writes into it, and left
/// alone by one that does not — which is what a selection means in any text
/// field, and is why `←` and `→` keep the word the reader came back to.
fn writes(req: InputRequest) -> bool {
    !matches!(
        req,
        InputRequest::SetCursor(_)
            | InputRequest::GoToPrevChar
            | InputRequest::GoToNextChar
            | InputRequest::GoToPrevWord
            | InputRequest::GoToNextWord
            | InputRequest::GoToStart
            | InputRequest::GoToEnd
    )
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
    /// The open box, if one is open. The one place anything reaches into the
    /// mode for it.
    pub fn search(&self) -> Option<&Search> {
        match &self.mode {
            Mode::Search(s) => Some(s),
            _ => None,
        }
    }

    fn search_mut(&mut self) -> Option<&mut Search> {
        match &mut self.mode {
            Mode::Search(s) => Some(s),
            _ => None,
        }
    }

    /// How the open box is reading its query. `Literal` when none is open,
    /// which is what it opens as.
    pub fn search_reading(&self) -> Reading {
        self.search().map_or(Reading::Literal, |s| s.reading)
    }

    /// Rows the occurrence list is drawn in — the number it scrolls against
    /// too, from one function, for the reason `text::findings_rows` gives.
    pub(super) fn search_list_rows(&self) -> usize {
        text::search_list_rows(self.viewport.body_rows)
    }

    /// Open the search, on the query the reader last used.
    ///
    /// The query survives a close, and so does which hit was selected: a
    /// reader who has just jumped to one wants the next, and re-typing the
    /// word to get it is the tool asking them to repeat themselves.
    pub(super) fn open_search(&mut self) {
        self.visual = None;
        self.mode = Mode::Search(Search::new(self.last_query.clone(), self.last_reading));
        self.rescan();
        // The hit comes back too, which `rescan` has just reset to the first.
        let (hit, rows) = (self.last_hit, self.search_list_rows());
        if let Some(s) = self.search_mut() {
            s.select(hit.min(s.entries.len().saturating_sub(1)), rows);
        }
        self.refresh_preview();
    }

    /// Close it, keeping the query, the reading and the hit for the next `/`.
    pub(super) fn close_search(&mut self) {
        if let Some((query, reading, hit)) = self
            .search()
            .map(|s| (s.query().to_string(), s.reading, s.selected))
        {
            self.last_query = query;
            self.last_reading = reading;
            self.last_hit = hit;
        }
        self.mode = Mode::Normal;
    }

    /// The query or its reading changed: compile it, scan again, and start at
    /// the best hit. The one place those three happen, so they cannot drift.
    fn rescan(&mut self) {
        let Some(s) = self.search_mut() else { return };
        s.retype();
        // Cloned out because the scan reads the whole model and this borrows
        // one field of it. A compiled regex is an `Arc` inside, so it is a
        // pointer bump rather than a second compile.
        let re = s.re.clone();
        let found = match re {
            Some(re) => self.search_scan(&re),
            None => Vec::new(),
        };
        if let Some(s) = self.search_mut() {
            s.entries = found;
            s.selected = 0;
            s.scroll = 0;
        }
    }

    /// Rebuild the preview for whichever hit is selected.
    ///
    /// Reading a blob and running syntect both want `&mut self`, and `draw` is
    /// a pure function of the model — so the preview is model state, exactly
    /// as the symbol float's body is.
    fn refresh_preview(&mut self) {
        let body = self.build_preview();
        if let Some(s) = self.search_mut() {
            s.preview = body;
        }
    }

    /// The selected hit's line, with context either side and every hit on
    /// those lines marked. Empty when there is nothing to preview.
    fn build_preview(&mut self) -> Vec<SnippetLine> {
        let Mode::Search(s) = &self.mode else {
            return Vec::new();
        };
        let (Some(re), Some(occ)) = (s.re.clone(), s.hit()) else {
            return Vec::new();
        };
        let (path, line) = (occ.path.clone(), occ.line);
        let rows = text::search_preview_rows(self.viewport.body_rows);
        // The hit sits in the middle of what is shown, so the reader sees what
        // leads to it as well as what follows.
        let first = line.saturating_sub(rows as u32 / 2).max(1);
        let added = self.added_ranges(&path);
        let theme = self.theme.clone();
        let (body, _) =
            self.factory
                .declaration(&theme, &path, first, first + rows as u32 - 1, rows, &added);
        body.into_iter()
            .map(|l| SnippetLine {
                pairs: mark_hits(&l.pairs, &re, &theme),
                ..l
            })
            .collect()
    }

    /// A key inside the box.
    ///
    /// Five keys are the box's; everything else is the query's. That is why
    /// the list moves on the arrows that are NOT the query's, and why `?` is a
    /// character here rather than help.
    pub(super) fn search_key(&mut self, key: KeyEvent) {
        let rows = self.search_list_rows();
        let Some(s) = self.search_mut() else { return };
        let mut edited = false;
        let mut moved = false;
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.close_search();
                return;
            }
            (KeyCode::Enter, _) => {
                let Some(occ) = s.hit().cloned() else {
                    self.status = "nothing to jump to".into();
                    return;
                };
                self.close_search();
                self.jump_to_occurrence(&occ);
                return;
            }
            // Up and down are the LIST's, not the query's — a one-line field
            // has nothing for them to do, and this is the only box in the
            // reviewer where that is worth saying out loud.
            (KeyCode::Down, _) | (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                s.step(rows, true);
                moved = true;
            }
            (KeyCode::Up, _) | (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                s.step(rows, false);
                moved = true;
            }
            // The toggle, not a second box: the query a reader typed as a
            // literal is usually most of the expression they now want.
            (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                s.reading = s.reading.flip();
                edited = true;
            }
            _ => edited = s.edit(key),
        }
        if edited {
            self.rescan();
        }
        if edited || moved {
            self.refresh_preview();
        }
    }

    /// Text pasted into the query box.
    pub(super) fn search_paste(&mut self, text: &str) {
        let Some(s) = self.search_mut() else { return };
        // A paste replaces a selected query, as typing does.
        if std::mem::take(&mut s.picked) {
            s.input = Input::default();
        }
        // One line: the box is one row, and the rest of a multi-line paste
        // would be typed into a field nobody can see. Fed a character at a
        // time so it lands at the caret and the caret follows it.
        for c in text.lines().next().unwrap_or_default().chars() {
            s.input.handle(InputRequest::InsertChar(c));
        }
        self.rescan();
        self.refresh_preview();
    }

    /// A click on content line `line` of the box: select that occurrence, or
    /// jump to it if it is already the one.
    ///
    /// The line is the box's, not the list's — which row of the box is a row
    /// of the list is decided here, so the hit test has one rule to obey and
    /// not three.
    pub(super) fn search_click(&mut self, line: usize) {
        let rows = self.search_list_rows();
        let Some(s) = self.search_mut() else { return };
        // Row zero is the query; the rule and the preview are past the list.
        let Some(line) = line.checked_sub(1).filter(|l| *l < rows) else {
            return;
        };
        let hit = s.scroll + line;
        if hit >= s.entries.len() {
            return;
        }
        if hit == s.selected {
            let occ = s.entries[hit].clone();
            self.close_search();
            self.jump_to_occurrence(&occ);
            return;
        }
        s.select(hit, rows);
        self.refresh_preview();
    }

    /// The wheel over the list.
    pub(super) fn search_wheel(&mut self, down: bool) {
        let rows = self.search_list_rows();
        if let Some(s) = self.search_mut() {
            s.step(rows, down);
        }
        self.refresh_preview();
    }

    /// Every line of every changed file that matches, best first.
    ///
    /// Read-only: the corpus is already in memory, so this runs on the
    /// keystroke that changed the query.
    fn search_scan(&self, re: &Regex) -> Vec<Occurrence> {
        let plan = self.session.plan();
        let doc = self.session.doc();
        // Plan order is the position in `groups`, which the projection already
        // holds in reading order.
        let rank_of: HashMap<&str, usize> = plan
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| (g.id.as_str(), i))
            .collect();
        // What the reader is looking at, which outranks everything else.
        let here_path = self.file_at_cursor().map(|i| self.files()[i].path.clone());
        let here_group = (self.view_mode == ViewMode::Groups).then_some(self.selected_group);

        let mut out: Vec<Occurrence> = Vec::new();
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
            //
            // Through `file_index`, which the model already holds: finding the
            // projection's entry by comparing paths was a scan per file, and
            // a scan per file over the file list is the whole corpus squared.
            let hunks: Vec<Hunk> = self
                .file_index
                .get(f.path.as_str())
                .and_then(|i| plan.files.get(*i))
                .map(|v| {
                    v.hunks
                        .iter()
                        .map(|h| {
                            let e = &doc.hunks[h.index()];
                            Hunk {
                                top: e.new_start,
                                bot: e.new_start + e.new_count,
                                group: plan
                                    .group_of_hunk(*h)
                                    .and_then(|g| rank_of.get(g.id.as_str()).copied()),
                                class: e.class.clone(),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            // A line in no hunk belongs to the first group that reads this
            // file — "the first plan it is contained by".
            let file_group = hunks.iter().filter_map(|h| h.group).min();

            for (i, text) in lines.iter().enumerate() {
                let Some(m) = re.find(text) else { continue };
                let line = i as u32 + 1;
                let owner = hunks.iter().find(|h| h.holds(line));
                let group = owner.and_then(|h| h.group).or(file_group);
                let badge = group
                    .and_then(|g| plan.groups.get(g))
                    .map(|g| {
                        let (id, tier) = (&g.id, plan.tier_name(g));
                        match owner {
                            Some(h) => format!("{id} {tier} {}", h.class),
                            None => format!("{id} {tier}"),
                        }
                    })
                    .unwrap_or_default();
                out.push(Occurrence {
                    here: here_path.as_deref() == Some(f.path.as_str())
                        || (group.is_some() && group == here_group),
                    in_hunk: owner.is_some(),
                    path: f.path.clone(),
                    line,
                    hit: m.start()..m.end(),
                    group,
                    badge,
                });
            }
        }
        out.sort_by(|a, b| rank(a).cmp(&rank(b)));
        out.truncate(MOST_HITS);
        out
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
        let re = matcher("reading.plit", Reading::Regexp).unwrap();
        assert!(re.is_match("ReadingSplit"));
    }

    #[test]
    fn a_literal_query_is_never_a_pattern() {
        let re = matcher("a.c", Reading::Literal).unwrap();
        assert!(re.is_match("a.c"));
        assert!(!re.is_match("abc"), ". is a dot, not any character");
    }

    #[test]
    fn a_regexp_query_is_one() {
        let re = matcher("a.c", Reading::Regexp).unwrap();
        assert!(re.is_match("abc"), ". is any character now");
        assert!(re.is_match("a.c"));
        assert!(
            matcher("a(", Reading::Regexp).is_none(),
            "an expression that does not compile finds nothing"
        );
        assert!(
            matcher("a(", Reading::Literal).is_some(),
            "and the same characters are a fine literal"
        );
    }

    #[test]
    fn an_empty_query_matches_nothing_at_all() {
        assert!(matcher("", Reading::Literal).is_none());
        assert!(matcher("", Reading::Regexp).is_none());
    }

    /// An occurrence with nothing in it but what the ranking reads.
    fn ranked(here: bool, in_hunk: bool, group: usize, line: u32) -> Occurrence {
        Occurrence {
            path: "a.rs".into(),
            line,
            hit: 0..1,
            group: Some(group),
            badge: String::new(),
            here,
            in_hunk,
        }
    }

    #[test]
    fn the_rank_puts_here_then_hunks_then_plan_order() {
        let mut hits = [
            ranked(false, true, 0, 1),
            ranked(true, false, 9, 1),
            ranked(true, true, 3, 2),
            ranked(true, true, 3, 1),
        ];
        hits.sort_by(|a, b| rank(a).cmp(&rank(b)));
        assert_eq!(
            hits.iter()
                .map(|o| (o.here, o.in_hunk, o.group.unwrap(), o.line))
                .collect::<Vec<_>>(),
            vec![
                (true, true, 3, 1),
                (true, true, 3, 2),
                (true, false, 9, 1),
                (false, true, 0, 1),
            ]
        );
    }
}
