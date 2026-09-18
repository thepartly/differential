//! The reviewer's model, key handling and drawing. `handle_key` is a plain
//! method on the model returning effects — testable without a terminal.
//!
//! All review state (reviewed marks, findings, resume cursor) lives in the
//! engine's `ReviewSession`; this model holds presentation state only.
//!
//! This module holds the types and `App::new`. The four jobs `App` does are
//! one file each, because 3854 lines of them in one file meant a reader
//! looking for what a key does scrolled past everything that draws:
//!
//! - [`state`] — what the model recomputes when something changes: the tree,
//!   the rows, the cursor, the scroll, the layout toggles.
//! - [`keys`] — `handle_key` and `handle_paste`, the whole input surface.
//! - [`findings`] — the mutators that reach the session: reviewed marks,
//!   findings written, edited, deleted and jumped to.
//! - [`draw`] — every `draw_*`, and the row composition behind them.
//! - [`text`] — measuring and cutting text to a column budget. A leaf: it
//!   knows nothing about `App`, and both `keys` and `draw` read from it, which
//!   is what keeps a list's scroll height equal to its drawn height.
//! - [`help`] — one table of keys, read by the footer and by `?` (issue 30).
//! - [`forge`] — the forge's side: fetching review threads on a worker
//!   thread, resolving one, drafting a reply (ADR 0029).
//!
//! `App`'s inherent methods are split across those files, so a method that
//! was private to one file is `pub(super)` now. The scope is the same one it
//! always had — this module — and nothing new leaves it.

use std::collections::{HashMap, HashSet};

use differential_engine::FsReviewSession;
use differential_engine::config::ThemeName;
use differential_engine::plan::LineCounts;
use differential_engine::review_state::{FindingStatus, Lines};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};
use tui_textarea::TextArea;

use crate::rows::RowsContext;
use crate::rows::{
    DiffMode, GroupContext, RAIL, Row, RowContent, RowFactory, build_dir_rows, build_file_rows,
    build_group_rows, pill,
};
use crate::theme::Theme;
use crate::window::Expansion;

const SCROLL_MARGIN: usize = 3;

/// Presentation settings the application layer reads from config and hands to
/// the renderer. Not review state: nothing here is persisted in the sidecar.
#[derive(Debug, Clone)]
pub struct ReviewOptions {
    /// Context lines either side of a hunk before any expansion.
    pub context: usize,
    /// Lines one `z` on a context boundary row pulls in.
    pub context_step: usize,
    /// Which layout a review opens in when the reader has not chosen one.
    ///
    /// Resolved from config by the application layer, so the renderer takes a
    /// plain value and never learns the config's vocabulary.
    pub split_diff: bool,
    /// The range the reader typed, if they typed one — so the footer can name
    /// `dfr findings <range> --summary` when the clipboard is out of reach.
    ///
    /// Presentation, which is why it comes from the application layer rather
    /// than from the pipeline's result: the review's IDENTITY is a resolved
    /// sha plus a spec, and neither is what the reader would type back.
    pub range: Option<String>,
    /// Which palette to wear. A name, not a built [`Theme`]: building one
    /// parses the syntax set, and this struct is plain data the app layer
    /// fills in from config.
    pub theme: ThemeName,
}

impl Default for ReviewOptions {
    fn default() -> Self {
        ReviewOptions {
            context: 3,
            context_step: 10,
            // Matches `config::DiffLayout`'s default. The two are separate
            // because the renderer must not read config; a test asserts they
            // agree.
            split_diff: true,
            range: None,
            theme: ThemeName::default(),
        }
    }
}

/// Floor for the scroll arithmetic.
///
/// Not a guess about the terminal — geometry is measured — but a clamp so a
/// three-row window cannot produce nonsense.
const MIN_VIEWPORT: usize = 8;

/// The reviewer's panes: the left pane, the detail, a status row.
pub struct Panes {
    pub body: Rect,
    pub plan: Rect,
    pub detail: Rect,
    pub status: Rect,
}

