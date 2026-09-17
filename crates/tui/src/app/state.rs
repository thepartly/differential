//! What the model recomputes when something changes.
//!
//! The tree, the rows, the cursor and the scroll. Every entry point here ends
//! with the model consistent: `rebuild_rows` is what the rest of the app calls
//! after any change that could move a row.

use differential_engine::plan::{self, Fold};
use ratatui::text::Line;

use crate::rows::{Row, RowKind, RowsContext};
use crate::window::Side;

use super::draw::{Paint, compose_row_lines, half_centre, half_widths, overflow, pane_inner};
use super::*;

/// The tree rows for `files`, with every directory named in `folded` folded
/// away: the folded directory keeps its own row — there would be nothing left
/// to unfold otherwise — and everything beneath it, subdirectories included,
/// loses one.
///
/// Free rather than a method because it is called with TWO fold sets: the
/// reader's, for the file view's pane, and the empty one the group map reads.
fn build_tree(files: &[plan::FileView], folded: &HashSet<String>) -> Vec<TreeEntry> {
    let mut paths: Vec<(usize, Vec<String>)> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (i, f.path.split('/').map(str::to_string).collect()))
        .collect();
    paths.sort_by(|a, b| a.1.cmp(&b.1));

    let mut tree = Vec::new();
    let mut open: Vec<String> = Vec::new(); // directory components in scope
    // …and whether each of them is folded, carried alongside so that a
    // directory the NEXT file does not re-enter still hides what is under it.
    // Asking only the directories this file opens is what let a folded
    // directory's second subdirectory draw a row while its first one, and
    // every file, correctly disappeared.
    let mut open_folded: Vec<bool> = Vec::new();
    for (file_idx, parts) in paths {
        let dirs = &parts[..parts.len() - 1];
        // Close directories we have left.
        while open.len() > dirs.len() || (!open.is_empty() && open[..] != dirs[..open.len()]) {
            open.pop();
            open_folded.pop();
        }
        // Open the ones we entered. One question answers for both row kinds:
        // is anything ABOVE me folded?
        for (d, name) in dirs.iter().enumerate() {
            if d < open.len() {
                continue;
            }
            open.push(name.clone());
            let path = open.join("/");
            if !open_folded.iter().any(|&f| f) {
                tree.push(TreeEntry {
                    depth: d,
                    kind: TreeKind::Dir { path: path.clone() },
                });
            }
            open_folded.push(folded.contains(&path));
        }
        if !open_folded.iter().any(|&f| f) {
            tree.push(TreeEntry {
                depth: dirs.len(),
                kind: TreeKind::File { file_idx },
            });
        }
    }
    tree
}

impl App {
    /// Rebuild the visible tree rows from the flat file list, honouring
    /// collapsed directories. Directory rows appear once, in path order.
    pub fn rebuild_tree(&mut self) {
        self.tree = build_tree(self.files(), &self.collapsed);
        // Answered once per rebuild, not once per row per frame. Drawing the
        // file pane asks this for EVERY visible row, and the directory arm
        // allocated a prefix, scanned every file and sorted the result — so a
        // repaint cost O(rows x files log files) to redraw a tree that had
        // not changed.
        self.tree_files = (0..self.tree.len()).map(|r| self.files_under(r)).collect();
    }

    /// The group map's copy of the tree, with nothing folded.
    ///
    /// Built once, from `new`: it is a pure function of the file list, and the
    /// document does not change while a session is open.
    pub(super) fn build_map_tree(&mut self) {
        self.map_tree = build_tree(self.files(), &HashSet::new());
    }

    /// File indices covered by a tree row: one file, or every file under a
    /// directory (including collapsed ones).
    ///
    /// Reads the answer `rebuild_tree` computed. Panicking on an unknown row
    /// is not possible: the vector is rebuilt with the tree, in the same call.
    pub(super) fn files_of_tree_row(&self, row: usize) -> Vec<usize> {
        self.tree_files.get(row).cloned().unwrap_or_default()
    }

