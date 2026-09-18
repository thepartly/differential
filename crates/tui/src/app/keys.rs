//! The whole input surface: one `handle_key`, one `handle_mouse`, one
//! `handle_paste`.
//!
//! Plain methods on the model returning effects, so a test drives the
//! reviewer without a terminal. Each is a dispatcher and one handler per mode:
//! the dispatcher does what every key does — the latch, the quit, the status,
//! `?` — and the mode's handler does the rest, so a key's meaning in a list is
//! read next to the list's other keys and nowhere else.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::rows::RowKind;

use super::draw::{
    centered_x, composer_area, composer_footer, delete_comment_area, delete_comment_footer,
    file_list_modal_area, findings_modal_area, findings_question, footer_fits, footer_row,
    pane_inner, publish_area, publish_footer, search_modal_area,
};
use super::text::{
    Hint, basename, file_list_rows, findings_entry_at_line, findings_rows, findings_skip, hint_at,
    hints_width, step_list,
};
use super::*;

/// Columns one press of `h`/`l` moves the diff pane.
///
/// One indent step. A single column is what vim moves and it is eighty
/// presses to reach column eighty; a half pane overshoots the word the reader
/// was following.
const SHIFT_STEP: isize = 8;

/// Columns one press of `alt--`/`alt-=` moves the divider.
///
/// Half of `SHIFT_STEP`, and for the opposite reason. The shift moves CONTENT
/// past a cursor that has to keep up, so it moves far enough to be worth the
/// press. The divider is chrome: eight columns of it crosses a fifth of a
/// hundred-column screen in one tap, and the reader who wants a move that big
/// drags it.
const RESIZE_STEP: i16 = 4;

/// Alt held, and ctrl not.
///
/// Deliberately not an exact match on the modifier set: `+` is shifted `=` on
/// most keyboards, and a terminal with the keyboard enhancements on reports the
/// shift as well — an exact match would make `alt-+` a dead key on exactly
/// those terminals. Ctrl is excluded so a chord nobody aimed cannot land here.
fn is_alt(m: KeyModifiers) -> bool {
    m.contains(KeyModifiers::ALT) && !m.contains(KeyModifiers::CONTROL)
}

/// A bare `y`, and nothing else, answers a question here. Some terminals
/// report ctrl-y as `Char('y')` with a modifier, and the irreversible actions
/// in this reviewer must not answer to a chord nobody aimed.
fn is_yes(key: KeyEvent) -> bool {
    (key.code, key.modifiers) == (KeyCode::Char('y'), KeyModifiers::NONE)
}

impl App {
    /// The composer, opened on `body` with the cursor at its end.
    ///
    /// One place for the three keys that open it — a note, a reply, a
    /// rewrite — so they cannot drift on what a composer is. It soft-wraps at
    /// word boundaries: a note is prose, and a line the reader cannot see the
    /// end of is a line they cannot finish. The key footer lives on the
    /// block's last inner row, and the padding keeps the text above it: a long
    /// note used to scroll into the footer and the two overwrote each other.
    fn composer(&self, body: &str, title: String) -> Box<TextArea<'static>> {
        let mut ta = if body.is_empty() {
            TextArea::default()
        } else {
            TextArea::new(body.lines().map(str::to_string).collect::<Vec<_>>())
        };
        ta.move_cursor(tui_textarea::CursorMove::End);
        ta.set_wrap_mode(tui_textarea::WrapMode::Word);
        ta.set_block(
            Block::default()
                .borders(Borders::ALL)
                .padding(ratatui::widgets::Padding::new(0, 0, 0, 1))
                .border_style(Style::default().fg(self.theme.header_fg))
                .title(title),
        );
        Box::new(ta)
    }
}

impl App {
    /// Text pasted into the terminal.
    ///
    /// Bracketed paste is enabled so a multi-line paste arrives as ONE event
    /// rather than as a run of keys that would each drive a normal-mode
    /// action. The event was then dropped, which meant pasting into the
    /// finding composer did nothing at all.
    ///
    /// Only the composer takes it: in normal mode there is no text field for
    /// it to land in, and a paste there is a mis-aimed one.
    pub fn handle_paste(&mut self, text: &str) {
        match &mut self.mode {
            Mode::Editing { editor, .. } => {
                editor.insert_str(text);
            }
            Mode::Search(_) => self.search_paste(text),
            _ => {}
        }
    }

    /// Key handling. Returns effects for the loop to execute.
    /// `enter` in the plan pane: a directory opens rather than jumping to the
    /// diff; anything else moves focus to the diff.
    fn enter_plan_entry(&mut self) {
        if !(self.view_mode == ViewMode::Files && self.toggle_dir()) {
            self.focus = Focus::Detail;
        }
    }

    /// `enter` in the file-list modal: close it on the selected file's header.
    fn jump_to_listed_file(&mut self) {
        let Mode::FileList {
            entries, selected, ..
        } = &self.mode
        else {
            return;
        };
        let row = entries[*selected].row_idx;
        self.mode = Mode::Normal;
        self.cursor = self.next_selectable(row, 1).unwrap_or(row);
        self.focus = Focus::Detail;
        self.follow_cursor();
    }