/// The left pane's width before the reader moves the divider.
pub const DEFAULT_PLAN_COLS: u16 = 40;

/// The narrowest either pane may be squeezed to.
///
/// A pane spends two columns on its border, and the diff pane halves what is
/// left again in the split view, so a floor of a handful of columns would buy
/// a pane that draws a frame around nothing.
pub const MIN_PANE: u16 = 20;

/// A divider at `at`, held inside a `span` that keeps `floor` columns each
/// side of it.
///
/// Both of this reviewer's dividers are this rule — the one between the panes
/// and the split view's middle — with different spans and different floors. It
/// is one function because of the fallback: a span too narrow for two floors
/// **splits down the middle**, and `clamp` panics when its low bound passes its
/// high one, so a second copy is a second chance to leave that guard out.
///
/// Splitting rather than giving the floor to one side is the honest answer in
/// both places. At the panes it stops the left list starving the diff — the
/// pane the reader opened the tool for — and at the halves it is what the split
/// view did before either could be dragged.
pub fn split_point(at: usize, span: usize, floor: usize) -> usize {
    let max = span.saturating_sub(floor);
    if max < floor {
        return span / 2;
    }
    at.clamp(floor, max)
}

/// The divider between the panes, held inside the screen.
///
/// It lives here because `layout` is the one function `draw` and the hit test
/// both call: a clamp applied anywhere else could disagree with the frame on
/// screen. The model asks it too, on the way out of [`App::plan_cols`].
pub fn clamp_cols(cols: u16, width: u16) -> u16 {
    split_point(cols.into(), width.into(), MIN_PANE.into()) as u16
}

/// The one layout. `draw` places widgets with it and the event loop measures
/// with it, so the two can never disagree about how tall the detail pane is.
///
/// `plan_cols` is the reader's divider, and the ONLY thing they move. Focus
/// still does not enter into it: the overviews each focus brings up float over
/// a pane rather than splitting one, which is what lets the pane heights stay a
/// function of the terminal alone — and lets a key never change them. The width
/// arrives as an argument rather than being read from the model, so this stays
/// the single place the number turns into a rectangle.
pub fn layout(area: Rect, plan_cols: u16) -> Panes {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(clamp_cols(plan_cols, area.width)),
            Constraint::Min(0),
        ])
        .split(outer[0]);
    Panes {
        body: outer[0],
        plan: panes[0],
        detail: panes[1],
        status: outer[1],
    }
}

/// What a drag has hold of.
///
/// One field and not two flags: the pointer has hold of one thing at a time,
/// and two options would spell a state that cannot happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    /// The divider between the panes, carrying which of its two columns the
    /// press landed on as a distance from the left pane's width. A drag that
    /// assumed the first column would run one ahead of a pointer that grabbed
    /// the second, for the whole gesture.
    Panes(i16),
    /// The split view's middle. One column, so there is no offset to carry.
    Split,
}

/// The two columns the divider paints on: the left pane's right border and the
/// detail pane's left.
///
/// Shared with the hit test, so a drag grabs the columns the draw will paint
/// rather than a number that was true of them once.
pub fn divider(panes: &Panes) -> (u16, u16) {
    let right = panes.plan.x + panes.plan.width;
    (right.saturating_sub(1), right)
}

