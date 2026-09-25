//! `ctrl-o`: back to where a jump left from.
//!
//! Every jump in this reviewer — `enter` on the symbol float, on a search hit,
//! in the file list and in the findings list — pushes where the reader stood,
//! and `ctrl-o` pops one. A stack rather than one place, so a chain of
//! declarations followed down can be walked back up it.
//!
//! The push is ONE wrapper, [`App::jumping`], rather than a line in each jump,
//! for the reason `settle_peek` gives: a rule kept in one place cannot be
//! forgotten by the next jump to be written.

use super::{App, Focus, Place, ViewMode};

/// How many places the stack keeps before it drops the oldest. Vim's number;
/// nobody walks back a hundred jumps, and the bound keeps a long session from
/// growing it without end.
const MOST_PLACES: usize = 100;

impl App {
    /// Where the reader stands now.
    fn place(&self) -> Place {
        let at = self.rows.get(self.cursor).and_then(|r| {
            let l = r.line.as_ref()?;
            Some((
                self.file_path_above(self.cursor)?.to_string(),
                l.side,
                l.line,
            ))
        });
        Place {
            view: self.view_mode,
            entry: self.entry(),
            row: self.cursor,
            at,
        }
    }

    /// The selected entry of the list the left pane shows.
    fn entry(&self) -> usize {
        match self.view_mode {
            ViewMode::Groups => self.selected_group,
            ViewMode::Files => self.selected_file,
        }
    }

    /// Run a jump, and remember where it left from.
    ///
    /// Only a jump that MOVED the reader is remembered: one that failed, or
    /// landed on the row it started from, would make `ctrl-o` a key that
    /// sometimes does nothing.
    pub(super) fn jumping(&mut self, go: impl FnOnce(&mut Self)) {
        let from = self.place();
        go(self);
        if (self.view_mode, self.entry(), self.cursor) == (from.view, from.entry, from.row) {
            return;
        }
        if self.jumps.len() == MOST_PLACES {
            self.jumps.remove(0);
        }
        self.jumps.push(from);
    }

    /// `ctrl-o`: stand where the last jump left from.
    ///
    /// Only the cursor comes back. A float open there before the jump is not
    /// reopened: the underlines say the row has something to show, and `z`
    /// is one key away.
    pub(super) fn go_back(&mut self) {
        let Some(place) = self.jumps.pop() else {
            self.status = "nowhere to go back to".into();
            return;
        };
        // Cleared on `f`, so this holds; a place from the other view would
        // name a row that is not there.
        debug_assert_eq!(place.view, self.view_mode);
        if self.entry() != place.entry {
            self.select_entry(place.entry);
        }
        let row = place
            .at
            .as_ref()
            .and_then(|(path, side, line)| self.row_of_line(path, side, *line))
            .unwrap_or_else(|| place.row.min(self.rows.len().saturating_sub(1)));
        self.cursor = row;
        self.focus = Focus::Detail;
        self.follow_cursor();
    }
}