    /// `enter` in the findings modal: close it on the selected note or thread,
    /// or say why that is not possible.
    fn jump_to_listed_finding(&mut self) {
        let Mode::Findings {
            entries, selected, ..
        } = &self.mode
        else {
            return;
        };
        let e = &entries[*selected];
        let (id, orphaned, thread) = (e.id.clone(), e.orphaned, e.thread);
        // Assign the mode first: it is what drops the borrow this holds on it.
        self.mode = Mode::Normal;
        if orphaned {
            self.status = if thread {
                "that thread has no line in this diff".into()
            } else {
                "that finding has no line any more".into()
            };
        } else if thread {
            if !self.jump_to_thread(&id) {
                self.status = "could not reach that thread".into();
            }
        } else if !self.jump_to_finding(&id) {
            self.status = "could not reach that finding".into();
        }
    }

    /// The mouse. One rule: it acts on the pane under the pointer, and that
    /// pane takes focus. The wheel is `j`/`k` there, one row per notch; a
    /// click selects the row or entry under it; a click on what is already
    /// selected is `enter`; a click outside a box closes the box.
    ///
    /// The event loop hands over only the kinds this reads — the wheel's
    /// four directions and a left press — so a pointer moving across the
    /// screen never reaches the model, let alone repaints it.
    fn on_mouse(&mut self, m: MouseEvent) -> Vec<Effect> {
        let at = Position::new(m.column, m.row);
        // A held key and the wheel is the sideways wheel. Most mice have no
        // wheel of their own for it, and a terminal reports the held key as a
        // modifier on an ordinary notch. Any of the three: several terminals
        // keep shift for themselves — it is their "select text anyway" key
        // while a program has the mouse — and never send it on, so a reader
        // on one of those holds alt or ctrl instead.
        let held = KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL;
        let kind = match (m.kind, m.modifiers.intersects(held)) {
            (MouseEventKind::ScrollDown, true) => MouseEventKind::ScrollRight,
            (MouseEventKind::ScrollUp, true) => MouseEventKind::ScrollLeft,
            (kind, _) => kind,
        };
        let click = matches!(kind, MouseEventKind::Down(MouseButton::Left));
        let drag = matches!(kind, MouseEventKind::Drag(MouseButton::Left));
        // A drag with nothing grabbed is not this reviewer's. Dropped before
        // the status is cleared below: a reader who clicks a footer button and
        // then jiggles the mouse with the button still down would otherwise
        // lose the message that click just produced.
        if drag && self.divider_grab.is_none() {
            return Vec::new();
        }
        let step: isize = match kind {
            MouseEventKind::ScrollDown => 1,
            MouseEventKind::ScrollUp => -1,
            _ => 0,
        };
        // A key clears the footer, and so does a notch or a click: the message
        // answers "what did that just do", and this is the next thing done.
        self.status.clear();
        let panes = self.panes();
        // A modal's footer names its keys, and each is a button: a click on
        // one presses it. Looked for first, and over the model read-only, so
        // the presses are in hand before any arm below borrows it to change.
        if click && let Some(presses) = self.footer_presses_at(&panes, at) {
            return self.press_each(presses);
        }
        match &mut self.mode {
            Mode::Help(from) => {
                if click {
                    self.mode = *std::mem::replace(from, Box::new(Mode::Normal));
                }
            }
            Mode::Notice { .. } => {
                if click {
                    self.mode = Mode::Normal;
                }
            }
            // A box the caret owns, and two questions only `y` answers: their
            // footers took the click above, and nothing else in them does.
            Mode::Editing { .. } | Mode::Publish { .. } | Mode::DeleteComment { .. } => {}
            Mode::FileList { .. } => self.file_list_mouse(panes.body, at, click, step),
            Mode::Findings { .. } => self.findings_mouse(panes.body, at, click, step),
            Mode::Search(_) => {
                if step != 0 {
                    self.search_wheel(step > 0);
                } else if click {
                    // Which row of the box is a row of the LIST is the search
                    // module's rule, not this one's. Here there are two cases:
                    // a click inside the box, and one outside it.
                    match content_line(search_modal_area(panes.body), at) {
                        Some(line) => self.search_click(line),
                        None => self.close_search(),
                    }
                }
            }
            Mode::Normal => {
                // The dividers first, then the floats: each is a region of
                // the pane that takes the event and lets nothing beneath see
                // it. The order is the dividers', and `grab_divider` says why.
                if self.grab_divider(&panes, at, click, drag) || self.over_a_float(&panes, at) {
                    return Vec::new();
                }
                if panes.plan.contains(at) {
                    self.focus = Focus::Groups;
                    if step != 0 {
                        let idx = self.selected_entry().saturating_add_signed(step);
                        self.select_entry(idx);
                    } else if click
                        && let Some(line) = content_line(panes.plan, at)
                        && let Some(idx) = self.plan_entry_at_line(self.group_scroll + line)
                    {
                        if idx == self.selected_entry() {
                            self.enter_plan_entry();
                        } else {
                            self.select_entry(idx);
                        }
                    }
                } else if panes.detail.contains(at) {
                    self.focus = Focus::Detail;
                    match kind {
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                            self.move_cursor(step);
                        }
                        MouseEventKind::ScrollRight => self.shift_pane(Some(SHIFT_STEP)),
                        MouseEventKind::ScrollLeft => self.shift_pane(Some(-SHIFT_STEP)),
                        _ => {
                            // A row that cannot be selected — the group's
                            // header, a blank — leaves the cursor where it
                            // was, as `j` never lands on one either.
                            if click
                                && let Some(line) = content_line(panes.detail, at)
                                && let Some(row) = self.row_at_line(line)
                                && self.rows[row].kind.selectable()
                            {
                                self.cursor = row;
                                self.follow_cursor();
                            }
                        }
                    }
                }
            }
        }
        Vec::new()
    }

    /// The wheel and a click in the file list: a notch steps the selection,
    /// a click selects the entry under it — or, on the selected one, opens
    /// it — and a click outside the box closes it.
    fn file_list_mouse(&mut self, body: Rect, at: Position, click: bool, step: isize) {
        let Mode::FileList {
            entries,
            selected,
            scroll,
        } = &mut self.mode
        else {
            return;
        };
        if step != 0 {
            let rows = file_list_rows(entries.len(), self.viewport.body_rows);
            step_list(selected, scroll, entries.len(), rows, step > 0);
            return;
        }
        if !click {
            return;
        }
        match content_line(file_list_modal_area(body, entries), at) {
            None => self.mode = Mode::Normal,
            Some(line) => {
                let hit = *scroll + line;
                if hit >= entries.len() {
                    return;
                }
                if hit == *selected {
                    self.jump_to_listed_file();
                } else {
                    *selected = hit;
                }
            }
        }
    }

    /// The same in the findings list, with one more state: while `D` waits
    /// for its answer the wheel is not one, and a click off the footer is
    /// `n`, as any key but `y` is.
    fn findings_mouse(&mut self, body: Rect, at: Position, click: bool, step: isize) {
        let Mode::Findings {
            entries,
            selected,
            scroll,
            confirming,
        } = &mut self.mode
        else {
            return;
        };
        let rules = section_rules(entries);
        let area = findings_modal_area(body, entries.len(), rules.len());
        if *confirming {
            if click {
                *confirming = false;
                self.status = "nothing deleted".into();
            }
            return;
        }
        if step != 0 {
            let rows = findings_rows(entries.len(), rules.len(), self.viewport.body_rows);
            step_list(selected, scroll, entries.len(), rows, step > 0);
            return;
        }
        if !click {
            return;
        }
        match content_line(area, at) {
            None => self.mode = Mode::Normal,
            Some(line) => {
                let skip = findings_skip(*scroll, &rules);
                let Some(hit) = findings_entry_at_line(entries.len(), &rules, skip + line) else {
                    return;
                };
                if hit == *selected {
                    self.jump_to_listed_finding();
                } else {
                    *selected = hit;
                }
            }
        }
    }

    /// A press on one of the two dividers grabs it and the drags that follow
    /// move it; a press anywhere else lets go. True when the event was the
    /// divider's and nothing beneath should see it — a press that grabbed
    /// nothing has still let go, and is not.
    ///
    /// Ahead of the float guards because a float is drawn at its pane's
    /// full width, so each one lies across a divider's column and would
    /// swallow the grab. A divider wins: it is a line down the whole pane,
    /// and one that went dead where a transient box happened to sit would
    /// read as a divider that sometimes does not work.
    ///
    /// Each is grabbable only where its line IS drawn — the pane divider
    /// stops above the status row, the middle stays inside the diff pane's
    /// frame. A press off the end of a line is a press on nothing, whatever
    /// column it is in.
    fn grab_divider(&mut self, panes: &Panes, at: Position, click: bool, drag: bool) -> bool {
        let (left, right) = divider(panes);
        if click {
            self.divider_grab = if (at.x == left || at.x == right) && panes.body.contains(at) {
                Some(Grab::Panes(at.x as i16 - self.plan_cols() as i16))
            } else if self.split_column() == Some(at.x) && pane_inner(panes.detail).contains(at) {
                Some(Grab::Split)
            } else {
                None
            };
            if self.divider_grab.is_some() {
                return true;
            }
        }
        if let Some(grab) = self.divider_grab
            && drag
        {
            match grab {
                Grab::Panes(off) => self.drag_panes_to(at.x, off),
                Grab::Split => self.drag_split_to(at.x),
            }
            return true;
        }
        false
    }

    /// Whether `at` is on a float — the symbol peek, the group map, the file
    /// list. The floats are maps, deliberately not interactive: a click on
    /// one must not fall through to the pane beneath.
    fn over_a_float(&self, panes: &Panes, at: Position) -> bool {
        // The symbol float is a map too, and unlike the other two it
        // appears in every view — so its guard is not inside the
        // `Groups` check below.
        if self.peek_area(panes.detail).is_some_and(|a| a.contains(at)) {
            return true;
        }
        if self.view_mode == ViewMode::Groups {
            let float = match self.focus {
                Focus::Groups => self.group_map_area(panes.detail),
                Focus::Detail => self.file_list_area(panes.plan),
            };
            if float.is_some_and(|a| a.contains(at)) {
                return true;
            }
        }
        false
    }

    /// The keys a click at `at` presses on the open modal's footer, if it has
    /// one and the click is on a button of it.
    fn footer_presses_at(&self, panes: &Panes, at: Position) -> Option<Vec<KeyEvent>> {
        // The window's own footer names the keys of where the reader is, and
        // each is a button too — the same rule the modals already follow.
        if panes.status.contains(at) {
            let (hints, x0) = self.status_hints(panes.status);
            return hint_at(&hints, x0, at.x)
                .filter(|h| !h.presses.is_empty())
                .map(|h| h.presses.clone());
        }
        match &self.mode {
            Mode::Editing { editor, .. } => {
                let row = footer_row(composer_area(panes.body, editor));
                footer_presses(&composer_footer(), row, true, at)
            }
            // The `y` the footer shows is a `y` when clicked, and the clause
            // about every other key is one of those.
            Mode::Publish { plan } => {
                let lines = self.publish_lines(plan).len();
                let area = publish_area(panes.body, lines);
                float_footer_presses(&publish_footer(), area, lines, at)
            }
            Mode::DeleteComment { own } => {
                let lines = self.delete_comment_lines(own).len();
                let area = delete_comment_area(panes.body, lines);
                float_footer_presses(&delete_comment_footer(), area, lines, at)
            }
            Mode::Search(_) => {
                let area = search_modal_area(panes.body);
                footer_presses(&self.modal_footer(), footer_row(area), false, at)
            }
            Mode::Findings {
                entries,
                confirming,
                ..
            } => {
                let rules = section_rules(entries).len();
                let area = findings_modal_area(panes.body, entries.len(), rules);
                let hints = match *confirming {
                    true => {
                        let local = entries.iter().filter(|e| !e.thread && !e.published).count();
                        findings_question(local, self.published_count())
                    }
                    false => self.modal_footer(),
                };
                footer_presses(&hints, footer_row(area), false, at)
            }
            _ => None,
        }
    }

    /// A click on a footer hint presses the keys it names, one after another
    /// — `dd` is two — through the same handler a hand would reach.
    fn press_each(&mut self, presses: Vec<KeyEvent>) -> Vec<Effect> {
        // The inner handler, because this is still inside one event: the
        // click that reached the footer settles once, on its way out.
        presses.into_iter().flat_map(|k| self.on_key(k)).collect()
    }

    /// A key, and then the one thing that has to be true after any event.
    ///
    /// The float is settled HERE rather than inside each thing that moves a
    /// cursor. Seven of them do, and one had the line — so `g`, `G`, `ctrl+d`,
    /// `n`, a jump from the findings list and a click all left an answer
    /// pointing at a row the reader had gone from. A rule kept in one place is
    /// a rule that cannot be forgotten by the eighth.
    pub fn handle_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let effects = self.on_key(key);
        self.settle_peek();
        effects
    }

    /// The mouse, settled the same way and for the same reason.
    pub fn handle_mouse(&mut self, m: MouseEvent) -> Vec<Effect> {
        let effects = self.on_mouse(m);
        self.settle_peek();
        effects
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        // One latch, taken before anything reads a key. It used to be taken
        // inside the normal-mode block, which a modal's early return never
        // reaches — so `dd` could only ever mean one thing in one place.
        let pending_d = std::mem::take(&mut self.pending_d);
        // One key that quits from anywhere, the composer included. Every
        // other way out is a key of the place the reader is standing in, and
        // a reader who is lost is exactly the reader who cannot find one.
        // A draft in the box is lost; a finding already saved is not, since
        // the session writes on every change.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.save_cursor();
            return vec![Effect::Quit];
        }
        // The footer's message answers "what did that key just do", so the
        // next key is exactly when the answer stops being wanted. Cleared
        // HERE, before any handler runs: 35 places write this field and one
        // used to clear it, which made every one-off message permanent.
        self.status.clear();
        // A key is a hand off the mouse. Dropping the grab here means a modal
        // opened mid-gesture cannot leave one held over a screen where the
        // divider is no longer the thing under the pointer.
        self.divider_grab = None;
        // `?` opens help from every place whose keys help can answer for,
        // and the mode it was pressed in comes back when help closes. It is
        // NOT a key in the composer, where it is a character, nor in a
        // question, where every key but `y` is the no.
        if key.code == KeyCode::Char('?') && self.help_opens() {
            self.open_help();
            return Vec::new();
        }
        match &mut self.mode {
            // Help gives back the mode it was opened from: `?` in a modal
            // used to be unpressable for exactly this reason — any key would
            // have dropped the reader out of the list they were reading.
            Mode::Help(from) => {
                self.mode = *std::mem::replace(from, Box::new(Mode::Normal));
                Vec::new()
            }
            Mode::Notice { .. } => {
                self.mode = Mode::Normal;
                Vec::new()
            }
            Mode::FileList { .. } => self.file_list_key(key),
            Mode::Findings { .. } => self.findings_key(key, pending_d),
            Mode::DeleteComment { .. } => {
                let Mode::DeleteComment { own } = std::mem::replace(&mut self.mode, Mode::Normal)
                else {
                    unreachable!("matched above");
                };
                if is_yes(key) {
                    self.start_delete_comment(own);
                } else {
                    self.status = "nothing deleted".into();
                }
                Vec::new()
            }
            Mode::Publish { .. } => {
                // Only `y` sends. Anything else keeps every note local, which
                // is where it was: a slip must not be what notifies the author.
                let Mode::Publish { plan } = std::mem::replace(&mut self.mode, Mode::Normal) else {
                    unreachable!("matched above");
                };
                if is_yes(key) {
                    self.start_publish(plan);
                } else {
                    self.status = "nothing published".into();
                }
                Vec::new()
            }
            Mode::Editing { .. } => self.composer_key(key),
            // Every printable key types, which is why this arm takes the
            // whole event and why `?` is a character here and not help.
            Mode::Search(_) => {
                self.search_key(key);
                Vec::new()
            }
            Mode::Normal => self.normal_key(key, pending_d),
        }
    }

    /// A key in the file list: `j`/`k` step it, `enter` opens the file, and
    /// `esc`, `f` or `q` close it.
    fn file_list_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Mode::FileList {
            entries,
            selected,
            scroll,
        } = &mut self.mode
        else {
            return Vec::new();
        };
        let rows = file_list_rows(entries.len(), self.viewport.body_rows);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                step_list(selected, scroll, entries.len(), rows, true);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                step_list(selected, scroll, entries.len(), rows, false);
            }
            KeyCode::Enter => self.jump_to_listed_file(),
            // `/` reaches the search from here too: it is a key of
            // the review rather than of a pane, and a reader who has
            // opened the wrong list should not have to close it first.
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Esc | KeyCode::Char('f') | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        Vec::new()
    }

    /// A key in the findings list. `pending_d` is the `d` before this one,
    /// taken by the dispatcher: a handler reading the latch itself would find
    /// it already cleared.
    fn findings_key(&mut self, key: KeyEvent, pending_d: bool) -> Vec<Effect> {
        let Mode::Findings {
            entries,
            selected,
            scroll,
            confirming,
        } = &mut self.mode
        else {
            return Vec::new();
        };
        // Asking to delete everything: the next key answers, and only
        // `y` means yes. Anything else is a slip, and a slip must not
        // be the thing that empties the store.
        if *confirming {
            *confirming = false;
            if is_yes(key) {
                self.clear_findings();
            } else {
                self.status = "nothing deleted".into();
            }
            return Vec::new();
        }
        let rules = section_rules(entries).len();
        let rows = findings_rows(entries.len(), rules, self.viewport.body_rows);
        let mut copy = false;
        match (key.code, key.modifiers) {
            (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                step_list(selected, scroll, entries.len(), rows, true);
            }
            (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                step_list(selected, scroll, entries.len(), rows, false);
            }
            // Only the local notes are up for this: a published note is
            // the request's, and a thread is somebody else's.
            (KeyCode::Char('D'), _) => {
                if entries.iter().any(|e| !e.thread && !e.published) {
                    *confirming = true;
                } else {
                    self.status =
                        "nothing local to delete · dd deletes a published note on the forge".into();
                }
            }
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                if pending_d {
                    let (id, thread, published) = {
                        let e = &entries[*selected];
                        (e.id.clone(), e.thread, e.published)
                    };
                    // Whose the comment is, the session says; see
                    // `delete_finding_at_cursor` for the same rule.
                    let own = if thread {
                        self.session.own_root(&id)
                    } else if published {
                        self.session.own_of_finding(&id)
                    } else {
                        None
                    };
                    match (own, thread) {
                        (Some(own), _) => self.mode = Mode::DeleteComment { own },
                        (None, true) => self.status = NOT_YOURS.into(),
                        (None, false) => self.delete_finding(&id),
                    }
                } else {
                    self.pending_d = true;
                }
            }
            // Copy from here too, for the same reason `P` sends from
            // here: the list is where the reader sees what is not yet
            // on the request. The clipboard call is the caller's, so
            // this arm only says the summary is wanted.
            (KeyCode::Char('y'), _) => copy = true,
            // Publish from here too: the list is where the reader sees
            // what is not yet on the request, and it sends everything
            // that is not, exactly as P in the diff does.
            (KeyCode::Char('P'), _) => {
                self.mode = Mode::Normal;
                self.offer_publish();
            }
            (KeyCode::Enter, _) => self.jump_to_listed_finding(),
            (KeyCode::Char('/'), _) => self.open_search(),
            (KeyCode::Esc, _) | (KeyCode::Char('F'), _) | (KeyCode::Char('q'), _) => {
                self.mode = Mode::Normal;
            }
            _ => {}
        }
        if copy {
            return vec![Effect::CopySummary(self.findings_summary())];
        }
        Vec::new()
    }

    /// A key in the composer: `esc` discards, `enter` saves, a newline is
    /// `shift+enter` or a trailing `\`, and everything else types.
    fn composer_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Mode::Editing {
            hunk,
            lines,
            rewriting,
            reply_to,
            own,
            editor: textarea,
        } = &mut self.mode
        else {
            return Vec::new();
        };
        match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => {
                self.mode = Mode::Normal;
                self.status = "finding discarded".into();
                Vec::new()
            }
            // `enter` saves. A finding is usually one line, and the key
            // that ends a line is the key a reader reaches for to be
            // done with it. `ctrl-s` still saves too: it costs one arm,
            // and it is what the box said for two releases.
            //
            // A newline is `shift+enter` where the terminal reports it,
            // and a trailing `\` before `enter` where it does not —
            // most terminals send plain `enter` for both unless the
            // kitty keyboard protocol is on, which this reviewer
            // deliberately does not ask for.
            (KeyCode::Enter, m) if m.contains(KeyModifiers::SHIFT) => {
                textarea.insert_newline();
                Vec::new()
            }
            (KeyCode::Enter, _)
                // The character before the CURSOR, not the end of the
                // line: `delete_char` takes what the cursor sits after,
                // so a `\` at the end of a line the reader had gone
                // back to edit would have deleted something else.
                if {
                    let (row, col) = textarea.cursor();
                    textarea
                        .lines()
                        .get(row)
                        .and_then(|l| col.checked_sub(1).and_then(|i| l.chars().nth(i)))
                        == Some('\\')
                } =>
            {
                textarea.delete_char();
                textarea.insert_newline();
                Vec::new()
            }
            (KeyCode::Enter, _) | (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                let body = textarea.lines().join("\n").trim().to_string();
                // Read out before the mode is dropped; only the save
                // needs them, so only the save pays for the clones.
                let (hunk, lines, rewriting, reply_to, own) = (
                    *hunk,
                    lines.clone(),
                    rewriting.clone(),
                    reply_to.clone(),
                    own.clone(),
                );
                self.mode = Mode::Normal;
                // A comment on the forge: the text goes there first, and
                // emptying the box leaves it as it was, as with a note.
                if let Some(own) = own {
                    if body.is_empty() {
                        self.status = "comment left as it was".into();
                    } else {
                        self.start_edit_comment(own, body);
                    }
                    return Vec::new();
                }
                match (rewriting, reply_to, body.is_empty()) {
                    // Emptying the box does NOT delete the note. That
                    // is `dd`, which is a deliberate press; a note lost
                    // to a stray `ctrl-u` and an `enter` is not.
                    (Some(_), _, true) => self.status = "finding left as it was".into(),
                    (Some(id), _, false) => self.rewrite_finding(&id, body),
                    (None, _, true) => self.status = "empty finding discarded".into(),
                    (None, Some(thread), false) => self.add_reply(&thread, body),
                    (None, None, false) => self.add_finding(hunk, lines, body),
                }
                Vec::new()
            }
            _ => {
                textarea.input(key);
                Vec::new()
            }
        }
    }

    /// A key in the review proper: one table, because that is what a key
    /// table reads as. Forty of its arms are one call each; the two that
    /// are not, `c` and `r`, have names of their own below.
    fn normal_key(&mut self, key: KeyEvent, pending_d: bool) -> Vec<Effect> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => {
                self.save_cursor();
                return vec![Effect::Quit];
            }
            (KeyCode::Tab, _) => {
                self.focus = match self.focus {
                    Focus::Groups => Focus::Detail,
                    Focus::Detail => Focus::Groups,
                }
            }
            (KeyCode::Enter, _) if self.focus == Focus::Groups => self.enter_plan_entry(),
            (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => match self.focus {
                Focus::Groups => self.select_entry(self.selected_entry() + 1),
                Focus::Detail => self.move_cursor(1),
            },
            (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => match self.focus {
                Focus::Groups => self.select_entry(self.selected_entry().saturating_sub(1)),
                Focus::Detail => self.move_cursor(-1),
            },
            (KeyCode::Char('J'), _) | (KeyCode::Char('}'), _) => {
                self.select_entry(self.selected_entry() + 1)
            }
            (KeyCode::Char('K'), _) | (KeyCode::Char('{'), _) => {
                self.select_entry(self.selected_entry().saturating_sub(1))
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.cursor = self.half_page(self.cursor, 1);
                self.cursor = self.next_selectable(self.cursor, -1).unwrap_or(self.cursor);
                self.follow_cursor();
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.cursor = self.half_page(self.cursor, -1);
                self.cursor = self.next_selectable(self.cursor, 1).unwrap_or(self.cursor);
                self.follow_cursor();
            }
            (KeyCode::Char('g'), _) => {
                self.cursor = self.next_selectable(0, 1).unwrap_or(0);
                self.follow_cursor();
            }
            (KeyCode::Char('G'), _) => {
                self.cursor = self
                    .next_selectable(self.rows.len().saturating_sub(1), -1)
                    .unwrap_or(0);
                self.follow_cursor();
            }
            // One key for "show me what is being withheld", acting on the pane
            // it is pressed in. Reading the diff that is a context boundary or
            // a folded remainder; reading the file tree it is a directory.
            //
            // The pane matters: `self.cursor` is a DIFF row wherever the focus
            // is, so without it a press in the tree opened whatever the diff's
            // cursor happened to be parked on.
            (KeyCode::Char('z'), _)
                if self.focus == Focus::Detail
                    && matches!(
                        self.rows.get(self.cursor).map(|r| &r.kind),
                        Some(RowKind::ContextEdge { .. })
                    ) =>
            {
                self.expand_at_cursor();
            }
            // A resolved thread is collapsed to its header; `z` opens and
            // closes it, the way `z` opens a fold or a context gap elsewhere.
            (KeyCode::Char('z'), _)
                if self.focus == Focus::Detail
                    && self.thread_at_cursor().is_some_and(|t| t.resolved) =>
            {
                self.toggle_thread_expanded();
            }
            (KeyCode::Char('z'), _)
                if self.focus == Focus::Groups && self.view_mode == ViewMode::Files =>
            {
                self.toggle_dir();
            }
            // A declaration the reader cannot see is being withheld too, which
            // is why this is `z` and not a key of its own. The row decides:
            // only a code row whose line resolves a symbol gets here, so a row
            // with nothing to show falls through to the fold below, exactly as
            // it did before (ADR 0032).
            //
            // Below the boundary and thread arms deliberately. Those rows have
            // no new-side line, so they resolve nothing and could not reach
            // here anyway — but the order is what says which meaning wins,
            // and leaving that to chance is how a key starts meaning two
            // things on one row.
            (KeyCode::Char('z'), _)
                if self.focus == Focus::Detail && !self.peekable().is_empty() =>
            {
                self.step_peek();
            }
            (KeyCode::Char('z'), _) => self.toggle_group_fold(),
            (KeyCode::Char('n'), KeyModifiers::NONE) => self.jump_hunk(1),
            (KeyCode::Char('N'), _) => self.jump_hunk(-1),
            (KeyCode::Char('s'), KeyModifiers::NONE) => self.toggle_split(),
            (KeyCode::Char('w'), KeyModifiers::NONE) => self.toggle_wrap(),
            // Sideways, in the pane you are in. A line wider than its column
            // is cut in either layout and twice as often in split, where the
            // column is half a pane; `w` is the other answer and the two are
            // exclusive, which `shift_pane` says out loud.
            //
            // Eight columns is one indent step, so a press moves a distance
            // worth pressing a key for. `0` is the way back, in one.
            (KeyCode::Char('l'), KeyModifiers::NONE) | (KeyCode::Right, _)
                if self.focus == Focus::Detail =>
            {
                self.shift_pane(Some(SHIFT_STEP));
            }
            (KeyCode::Char('h'), KeyModifiers::NONE) | (KeyCode::Left, _)
                if self.focus == Focus::Detail =>
            {
                self.shift_pane(Some(-SHIFT_STEP));
            }
            (KeyCode::Char('0'), KeyModifiers::NONE) if self.focus == Focus::Detail => {
                self.shift_pane(None);
            }
            // The divider, as zellij moves it. The second key in this reviewer
            // that does not act on the pane you are in: it always names the
            // DIFF pane, because making room for the diff is the thing the
            // reader wants and the left pane is what pays for it.
            (KeyCode::Char('=') | KeyCode::Char('+'), m) if is_alt(m) => {
                self.resize_diff(RESIZE_STEP);
            }
            (KeyCode::Char('-'), m) if is_alt(m) => {
                self.resize_diff(-RESIZE_STEP);
            }
            // One key for files, acting on the pane it is pressed in. In the
            // left pane that is which list of files you are reading — the
            // plan or the tree; in the diff pane it is which file you want to
            // be looking at. `v` used to switch the left pane from either
            // side, which meant a key in one pane silently rearranged the
            // other.
            // Every finding at once, from either pane. It is a fact about the
            // review rather than about a pane, unlike `f`.
            (KeyCode::Char('F'), _) => self.open_findings(),
            // And so is a word. `/` is the one key in this reviewer that does
            // NOT act on the pane it is pressed in: what a name is and where
            // it lives is a fact about the branch, and the reader asking has
            // by definition not found the pane it is in yet.
            (KeyCode::Char('/'), _) => self.open_search(),
            (KeyCode::Char('f'), KeyModifiers::NONE) => match self.focus {
                Focus::Groups => self.toggle_file_view(),
                Focus::Detail => self.open_file_list(),
            },
            (KeyCode::Char(' '), _) => self.toggle_reviewed(),
            // A selection, so a finding can be about the lines it is about.
            // One field, not a mode: `j`/`k` keep moving the cursor and the
            // selection is the span between the two ends, which is what makes
            // `V` cost nothing to explain.
            // A toggle. `v` is how a reader gets into a selection, so it is
            // the key their hand is on to get out of one — `esc` works too,
            // and so does `c`, which leaves by using it.
            // Neither end writes a message. The footer's pill IS the state:
            // it appears while a selection is open and goes when it closes,
            // where a passing message described a MODE in the same grey slot
            // that "finding saved" uses for something already over.
            (KeyCode::Char('v'), KeyModifiers::NONE) if self.visual.is_some() => {
                self.visual = None;
            }
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                if self.rows.get(self.cursor).is_some_and(|r| r.line.is_some()) {
                    self.visual = Some(self.cursor);
                } else {
                    // A refusal, not a mode: nothing happened, so nothing on
                    // screen says why unless the footer does.
                    self.status = "move onto a line first".into();
                }
            }
            // Before the selection's `esc`, so one press closes one thing:
            // the float came last, so it goes first, and a second `esc` still
            // drops the selection underneath it.
            (KeyCode::Esc, _) if self.peek.is_some() => {
                self.peek = None;
            }
            (KeyCode::Esc, _) if self.visual.is_some() => {
                self.visual = None;
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => self.edit_at_cursor(),
            (KeyCode::Char('d'), KeyModifiers::NONE) => {
                if pending_d {
                    self.delete_finding_at_cursor();
                } else {
                    self.pending_d = true;
                }
            }
            (KeyCode::Char('y'), _) => {
                return vec![Effect::CopySummary(self.findings_summary())];
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => self.reply_at_cursor(),
            // The forge's threads (ADR 0029): resolve the one under the cursor,
            // or fetch them all again. Both go out on a worker thread and
            // land through `poll_forge`.
            (KeyCode::Char('x'), KeyModifiers::NONE) => self.toggle_thread_resolved(),
            (KeyCode::Char('R'), _) => self.start_fetch(),
            (KeyCode::Char('P'), _) => self.offer_publish(),
            _ => {}
        }
        Vec::new()
    }

    /// `c`: open the composer on what is under the cursor. On a comment of
    /// the reader's the box opens with its text, and saving sends the new
    /// text to the forge. On anyone else's comment there is nothing of the
    /// reader's to edit, and the footer says so — `r` replies there instead.
    /// Anywhere else it files a note.
    fn edit_at_cursor(&mut self) {
        if let Some(own) = self.own_comment_at_cursor() {
            let hunk = self.current_hunk().unwrap_or(0);
            let ta = self.composer(&own.body, format!(" {} · on the request ", own.at));
            self.visual = None;
            self.mode = Mode::Editing {
                hunk,
                lines: None,
                rewriting: None,
                reply_to: None,
                own: Some(own),
                editor: ta,
            };
        } else if self.thread_at_cursor().is_some() {
            self.status = NOT_YOURS.into();
        } else if let Some(h) = self.current_hunk() {
            // A line already carrying a note opens THAT note. Two
            // notes on one line would each be half the story, and
            // there was no way to correct a typo but delete and
            // retype. A SELECTION is the exception: picking a run of
            // lines is asking for a note about the run.
            let existing = self
                .visual
                .is_none()
                .then(|| self.finding_at_cursor())
                .flatten()
                .map(|f| (f.id.clone(), f.body.clone(), f.anchor.line_span()));
            let lines = self.selected_lines();
            // Name what is being annotated: a note whose subject you
            // cannot see is a note you have to trust yourself to have
            // written carefully.
            let hunk = &self.session.doc().hunks[h];
            let file = basename(&hunk.file);
            let at = match (&existing, &lines) {
                // A note's own anchor, which may not be the row the
                // cursor is on — it can have re-anchored to the hunk.
                (Some((_, _, span)), _) => format!("L{span}"),
                (None, Some(l)) if l.end > l.start => format!("L{}-{}", l.start, l.end),
                (None, Some(l)) => format!("L{}", l.start),
                // No line under the cursor — a hunk header, a fold.
                // The finding anchors the hunk, so the title says so.
                (None, None) if hunk.new_count > 1 => format!(
                    "L{}-{}",
                    hunk.new_start,
                    hunk.new_start + hunk.new_count - 1
                ),
                (None, None) => format!("L{}", hunk.new_start),
            };
            let body = existing.as_ref().map(|(_, b, _)| b.as_str()).unwrap_or("");
            let ta = self.composer(body, format!(" {file} · {at} "));
            self.visual = None;
            self.mode = Mode::Editing {
                hunk: h,
                lines,
                rewriting: existing.map(|(id, _, _)| id),
                reply_to: None,
                own: None,
                editor: ta,
            };
        } else {
            self.status = "move onto a hunk first".into();
        }
    }

    /// `r`: reply to the review thread under the cursor — the reader's own
    /// thread or anyone's. The reply is a finding carrying the thread's id
    /// until a publish sends it (ADR 0029).
    fn reply_at_cursor(&mut self) {
        if let Some(t) = self.thread_at_cursor() {
            let (id, author, path) = (
                t.id.clone(),
                t.root().map(|c| c.author.clone()).unwrap_or_default(),
                t.path.clone(),
            );
            let hunk = self.current_hunk().unwrap_or(0);
            let ta = self.composer("", format!(" {} · reply to {author} ", basename(&path)));
            self.visual = None;
            self.mode = Mode::Editing {
                hunk,
                lines: None,
                rewriting: None,
                reply_to: Some(id),
                own: None,
                editor: ta,
            };
        } else {
            self.status = "r replies to a review thread".into();
        }
    }
}