/// Measured terminal geometry, pushed into the model BEFORE any key is
/// handled — so scroll math is arithmetic over a known height rather than a
/// guess corrected one frame later.
///
/// Row BUILDING must still never depend on width. `RowContent::Split` defers
/// its columns to draw time precisely so a resize never rebuilds rows, and the
/// one width measured here does not change that: it is what a WRAPPED row is
/// composed at. Wrapping is a draw-time fact, and the scroll budget needs the
/// same fact, so both read one number measured in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub detail_rows: usize,
    /// The detail pane's CONTENT width — what a row wraps at, and therefore
    /// what its height is a function of.
    pub detail_cols: usize,
    pub plan_rows: usize,
    /// The body's FULL height, borders included — a modal floats over the
    /// body and draws its own box, so it needs the raw number the two panes
    /// have already subtracted their borders from.
    pub body_rows: usize,
    /// The whole screen the panes were laid out on. A mouse event names a
    /// cell, and which pane that cell is in is [`App::panes`] — the same
    /// `layout` call `draw` makes, so a hit test and a frame cannot disagree.
    pub area: Rect,
}

impl Viewport {
    pub fn measure(area: Rect, plan_cols: u16) -> Self {
        let panes = layout(area, plan_cols);
        Viewport {
            // Every pane is bordered.
            detail_rows: panes.detail.height.saturating_sub(2) as usize,
            detail_cols: panes.detail.width.saturating_sub(2) as usize,
            plan_rows: panes.plan.height.saturating_sub(2) as usize,
            body_rows: panes.body.height as usize,
            area,
        }
    }
}

impl Default for Viewport {
    /// Before the first measurement.
    fn default() -> Self {
        Viewport {
            detail_rows: 24,
            detail_cols: 78,
            plan_rows: 24,
            body_rows: 26,
            area: Rect::new(0, 0, 120, 27),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Groups,
    Detail,
}

pub enum Mode {
    Normal,
    /// Writing a finding.
    Editing {
        /// The canonical hunk it belongs to.
        hunk: usize,
        /// The lines the reviewer had picked when the box opened; `None`
        /// anchors the whole hunk.
        lines: Option<Lines>,
        /// The finding being rewritten. `None` files a new one.
        rewriting: Option<String>,
        /// The forge thread this answers. A reply is still a finding until a
        /// publish sends it (ADR 0029).
        reply_to: Option<String>,
        /// A comment of the reader's on the forge being rewritten. Saving
        /// sends the text there first.
        own: Option<differential_engine::forge::OwnComment>,
        editor: Box<TextArea<'static>>,
    },
    /// The keys of where the reader pressed `?`, carrying the mode they
    /// pressed it in: help opened over a modal has to give that modal back,
    /// not drop the reader into the review behind it.
    Help(Box<Mode>),
    /// Something the footer cannot hold: a forge's whole answer to a call
    /// that failed. Any key closes it.
    Notice {
        title: String,
        text: String,
    },
    /// File-list modal over the current rows: jump to a file header.
    FileList {
        entries: Vec<FileListEntry>,
        selected: usize,
        /// First visible entry. It had none, and clipped instead: a document
        /// with more files than the pane is tall simply never drew the rest,
        /// and the cursor walked off into rows that were not on screen.
        scroll: usize,
    },
    /// Every finding in the review, in one list.
    ///
    /// A note is written on a line and drawn under it, which is where it
    /// belongs while reading the code and no help at all in answering "what
    /// have I found". An ORPHANED note is worse off: it has no row anywhere,
    /// so this is the only place it can be read or deleted.
    Findings {
        entries: Vec<FindingEntry>,
        selected: usize,
        /// First visible entry. The file list has none and clips at the body
        /// height; a review has more notes than it has files.
        scroll: usize,
        /// `D` was pressed and the next key answers.
        confirming: bool,
    },
    /// `P` was pressed: what a publish would send and what it would leave,
    /// shown before anything leaves the machine. `y` sends (ADR 0029).
    Publish {
        plan: differential_engine::forge::PublishPlan,
    },
    /// `dd` on a comment of the reader's: the next key answers, and only
    /// `y` deletes it on the forge (ADR 0029).
    DeleteComment {
        own: differential_engine::forge::OwnComment,
    },
    /// `/` — a word, found anywhere in the changed files (see [`search`]).
    ///
    /// One payload, which that module owns: every printable key types into it,
    /// so the box carries a query, how it is read, what it found and where the
    /// reader is in that — seven fields, and naming them here would be seven
    /// places outside the module that know its shape.
    Search(search::Search),
}

pub struct FindingEntry {
    pub id: String,
    /// `src/app.rs:1307`, or `src/app.rs:1307-1312` for a range.
    pub at: String,
    /// The note's first line.
    pub body: String,
    pub orphaned: bool,
    pub moved: bool,
    /// A forge thread rather than a note: `id` is the thread's (ADR 0029).
    pub thread: bool,
    /// A note the request already has, whose twin was not fetched (yet).
    pub published: bool,
    /// A thread the forge marks resolved.
    pub resolved: bool,
}

impl FindingEntry {
    /// Which of the list's three sections this sits in, in list order:
    /// notes, threads, orphaned.
    pub fn section(&self) -> u8 {
        match (self.orphaned, self.thread) {
            (true, _) => 2,
            (false, true) => 1,
            (false, false) => 0,
        }
    }
}

/// Where the rules between the list's sections fall: the index of the first
/// entry of each section after the first non-empty one. Drawn, not stored, so
/// they cost a row on screen and nothing in the model.
pub fn section_rules(entries: &[FindingEntry]) -> Vec<usize> {
    entries
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0].section() != w[1].section())
        .map(|(i, _)| i + 1)
        .collect()
}