    /// The computation behind `tree_files`. Called only from `rebuild_tree`.
    fn files_under(&self, row: usize) -> Vec<usize> {
        match self.tree.get(row).map(|e| &e.kind) {
            Some(TreeKind::File { file_idx }) => vec![*file_idx],
            Some(TreeKind::Dir { path }) => {
                let prefix = format!("{path}/");
                let mut under: Vec<usize> = self
                    .files()
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.path.starts_with(&prefix))
                    .map(|(i, _)| i)
                    .collect();
                // Path order, so the diff pane presents files in the order the
                // tree lists them rather than in document order.
                under.sort_by(|a, b| self.files()[*a].path.cmp(&self.files()[*b].path));
                under
            }
            None => Vec::new(),
        }
    }

    /// Path of the selected file-tree row (a file path, or a directory).
    ///
    /// Test-only: the resume test reads it to prove the cursor came back to the
    /// same row. The app persists through `tree_row_path` directly.
    #[doc(hidden)]
    pub fn selected_path(&self) -> Option<String> {
        self.tree_row_path(self.selected_file)
    }

    /// The path a tree row stands for — what the resume cursor persists.
    pub(super) fn tree_row_path(&self, row: usize) -> Option<String> {
        match self.tree.get(row).map(|e| &e.kind) {
            Some(TreeKind::File { file_idx }) => Some(self.files()[*file_idx].path.clone()),
            Some(TreeKind::Dir { path }) => Some(path.clone()),
            None => None,
        }
    }

    /// Locate the tree row showing `path`, expanding collapsed ancestors.
    pub(super) fn reveal_path(&mut self, path: &str) -> Option<usize> {
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        for n in 1..=parts.len() {
            self.collapsed.remove(&parts[..n].join("/"));
        }
        self.rebuild_tree();
        self.tree.iter().position(|e| match &e.kind {
            TreeKind::File { file_idx } => self.files()[*file_idx].path == path,
            TreeKind::Dir { path: p } => p == path,
        })
    }

    /// Fold or unfold the selected directory.
    pub(super) fn toggle_dir(&mut self) -> bool {
        let Some(TreeKind::Dir { path }) = self.tree.get(self.selected_file).map(|e| &e.kind)
        else {
            return false;
        };
        let path = path.clone();
        if !self.collapsed.insert(path.clone()) {
            self.collapsed.remove(&path);
        }
        self.rebuild_tree();
        self.selected_file = self
            .tree
            .iter()
            .position(|e| matches!(&e.kind, TreeKind::Dir { path: p } if *p == path))
            .unwrap_or(0);
        self.follow_plan_scroll();
        self.rebuild_rows();
        true
    }

    /// The row a document with no groups shows instead of nothing.
    fn empty_rows() -> Vec<Row> {
        vec![Row::full(
            RowKind::Blank,
            Line::from("nothing to review — empty diff"),
        )]
    }

    pub fn rebuild_rows(&mut self) {
        // One read of the marks, kept for the frame to use too. Drawing asked
        // for its own copy three more times, and each one walked every hunk
        // digest in the document to build a set it then threw away.
        self.reviewed = self.session.reviewed_hunks();
        // The three degenerate cases `break` rather than `return`, so the
        // overviews at the tail are rebuilt from whatever rows this call
        // produced — including none. They used to return, which left three
        // cached walks describing rows that no longer existed. No document
        // reaches those branches with a non-empty overview today, so nothing
        // was wrong; the invariant holding it up was just nowhere written.
        'rows: {
            match self.view_mode {
                ViewMode::Groups => {
                    let Some(groups) = self.session.doc().groups.as_ref() else {
                        self.rows = Vec::new();
                        break 'rows;
                    };
                    if groups.is_empty() {
                        self.rows = Self::empty_rows();
                        break 'rows;
                    }
                    // The document's ids were validated once, when the session
                    // opened and projected it. This used to rebuild a `PlanIndex`
                    // here — revalidating every id in the document on every
                    // keypress — because that was the only way to reach a class's
                    // exemplar and members. The projection carries both now.
                    let view =
                        &self.session.plan().groups[self.selected_group.min(groups.len() - 1)];
                    // Spelled out, not built by a method, and it has to be: a
                    // method borrows the whole of `self`, and the row builders
                    // below need `&mut self.factory` while this holds the rest.
                    // Only a literal gives the compiler the field-level borrows.
                    let ctx = GroupContext {
                        core: RowsContext {
                            theme: &self.theme,
                            doc: self.session.doc(),
                            plan: self.session.plan(),
                            findings: self.session.findings(),
                            threads: self.session.threads(),
                            reviewed: &self.reviewed,
                            mode: self.diff_mode(),
                            show_group_labels: false,
                            context: self.opts.context,
                            context_step: self.opts.context_step,
                            expansion: &self.expanded,
                            expanded_threads: &self.expanded_threads,
                        },
                        view,
                        fold: if self.folds_open.contains(&view.id) {
                            Fold::Unfolded
                        } else {
                            Fold::Folded
                        },
                    };
                    self.rows = build_group_rows(&mut self.factory, &ctx);
                }
                ViewMode::Files => {
                    if self.tree.is_empty() {
                        self.rows = Self::empty_rows();
                        break 'rows;
                    }
                    let row = self.selected_file.min(self.tree.len() - 1);
                    let targets = self.files_of_tree_row(row);
                    // A literal for the same reason as the group arm above.
                    let ctx = RowsContext {
                        theme: &self.theme,
                        doc: self.session.doc(),
                        plan: self.session.plan(),
                        findings: self.session.findings(),
                        threads: self.session.threads(),
                        reviewed: &self.reviewed,
                        mode: self.diff_mode(),
                        show_group_labels: true,
                        context: self.opts.context,
                        context_step: self.opts.context_step,
                        expansion: &self.expanded,
                        expanded_threads: &self.expanded_threads,
                    };
                    self.rows = match targets.as_slice() {
                        // A single file keeps its dedicated builder (it renders a
                        // placeholder for zero-hunk binary/submodule changes).
                        [only] => {
                            let f = &self.session.plan().files[*only];
                            let (path, hunks) =
                                (f.path.clone(), f.hunks.iter().map(|h| h.index()).collect());
                            build_file_rows(&mut self.factory, &ctx, &path, hunks)
                        }
                        // A directory: every hunk beneath it, file headers and all.
                        many => {
                            let hunks: Vec<usize> = many
                                .iter()
                                .flat_map(|i| self.files()[*i].hunks.iter().map(|h| h.index()))
                                .collect();
                            build_dir_rows(&mut self.factory, &ctx, hunks)
                        }
                    };
                }
            }
        }
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        if !self
            .rows
            .get(self.cursor)
            .is_some_and(|r| r.kind.selectable())
        {
            self.cursor = self.next_selectable(0, 1).unwrap_or(0);
        }
        self.clamp_hscroll();
        self.rebuild_overviews();
    }

    pub(super) fn next_selectable(&self, from: usize, dir: isize) -> Option<usize> {
        let mut i = from as isize;
        loop {
            if i < 0 || i as usize >= self.rows.len() {
                return None;
            }
            if self.rows[i as usize].kind.selectable() {
                return Some(i as usize);
            }
            i += dir;
        }
    }

    pub(super) fn move_cursor(&mut self, dir: isize) {
        let start = self.cursor as isize + dir;
        if let Some(next) = self.next_selectable(start.max(0) as usize, dir) {
            self.cursor = next;
        }
        // The symbol float is not closed here. It belongs to the row it was
        // opened on, and every mover has to honour that — so the rule lives
        // where it cannot be forgotten, in `settle_peek`, once per event.
        self.follow_cursor();
    }

    /// Fold measured geometry into the model.
    ///
    /// A resize is an event like any other: both scroll offsets are re-clamped
    /// here, in update, rather than discovered while rendering.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        self.viewport = viewport;
        self.follow_cursor();
        self.follow_plan_scroll();
        self.clamp_hscroll();
    }

    /// Measure `area` against the divider in force and fold the answer in.
    ///
    /// The one way geometry enters the model. Moving the divider goes through
    /// it as a terminal resize does, because the two change the same numbers:
    /// a narrower diff pane wraps its rows at a narrower width, and the scroll
    /// budget that counts screen lines has to be re-clamped either way.
    pub fn set_area(&mut self, area: Rect) {
        self.set_viewport(Viewport::measure(area, self.plan_cols()));
    }

    /// Re-measure on the screen already recorded. What moving the divider and
    /// switching the left pane's list both end with.
    pub(super) fn remeasure(&mut self) {
        self.set_area(self.viewport.area);
    }

    /// The divider in force: the left pane's width, for the list it is
    /// showing, as the screen can actually show it.
    ///
    /// Clamped on the way OUT, so the model and the frame can never disagree
    /// about where the divider is. Without that, a width the reader chose on a
    /// wide terminal survived a shrink as a number nothing on screen matched,
    /// and a resize key read the stored number, moved it, wrote back the same
    /// clamped width and reported a move that never happened. The stored
    /// number is left alone: a terminal that widens again puts the divider
    /// back where they left it.
    pub fn plan_cols(&self) -> u16 {
        let cols = match self.view_mode {
            ViewMode::Groups => self.plan_cols,
            ViewMode::Files => self.tree_cols,
        };
        clamp_cols(cols, self.viewport.area.width)
    }

    /// The same number, to write. `f` swaps which one the reader is moving.
    fn plan_cols_mut(&mut self) -> &mut u16 {
        match self.view_mode {
            ViewMode::Groups => &mut self.plan_cols,
            ViewMode::Files => &mut self.tree_cols,
        }
    }

    /// The panes as they are on screen now.
    ///
    /// `layout` against the measured screen and the divider in force — what
    /// the hit test asks, so a click lands in the pane the reader can see.
    pub fn panes(&self) -> Panes {
        layout(self.viewport.area, self.plan_cols())
    }

    /// Where the split view's middle sits, as a distance from the centre.
    pub fn split_offset(&self) -> i16 {
        self.split_offset
    }

    /// The screen column the split view's middle is painted on, or `None` when
    /// there is no middle to grab: a unified diff has one column.
    ///
    /// Asks `half_widths`, which is what draws the middle, so the line under
    /// the pointer is the line on the screen.
    pub fn split_column(&self) -> Option<u16> {
        if !self.split_diff() {
            return None;
        }
        let inner = pane_inner(self.panes().detail);
        let (lw, _) = half_widths(inner.width as usize, self.split_offset);
        Some(inner.x + lw as u16)
    }

    /// Put the pane divider under screen column `x`.
    ///
    /// `grab` is which of its two columns the press landed on, as a distance
    /// from the left pane's width, so the column the reader took hold of is the
    /// one that stays under the pointer. Read from the pointer each time rather
    /// than accumulated, so a drag that runs into the clamp and comes back does
    /// not drift.
    pub(super) fn drag_panes_to(&mut self, x: u16, grab: i16) {
        let cols = i32::from(x) - i32::from(self.panes().body.x) - i32::from(grab);
        self.set_plan_cols(cols.clamp(0, u16::MAX.into()) as u16);
    }

    /// Put the split view's middle under screen column `x`.
    ///
    /// Stored as a distance from the centre, so the two halves keep their skew
    /// when the pane divider or the terminal moves. `half_widths` clamps, so a
    /// pointer dragged past either half's floor stops there.
    pub(super) fn drag_split_to(&mut self, x: u16) {
        let inner = pane_inner(self.panes().detail);
        // Read from the same centre `half_widths` spends the offset from, or a
        // grab and the draw would disagree about where nought is.
        let centre = half_centre(inner.width.into());
        let offset = i32::from(x) - i32::from(inner.x) - centre as i32;
        self.split_offset = offset.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
        // The middle changes how wide each half draws at, so it changes what
        // hangs off their right edges and how tall a wrapped row is. Both are
        // what `remeasure` re-derives.
        self.remeasure();
    }

    /// Put the divider at `cols`, and say so when it will not go.
    ///
    /// Returns whether it moved. A press that changes nothing reads as a key
    /// that does not work, which is why the callers that are keys speak up.
    pub(super) fn set_plan_cols(&mut self, cols: u16) -> bool {
        let want = clamp_cols(cols, self.viewport.area.width);
        if want == self.plan_cols() {
            return false;
        }
        *self.plan_cols_mut() = want;
        self.remeasure();
        true
    }

    /// `alt-=` and `alt--`: widen or narrow the DIFF pane by `by` columns.
    ///
    /// The diff pane, whichever pane has focus. Every other key acts on the
    /// pane you are in; this one and `/` do not, because the diff is the pane
    /// the reader asked to make room for.
    pub(super) fn resize_diff(&mut self, by: i16) {
        let cols = self.plan_cols().saturating_add_signed(-by);
        if self.set_plan_cols(cols) {
            return;
        }
        // A press that changes nothing reads as a key that does not work, so
        // the footer says which wall it is against and names the way back —
        // except on a screen too narrow to move the divider at all, where
        // naming the other key would be a lie.
        let width = self.viewport.area.width;
        self.status = if width < 2 * MIN_PANE {
            format!(
                "the terminal is too narrow to move the divider — it needs {} columns",
                2 * MIN_PANE
            )
        } else if by > 0 {
            format!("the diff pane is as wide as it goes · the left pane keeps {MIN_PANE} columns")
        } else {
            "the diff pane is as narrow as it goes · alt-= widens it".to_string()
        };
    }

    /// Diff-pane scroll offset. Decided in update, never at draw time — which
    /// is why the field itself is private.
    /// The pane heights currently in force.
    ///
    /// Exposed so a test can assert the guarantee the geometry rework rests on
    /// — that they are re-derived when focus changes, not left stale.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Rows one left-pane entry occupies.
    ///
    /// Arithmetic, not rendering, so the scroll math does not need a `Frame` —
    /// which is what lets it move out of `draw_groups`. A `debug_assert` there
    /// checks the two still agree.
    pub(super) fn plan_block_height(&self, idx: usize) -> usize {
        match self.view_mode {
            // group_lines: a title row, a counts row, and an `after:` row only
            // when the group has edges.
            ViewMode::Groups => self
                .groups()
                .get(idx)
                .map_or(0, |g| 2 + usize::from(!g.depends_on.is_empty())),
            ViewMode::Files => usize::from(idx < self.tree.len()),
        }
    }

    /// Keep the whole selected plan block in view. Lifted out of `draw_groups`.
    pub(super) fn follow_plan_scroll(&mut self) {
        let h = self.viewport.plan_rows.max(MIN_VIEWPORT);
        let selected = self.selected_entry();
        let start_row: usize = (0..selected).map(|i| self.plan_block_height(i)).sum();
        let end_row = start_row + self.plan_block_height(selected);
        if start_row < self.group_scroll {
            self.group_scroll = start_row;
        } else if end_row > self.group_scroll + h {
            self.group_scroll = end_row.saturating_sub(h);
        }
    }

    pub(super) fn follow_cursor(&mut self) {
        if self.cursor < self.scroll + SCROLL_MARGIN {
            self.scroll = self.cursor.saturating_sub(SCROLL_MARGIN);
        } else {
            self.scroll = self.scroll.max(self.highest_scroll());
        }
        // The rows above the first selectable one are the group header —
        // label, description, dependencies — and the cursor can never enter
        // them. Without this the scroll margin would pin the view one row
        // below the top and that header could never be read.
        if self
            .next_selectable(0, 1)
            .is_none_or(|first| self.cursor <= first)
        {
            self.scroll = 0;
        }
    }

    /// The furthest the view can be scrolled DOWN and still hold the cursor
    /// and its margin.
    ///
    /// The bottom edge is a budget of screen LINES, not a count of rows: a
    /// wrapped row is one row and several lines. Walking back from the last
    /// row that must stay visible, the first row the budget cannot afford is
    /// where the view has to start.
    pub(super) fn highest_scroll(&self) -> usize {
        let h = self.viewport.detail_rows.max(MIN_VIEWPORT);
        let last = (self.cursor + SCROLL_MARGIN).min(self.rows.len().saturating_sub(1));
        let mut used = 0;
        let mut top = last;
        for i in (0..=last).rev() {
            used += self.row_height(i);
            if used > h {
                break;
            }
            top = i;
        }
        // Never past the cursor: a row taller than the whole pane pins to the
        // top of the view and is cut off at the bottom, rather than scrolling
        // the row the reader is on out of sight.
        top.min(self.cursor)
    }

    /// The row drawn on screen line `line` of the detail pane, counted from
    /// the pane's first content line — `highest_scroll` run the other way: a
    /// click names a line, and a wrapped row is several of them.
    pub(super) fn row_at_line(&self, line: usize) -> Option<usize> {
        let mut used = 0;
        for i in self.scroll..self.rows.len() {
            used += self.row_height(i);
            if line < used {
                return Some(i);
            }
        }
        None
    }

    /// The plan entry whose block holds line `line` of the whole list — the
    /// caller adds `group_scroll` to a screen line, so this is
    /// `follow_plan_scroll`'s arithmetic run the other way.
    pub(super) fn plan_entry_at_line(&self, line: usize) -> Option<usize> {
        let n = match self.view_mode {
            ViewMode::Groups => self.groups().len(),
            ViewMode::Files => self.tree.len(),
        };
        let mut used = 0;
        for i in 0..n {
            used += self.plan_block_height(i);
            if line < used {
                return Some(i);
            }
        }
        None
    }

    /// The row half a pane away from `from`, walking `dir`.
    ///
    /// Half a pane of screen LINES. Counting rows would jump a screenful of
    /// wrapped prose in one press.
    pub(super) fn half_page(&self, from: usize, dir: isize) -> usize {
        let budget = self.viewport.detail_rows.max(MIN_VIEWPORT) / 2;
        let mut used = 0;
        let mut i = from;
        while used < budget {
            let next = i as isize + dir;
            if next < 0 || next as usize >= self.rows.len() {
                break;
            }
            i = next as usize;
            used += self.row_height(i);
        }
        i
    }

    /// Select the idx-th entry of the left pane (group or file).
    pub(super) fn select_entry(&mut self, idx: usize) {
        match self.view_mode {
            ViewMode::Groups => {
                if self.groups().is_empty() {
                    return;
                }
                self.selected_group = idx.min(self.groups().len() - 1);
            }
            ViewMode::Files => {
                if self.tree.is_empty() {
                    return;
                }
                self.selected_file = idx.min(self.tree.len() - 1);
            }
        }
        self.cursor = 0;
        self.scroll = 0;
        self.follow_plan_scroll();
        self.rebuild_rows();
    }

    pub(super) fn selected_entry(&self) -> usize {
        match self.view_mode {
            ViewMode::Groups => self.selected_group,
            ViewMode::Files => self.selected_file,
        }
    }

    /// Toggle semantic groups <-> flat file list; the choice persists.
    pub(super) fn toggle_file_view(&mut self) {
        let on = self.view_mode == ViewMode::Groups;
        if let Err(e) = self.session.set_file_view(on) {
            self.status = format!("save failed: {e:#}");
            return;
        }
        self.view_mode = if on {
            ViewMode::Files
        } else {
            ViewMode::Groups
        };
        self.cursor = 0;
        self.scroll = 0;
        // The divider is per list, so switching lists moves it — and the diff
        // pane's width with it. Re-measure before the rows are built, or they
        // wrap at the width the pane had a moment ago.
        self.remeasure();
        self.follow_plan_scroll();
        self.rebuild_rows();
        self.status = if on { "file view" } else { "reading plan view" }.into();
    }

    /// Open the file-list modal over the current rows (reflects folds and the
    /// active view). Enter jumps to the chosen file's header.
    pub(super) fn open_file_list(&mut self) {
        let reviewed = self.session.reviewed_hunks();
        let entries: Vec<FileListEntry> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match &r.kind {
                RowKind::FileHeader(path) => Some((i, path.clone())),
                _ => None,
            })
            .map(|(row_idx, path)| {
                // What this row says — both the counts and the ✓ — is about
                // what the reader is being SHOWN of the file, not about the
                // file. Reading a group that owns two of a file's ten hunks,
                // the file's own totals describe the eight that are not here.
                let view = self.session.plan();
                let hunks = match (self.view_mode, self.file_index.get(path.as_str())) {
                    (_, None) => Vec::new(),
                    (ViewMode::Groups, Some(&i)) => view.hunks_in(self.selected_group, i),
                    // The file view shows the file whole, so the whole file is
                    // the honest answer there.
                    (ViewMode::Files, Some(&i)) => view.files[i].hunks.clone(),
                };
                let counts = view.counts(&hunks);
                FileListEntry {
                    path,
                    row_idx,
                    adds: counts.adds,
                    dels: counts.dels,
                    reviewed: plan::all_reviewed(&hunks, &reviewed),
                }
            })
            .collect();
        if entries.is_empty() {
            self.status = "no files listed here (unfold with z?)".into();
            return;
        }
        self.mode = Mode::FileList {
            entries,
            selected: 0,
            scroll: 0,
        };
    }

    /// Every finding in the review, open ones first and orphans after.
    ///
    /// Store order within each group, which is the order they were written.
    /// The rule between the two groups is drawn rather than stored, so
    /// `selected` indexes findings and nothing has to skip a row it cannot
    /// land on.
    pub(super) fn open_findings(&mut self) {
        // No row index is kept. A note's row exists only in the view that is
        // built, so one captured here would be stale the moment the reader
        // navigates — `jump_to_finding` re-finds it by id after selecting the
        // group or file that owns it.
        // Notes first, then the forge's threads, then orphans. A published
        // note whose twin is fetched IS a thread now and is listed once, as
        // the thread (ADR 0029).
        let mut entries: Vec<FindingEntry> = self
            .session
            .findings()
            .iter()
            .filter(|f| !self.session.is_twinned(f))
            .map(|f| FindingEntry {
                at: f.anchor.at(),
                body: f.body.lines().next().unwrap_or("").to_string(),
                orphaned: f.status == FindingStatus::Orphaned,
                moved: f.moved,
                id: f.id.clone(),
                thread: false,
                published: f.upstream.is_some(),
                resolved: false,
            })
            .collect();
        entries.extend(self.session.threads().iter().map(|t| {
            let at = match &t.anchor {
                Some(a) => a.at(),
                None => match t.line {
                    Some(l) => format!("{}:{l}", t.path),
                    None => t.path.clone(),
                },
            };
            let root = t.root();
            FindingEntry {
                at,
                body: format!(
                    "{}: {}",
                    root.map(|c| c.author.as_str()).unwrap_or("?"),
                    root.and_then(|c| c.body.lines().next()).unwrap_or("")
                ),
                // A thread nothing in this plan holds is listed like an
                // orphan: it can be read here and reached nowhere.
                orphaned: t.anchor.is_none(),
                moved: false,
                id: t.id.clone(),
                thread: true,
                published: false,
                resolved: t.resolved,
            }
        }));
        if entries.is_empty() {
            self.status = "no findings yet — c writes one".into();
            return;
        }
        entries.sort_by_key(FindingEntry::section);
        self.mode = Mode::Findings {
            entries,
            selected: 0,
            scroll: 0,
            confirming: false,
        };
    }

    /// Jump to the next/previous hunk header, so a reviewer can move by
    /// change instead of by line.
    pub(super) fn jump_hunk(&mut self, dir: isize) {
        let mut i = self.cursor as isize + dir;
        while i >= 0 && (i as usize) < self.rows.len() {
            // A foreign hunk is context the reviewer asked for, not an entry
            // on this group's reading list, so hunk-to-hunk navigation passes
            // over it.
            if matches!(
                self.rows[i as usize].kind,
                RowKind::HunkHeader { foreign: false, .. }
            ) {
                self.cursor = i as usize;
                self.focus = Focus::Detail;
                self.follow_cursor();
                return;
            }
            i += dir;
        }
        self.status = if dir > 0 {
            "last hunk in this view".into()
        } else {
            "first hunk in this view".into()
        };
    }

    /// Pull more of the file in at the boundary row under the cursor.
    ///
    /// Growing upward inserts rows ABOVE the cursor, so the index has to be
    /// re-found rather than kept: the boundary row moves, and when the window
    /// reaches the start of the file (or merges into its neighbour) it stops
    /// existing at all.
    pub(super) fn expand_at_cursor(&mut self) {
        let Some(RowKind::ContextEdge {
            hunk,
            side,
            crossing,
        }) = self.rows.get(self.cursor).map(|r| r.kind.clone())
        else {
            return;
        };
        let step = self.opts.context_step;
        let e = self.expanded.entry(hunk).or_default();
        match (side, crossing) {
            (Side::Up, false) => e.up += step,
            (Side::Down, false) => e.down += step,
            // Crossing resets that side's context counter: the gap it measured
            // is not the outermost one any more.
            (Side::Up, true) => {
                e.crossed_up += 1;
                e.up = 0;
            }
            (Side::Down, true) => {
                e.crossed_down += 1;
                e.down = 0;
            }
        }
        self.rebuild_rows();

        // Match on the edge, not on what it offers: crossing turns a "next:"
        // boundary back into a context one, and the cursor should follow it.
        match self.rows.iter().position(
            |r| matches!(r.kind, RowKind::ContextEdge { hunk: h, side: sd, .. } if h == hunk && sd == side),
        ) {
            Some(pos) => self.cursor = pos,
            None => {
                // Nothing left to unfold in that direction: land on the hunk
                // itself and say why the boundary vanished.
                if let Some(pos) = self
                    .rows
                    .iter()
                    .position(|r| matches!(r.kind, RowKind::HunkHeader { hunk: h, .. } if h == hunk))
                {
                    self.cursor = pos;
                }
                self.status = match side {
                    Side::Up => "top of what precedes this hunk".into(),
                    Side::Down => "end of what follows this hunk".into(),
                };
            }
        }
        self.follow_cursor();
    }

    /// The files the flat list under the plan would show: every file the
    /// current rows touch, in the order they appear.
    ///
    /// Its LENGTH decides how tall that pane is, so this is called from layout
    /// as well as from drawing — one answer, so the two cannot disagree about
    /// how much room the list needs.
    pub(super) fn file_list(&self) -> Vec<usize> {
        // A set beside the vector, because the vector's ORDER is the answer —
        // the order the rows present the files — and `contains` on it made
        // this O(rows x files) twice over.
        let mut seen = HashSet::new();
        let mut out: Vec<usize> = Vec::new();
        for row in &self.rows {
            if let RowKind::FileHeader(path) = &row.kind
                && let Some(&i) = self.file_index.get(path.as_str())
                && seen.insert(i)
            {
                out.push(i);
            }
        }
        out
    }

    /// Recompute what the two overviews draw. Called with the rows, because
    /// that is what they describe — and never from `draw`, which runs on every
    /// keypress.
    pub(super) fn rebuild_overviews(&mut self) {
        self.listed_files = self.file_list();
        self.map_files = self.files_of_selected_group();
        // Third of the three, and the one that was left in `draw`. It reads
        // the two above and the tree, so this is the moment it can change.
        self.map_rows = self.compute_map_rows();
    }

    /// The row index of the file header the cursor is under.
    ///
    /// Walked backwards from the cursor, because a diff row does not name its
    /// file — the header above it does. Both the flat list's marker and the
    /// sticky header need this, so it is one function.
    pub(super) fn file_header_above(&self, from: usize) -> Option<usize> {
        // `..=0` on an empty slice panics, and rows ARE empty for a document
        // with no groups. Drawing was harmlessly a no-op there before this
        // helper existed; it stays one.
        let last = self.rows.len().checked_sub(1)?;
        self.rows[..=from.min(last)]
            .iter()
            .rposition(|r| matches!(r.kind, RowKind::FileHeader(_)))
    }

    /// The file the cursor is in, as an index into `files()`.
    pub fn file_at_cursor(&self) -> Option<usize> {
        let row = self.file_header_above(self.cursor)?;
        let RowKind::FileHeader(path) = &self.rows[row].kind else {
            return None;
        };
        self.file_index.get(path.as_str()).copied()
    }

    pub(super) fn current_hunk(&self) -> Option<usize> {
        self.rows.get(self.cursor).and_then(|r| r.kind.hunk())
    }

    /// The reader's choice if they have made one, otherwise the configured
    /// default. A review that has recorded a choice keeps it, so changing the
    /// config never moves a layout under someone mid-read.
    pub(super) fn split_diff(&self) -> bool {
        self.session.split_diff().unwrap_or(self.opts.split_diff)
    }

    pub(super) fn diff_mode(&self) -> DiffMode {
        if self.split_diff() {
            DiffMode::Split
        } else {
            DiffMode::Unified
        }
    }

    /// Is soft wrap on? Off until the reader presses `w` on this review.
    ///
    /// No config default, unlike `split_diff`: a layout preference is worth
    /// setting once, but wrapping is something a reader wants for the file
    /// they are on.
    pub(super) fn wrap_on(&self) -> bool {
        self.session.wrap().unwrap_or(false)
    }

    /// Test-only read of the code-wrap switch.
    #[doc(hidden)]
    pub fn wrap_on_for_test(&self) -> bool {
        self.wrap_on()
    }

    /// Does this row wrap right now?
    ///
    /// Prose always does. A group's description, a reviewer's note and a
    /// forge thread's comment are the reasons a plan, a finding and a review
    /// exist, they are never code, and a reader who cannot see the end of one
    /// is missing the point of the pane. File content is the reader's call,
    /// because wrapping code is often unwanted.
    pub(super) fn wraps(&self, row: &Row) -> bool {
        match row.kind {
            RowKind::GroupHeader | RowKind::Finding(..) | RowKind::Thread { .. } => true,
            RowKind::Diff(_) => self.wrap_on(),
            _ => false,
        }
    }

    /// How far this row's content is shifted right now.
    ///
    /// Only file content moves. A hunk header, a context boundary, a fold, a
    /// finding and a group header are chrome and prose: they are already
    /// fitted to the pane, and a `╱` band slid sideways says nothing. So the
    /// rows that shift are exactly the rows `wraps` calls the reader's call.
    ///
    /// Zero while the row wraps, because a wrapped row has no tail off the
    /// edge to reach.
    pub(super) fn shift(&self, row: &Row) -> usize {
        match row.kind {
            RowKind::Diff(_) if !self.wraps(row) => self.hscroll,
            _ => 0,
        }
    }

    /// How far the pane can shift before the widest line runs out.
    ///
    /// The widest overflow any file row has at the pane's current width. Past
    /// that there is nothing left to reveal, so `l` stops rather than walking
    /// the pane into blank space. `overflow` measures against the columns a
    /// row is actually drawn in, so the split layout needs no rule of its own.
    ///
    /// Measured on the keypress, not on every frame: shifting is rare and
    /// drawing is not.
    pub(super) fn max_hscroll(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Diff(_)))
            .map(|r| overflow(&r.content, self.viewport.detail_cols, self.split_offset))
            .max()
            .unwrap_or(0)
    }

    /// Bring the shift back inside what the pane can now reach.
    ///
    /// The bound moves when the rows do — a narrower terminal, `s`, another
    /// group, a hunk pulled open — and a shift left past it would leave the
    /// pane blank until the reader pressed something. Called from the two
    /// places that change it, rather than measured on every frame: the scan is
    /// O(rows) and drawing is not the place for one.
    pub(super) fn clamp_hscroll(&mut self) {
        // A pane at its left edge is already in range, and `max_hscroll` walks
        // every row to measure the widest overflow. That cost was accepted on
        // a keypress; a divider dragged across forty columns pays it forty
        // times, and the answer is zero every one of them.
        if self.hscroll == 0 {
            return;
        }
        self.hscroll = self.hscroll.min(self.max_hscroll());
    }

    /// Shift the diff pane sideways by `by` columns, or home it when `by` is
    /// `None`.
    ///
    /// Refuses while soft wrap is on rather than doing nothing quietly: with
    /// `w` on there is no tail off the edge, so a press that changed nothing
    /// would look like a key that does not work. A refusal is not a mode, so
    /// it takes the footer's passing message.
    pub(super) fn shift_pane(&mut self, by: Option<isize>) {
        if self.wrap_on() {
            self.status = "soft wrap is on · w turns it off".into();
            return;
        }
        self.hscroll = match by {
            None => 0,
            Some(by) => (self.hscroll as isize + by).max(0) as usize,
        }
        .min(self.max_hscroll());
    }

    /// Screen lines one row takes.
    ///
    /// The scroll budget and the drawing both read this, so they cannot
    /// disagree about where a row ends. Deliberately independent of the
    /// cursor: no row that wraps carries a marker or a hint, so nothing the
    /// cursor changes can change a height.
    ///
    /// Exposed for the same reason `viewport` is: a test asserting that the
    /// scroll budget counts screen LINES has to be able to count them, and
    /// counting rows instead is exactly the bug it guards against.
    pub fn row_height(&self, i: usize) -> usize {
        self.rows.get(i).map_or(1, |r| {
            compose_row_lines(
                &self.theme,
                &r.content,
                self.viewport.detail_cols,
                Paint::plain(self.wraps(r), self.split_offset),
            )
            .len()
        })
    }

    /// Toggle soft wrap. Row COUNTS do not change — a wrapped line is still one
    /// row — so nothing is rebuilt and the cursor stays where it was.
    pub(super) fn toggle_wrap(&mut self) {
        let on = !self.wrap_on();
        if let Err(e) = self.session.set_wrap(on) {
            self.status = format!("save failed: {e:#}");
            return;
        }
        // Only one of the two can be right at a time, and `shift_pane` says so
        // in the other direction. Wrapping while shifted left the offset in the
        // model where nothing could act on it — `shift` returns zero for a
        // wrapped row — so the pane came home and the footer went on claiming a
        // shift that was not happening.
        if on {
            self.hscroll = 0;
        }
        self.follow_cursor();
    }

    /// Toggle unified/split. Row counts differ between the modes, so keep the
    /// reviewer's place by re-anchoring the cursor to the current hunk.
    pub(super) fn toggle_split(&mut self) {
        let hunk = self.current_hunk();
        let on = !self.split_diff();
        if let Err(e) = self.session.set_split_diff(on) {
            self.status = format!("save failed: {e:#}");
            return;
        }
        self.rebuild_rows();
        if let Some(h) = hunk
            && let Some(pos) = self.rows.iter().position(|r| r.kind.hunk() == Some(h))
        {
            self.cursor = pos;
            self.follow_cursor();
        }
    }

    /// Open or close the selected group's folded remainder — the skim group's
    /// hunks past its exemplars, or a noise group entire.
    pub(super) fn toggle_group_fold(&mut self) {
        if self.view_mode != ViewMode::Groups {
            return;
        }
        let Some(g) = self.groups().get(self.selected_group) else {
            return;
        };
        let gid = g.id.clone();
        if !self.folds_open.insert(gid.clone()) {
            self.folds_open.remove(&gid);
        }
        self.rebuild_rows();
    }

    /// Persist the resume position through the session; surface failures in
    /// the status line rather than tearing the TUI down.
    pub(super) fn save_cursor(&mut self) {
        let id = match self.view_mode {
            ViewMode::Groups => self
                .groups()
                .get(self.selected_group)
                .map(|g| g.id.clone())
                .unwrap_or_default(),
            ViewMode::Files => self.tree_row_path(self.selected_file).unwrap_or_default(),
        };
        if let Err(e) = self.session.save_cursor(id, self.cursor) {
            self.status = format!("save failed: {e:#}");
        }
    }
}