/// The content line of a framed box under `at`, counted from the box's first
/// line inside its frame — or `None` when `at` is on the frame or outside.
fn content_line(area: Rect, at: Position) -> Option<usize> {
    let inner = pane_inner(area);
    inner.contains(at).then(|| usize::from(at.y - inner.y))
}

/// The presses a click at `at` on a footer makes, or `None` off the footer
/// or on words that only read. `row` is where the footer is drawn; a centred
/// footer starts where ratatui's centre alignment starts it.
fn footer_presses(
    hints: &[Hint],
    row: Rect,
    centered: bool,
    at: Position,
) -> Option<Vec<KeyEvent>> {
    if !row.contains(at) {
        return None;
    }
    let x0 = if centered {
        centered_x(row, hints_width(hints))
    } else {
        row.x
    };
    hint_at(hints, x0, at.x)
        .filter(|h| !h.presses.is_empty())
        .map(|h| h.presses.clone())
}

/// The same, for a float whose footer is the last of its `lines` — which a
/// box clamped to the body may have cut off, in which case nothing is there
/// to click.
fn float_footer_presses(
    hints: &[Hint],
    area: Rect,
    lines: usize,
    at: Position,
) -> Option<Vec<KeyEvent>> {
    footer_fits(area, lines)
        .then(|| footer_presses(hints, footer_row(area), false, at))
        .flatten()
}