pub struct FileListEntry {
    pub path: String,
    /// Row index of the file's header in the current rows.
    pub row_idx: usize,
    pub adds: usize,
    pub dels: usize,
    pub reviewed: bool,
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Quit,
    CopySummary(String),
}

/// An open symbol float: the token it lights, and the declaration it shows.
///
/// Everything here is resolved when `z` is pressed, because reading a blob and
/// running syntect both need `&mut self` and drawing is a pure function of the
/// model. Holding the CONTENT rather than the coordinates to fetch it is what
/// keeps that true.
pub struct Peek {
    /// The diff row this belongs to. Moving the cursor drops the whole thing —
    /// a highlight pointing at a row the cursor has left is worse than none.
    pub row: usize,
    /// Which resolvable symbol on that row, counting from 0, left to right.
    pub nth: usize,
    /// `name · file:line · C7 · g3`.
    pub title: String,
    /// The declaration, one entry per line: its number, and its styled text.
    pub body: Vec<crate::rows::SnippetLine>,
    /// Lines the cap cut, for the `… N more lines` row.
    pub more: usize,
}

/// A plan row's relation to the selected group — what the gutter connector
/// draws. The plan is a graph (a group can follow several others), not a tree, and not
/// acyclic either.
///
/// One direction only: what the selected group *follows*. The reverse edge was
/// drawn too, in a second colour of the same glyph, which meant the gutter said
/// something different from the `after:` line directly beneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Selected,
    /// The selected group follows this one.
    Dependency,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Semantic groups — the reading plan.
    Groups,
    /// Flattened per-file list, every hunk in file order.
    Files,
}

/// One row of the group map — the document's file tree with everything the
/// selected group does not touch folded away.
///
/// A separate row type rather than a filtered `Vec<TreeEntry>`: the map folds
/// on a different question from the file view (does the GROUP touch this?
/// rather than did the reader press `z`?), and the two folds must not share
/// state — folding here would move the file view's cursor.
enum MapRow {
    /// A directory the group touches. Its children follow.
    Dir { depth: usize, name: String },
    /// A directory the group does not touch, and everything under it. A chain
    /// of single-child directories is joined, so one row says `a/b/c/`.
    Folded {
        depth: usize,
        name: String,
        files: usize,
    },
    /// A file the group touches, carrying the group's part of it — not the
    /// file's own totals. The map is drawn for ONE group, so a count on it
    /// that described the whole file described something the reader is not
    /// looking at.
    File {
        depth: usize,
        file_idx: usize,
        counts: LineCounts,
    },
    /// A run of files the group does not touch, inside one it does.
    More { depth: usize, files: usize },
}

impl MapRow {
    pub(super) fn depth(&self) -> usize {
        match self {
            MapRow::Dir { depth, .. }
            | MapRow::Folded { depth, .. }
            | MapRow::File { depth, .. }
            | MapRow::More { depth, .. } => *depth,
        }
    }
}

/// One visible row of the file tree: a directory, or a file (indexing into
/// `files`, which stays flat — it anchors reviewed state and the persisted
/// cursor).
pub struct TreeEntry {
    pub depth: usize,
    pub kind: TreeKind,
}

pub enum TreeKind {
    Dir { path: String },
    File { file_idx: usize },
}

/// What the reader has opened up for this sitting: a group's fold, a hunk's
/// context, a resolved thread. Reading aids, not findings, so nothing here
/// reaches the sidecar store. They are also the inputs every row builder
/// reads that are not the session's or the palette's, which is why they are
/// one struct with one method.
#[derive(Default)]
pub struct Opened {
    /// Group ids whose fold is open.
    pub folds: HashSet<String>,
    /// How far each hunk's context has been pulled open, by canonical index.
    pub context: HashMap<usize, Expansion>,
    /// Resolved threads the reader has opened with `z`. A resolved thread is
    /// collapsed to its header by default; the rest is settled reading, shown
    /// on demand.
    pub threads: HashSet<String>,
}

impl Opened {
    /// What every hunk-level row builder needs, borrowed field by field.
    ///
    /// A method on `App` would borrow the whole of it, and the builders need
    /// `&mut App::factory` alongside; the compiler accepts only field-level
    /// borrows there, which is why `rebuild_rows` used to spell this out as
    /// two twelve-field literals.
    fn rows_context<'a>(
        &'a self,
        theme: &'a Theme,
        opts: &ReviewOptions,
        session: &'a FsReviewSession,
        reviewed: &'a HashSet<usize>,
        mode: DiffMode,
        show_group_labels: bool,
    ) -> RowsContext<'a> {
        RowsContext {
            theme,
            doc: session.doc(),
            plan: session.plan(),
            findings: session.findings(),
            threads: session.threads(),
            reviewed,
            mode,
            show_group_labels,
            context: opts.context,
            context_step: opts.context_step,
            expansion: &self.context,
            expanded_threads: &self.threads,
        }
    }
}

/// The measured screen and where the reader has put the dividers. An input
/// to update, never a draw-time output; none of it reaches the sidecar —
/// where a divider sits is a reading position for this sitting, as the
/// sideways shift is.
pub struct Geometry {
    viewport: Viewport,
    /// The divider, with the reading plan in the left pane.
    ///
    /// Two numbers and not one because `f` swaps two different lists into that
    /// pane: a group block is a paragraph that wants room, and a tree row is a
    /// path that wants more of it. One width made `f` a choice between the two
    /// readings.
    plan_cols: u16,
    /// The divider, with the file tree in the left pane.
    tree_cols: u16,
    /// Where the split view's middle sits, as a signed distance from the
    /// centre of the diff pane. Zero opens every review, and a unified diff
    /// ignores it.
    split_offset: i16,
    /// What the pointer took hold of, while the button is still down.
    divider_grab: Option<Grab>,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            plan_cols: DEFAULT_PLAN_COLS,
            tree_cols: DEFAULT_PLAN_COLS,
            split_offset: 0,
            divider_grab: None,
        }
    }
}

/// How far each pane's content is shifted from where it starts. Reading
/// positions for this sitting, decided in update and never at draw time,
/// which is why `App` keeps the whole struct private.
#[derive(Default)]
pub struct Scroll {
    /// Rows the diff pane has scrolled.
    detail: usize,
    /// Columns the diff pane's CONTENT is shifted left. `s` and `w` are
    /// recorded against a review because they are layout choices; a column
    /// is not one.
    sideways: usize,
    /// Rows the left pane's list has scrolled.
    plan: usize,
}

/// What the document and the tree imply, computed when its inputs change
/// rather than on every frame. The overviews' three used to be derived inside
/// `draw`, which meant an O(hunks) scan with a string compare per hunk on
/// EVERY frame — enough to make a large review feel stuck on each keypress.
#[derive(Default)]
pub struct Derived {
    /// The file tree with nothing folded — what the group map folds on the
    /// group, and what the reader's `z` must never reach.
    ///
    /// A second copy rather than `App::tree`, because the two folds must not
    /// share state in EITHER direction (see `MapRow`). Reading `tree` meant a
    /// directory the reader had folded in the file view arrived at the map
    /// already folded, so the map drew `▸ src/ 4` where the group's own lit
    /// files belong. Built once: the document does not change while a
    /// session is open.
    map_tree: Vec<TreeEntry>,
    /// Which files each tree row covers, by row. Rebuilt with `App::tree`,
    /// because it is a pure function of it and the file list.
    tree_files: Vec<Vec<usize>>,
    /// Where each file sits in the document, by path. Built once.
    file_index: HashMap<String, usize>,
    /// Hunk indices marked reviewed, in THIS document. Refreshed by
    /// `rebuild_rows`, which every path that changes a mark ends with.
    reviewed: HashSet<usize>,
    /// The selected group's files, which the group map lights.
    map_files: HashSet<usize>,
    /// The group map's rows, derived from `map_tree` and `map_files`.
    map_rows: Vec<MapRow>,
    /// The files the file list shows, in order.
    listed_files: Vec<usize>,
}

pub struct App {
    pub session: FsReviewSession,
    factory: RowFactory,

    /// Visible rows of the file tree (rebuilt when a directory folds).
    pub tree: Vec<TreeEntry>,
    /// Directory paths currently collapsed.
    collapsed: HashSet<String>,
    derived: Derived,

    pub focus: Focus,
    pub mode: Mode,
    pub view_mode: ViewMode,
    pub selected_group: usize,
    pub selected_file: usize,
    pub rows: Vec<Row>,
    pub cursor: usize,
    /// The row a line selection is anchored at; the other end is the cursor.
    ///
    /// One field, not a mode: `j`/`k` go on moving the cursor and the
    /// selection is the span between the two ends, so `V` adds a state to
    /// the model without adding one to the key table.
    pub visual: Option<usize>,
    /// The open symbol float, and which symbol on its row it is showing.
    ///
    /// One field, not a mode — the same reason `visual` is one. `j`/`k` must go
    /// on moving the cursor, and a mode would add a state to the key table to
    /// say something the cursor already says.
    ///
    /// Transient, like `opened`: looking something up is a reading aid for
    /// this sitting, not a finding, so nothing here reaches the sidecar store.
    pub peek: Option<Peek>,
    scroll: Scroll,
    pub opened: Opened,
    opts: ReviewOptions,
    /// The palette, built once. Held rather than rebuilt per frame because
    /// building one parses the syntax set, and because rows bake their colours
    /// in at build time — `rebuild_rows` reads it as much as `draw` does.
    theme: Theme,
    pub status: String,
    geometry: Geometry,
    pending_d: bool,
    /// The forge this review is of, when it is of a request (ADR 0029).
    forge: Option<forge::ForgeLink>,
    /// The one forge call that may be out. See `app::forge`.
    inflight: Option<forge::Inflight>,
    last_search: search::Last,
}

impl App {
    /// Swap the palette. Rows bake their colours in at build time, so this
    /// rebuilds them rather than leaving the old ink on screen.
    ///
    /// Test-only: the float-ground assertion is the caller. A running reviewer
    /// picks its palette from config once, at startup, and never swaps it.
    #[doc(hidden)]
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.rebuild_rows();
    }

    pub fn new(
        session: FsReviewSession,
        factory: RowFactory,
        opts: ReviewOptions,
        theme: Theme,
    ) -> Self {
        // Resume position: the cursor id is a group id in the semantic view,
        // a file path in the file view (session.file_view() disambiguates).
        let view_mode = if session.file_view() {
            ViewMode::Files
        } else {
            ViewMode::Groups
        };
        let resume: Option<(String, usize)> = session.cursor().cloned();
        let selected_group = match (&resume, view_mode) {
            (Some((id, _)), ViewMode::Groups) => session.plan().group_position(id).unwrap_or(0),
            _ => 0,
        };

        let mut app = App {
            session,
            factory,
            theme,
            tree: Vec::new(),
            collapsed: HashSet::new(),
            derived: Derived::default(),
            focus: Focus::Groups,
            mode: Mode::Normal,
            view_mode,
            selected_group,
            selected_file: 0,
            rows: Vec::new(),
            cursor: 0,
            visual: None,
            peek: None,
            scroll: Scroll::default(),
            opened: Opened::default(),
            opts,
            status: String::new(),
            geometry: Geometry::default(),
            pending_d: false,
            forge: None,
            inflight: None,
            last_search: search::Last::default(),
        };
        // The document is fixed for the session's life, so this is built once
        // rather than found by scanning the file list per row.
        app.derived.file_index = app
            .files()
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();
        // Every changed file, read once, before the reviewer draws anything.
        //
        // It is the search corpus (see [`search`]): one `git` call here is
        // what lets `/` answer on the keystroke rather than stop to read a
        // few hundred blobs the first time it is pressed. The rows would have
        // read most of these anyway, one group at a time.
        let paths: Vec<String> = app.files().iter().map(|f| f.path.clone()).collect();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        app.factory.prefetch(&paths);
        app.build_map_tree();
        app.rebuild_tree();
        // The persisted cursor names a path; reveal it in the tree.
        if app.view_mode == ViewMode::Files
            && let Some((path, _)) = resume.as_ref()
            && let Some(row) = app.reveal_path(path)
        {
            app.selected_file = row;
        }
        app.rebuild_rows();
        if let Some((_, row)) = resume {
            app.cursor = row.min(app.rows.len().saturating_sub(1));
        }
        app
    }

    /// Lines syntect has parsed since the reviewer opened.
    ///
    /// Exposed so the windowed rebuild's whole point — cost proportional to
    /// what is drawn, not to the files touched — is a testable property rather
    /// than a claim in a comment.
    pub fn highlighted_lines(&self) -> usize {
        self.factory.highlighted_lines()
    }

    /// The document's groups, projected by the engine.
    pub fn groups(&self) -> &[differential_engine::plan::GroupView] {
        &self.session.plan().groups
    }

    /// Every file in the document, document order — including the zero-hunk
    /// binary/submodule changes the group view cannot surface.
    pub fn files(&self) -> &[differential_engine::plan::FileView] {
        &self.session.plan().files
    }
}

mod draw;
mod findings;
mod forge;
mod help;
mod keys;
mod peek;
mod search;
mod state;
mod text;

// The geometry a click is judged against, from the functions that draw it.
// Exposed so a test can aim a click at the box the draw will place, rather
// than at a number that was true of the box once.
pub use draw::{
    FRAME_ROWS, MIN_HALF, centered_x, composer_area, composer_footer, delete_comment_area,
    delete_comment_footer, file_list_modal_area, findings_modal_area, findings_question,
    footer_row, half_centre, half_widths, pane_inner, publish_area, publish_footer,
    search_modal_area,
};
pub use help::{Act, Area, HelpSection};
pub use search::{Occurrence, Reading, Search};
pub use text::{Hint, Ink, hints_width};

pub use forge::ForgeLink;

/// What the footer says on a key aimed at someone else's comment.
pub(super) const NOT_YOURS: &str = "not your comment · r replies · x resolves";

/// The `s` a count takes, or not.
pub(super) fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
