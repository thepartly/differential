//! TUI model tests: key events → state transitions + a TestBackend draw smoke
//! test. No real terminal, no real LLM.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use differential_engine::ReviewSession;
use differential_engine::gitio::Repo;
use differential_engine::ports::ReviewStore;
use differential_engine::store::FsReviewStore;
use differential_testutil::{FakeBackend, TestRepo, github_request, json_group, remote_comment};
use differential_tui::app::{
    App, Effect, Focus, Mode, Reading, ReviewOptions, Search, ViewMode, Viewport,
};
use differential_tui::rows::{BoxStyle, LineOrigin, RowFactory, RowKind};

use differential_tui::theme::Theme;
use differential_tui::window::Side;

/// The default palette, built once. `Theme::named` parses the syntax set, and
/// the assertions below reach for it a couple of dozen times.
fn theme() -> &'static Theme {
    static T: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    T.get_or_init(|| Theme::named(differential_engine::config::ThemeName::Dark))
}
use ratatui::style::Color;

/// First-listed class (the largest) becomes the skim sweep; the rest are
/// focus work — so the skim group has a foldable remainder.
fn skim_first_backend() -> FakeBackend {
    FakeBackend::new("fake", |ids| {
        let skim = ids.first().map(String::as_str).unwrap_or("C0");
        let rest: Vec<&str> = ids.iter().skip(1).map(String::as_str).collect();
        let mut groups = vec![json_group("Skim sweep", "skim", &[skim])];
        if !rest.is_empty() {
            groups.push(json_group("Focus work", "focus", &rest));
        }
        format!(r#"{{"groups": [{}]}}"#, groups.join(", "))
    })
}

/// Open an App over HEAD~1..HEAD of `r` with an explicit backend.
fn open_app_with(r: &TestRepo, backend: &FakeBackend, store: &str) -> App {
    open_app_with_opts(r, backend, store, ReviewOptions::default())
}

/// Options pinning the diff layout, so a test that depends on one says so
/// rather than inheriting whatever the default happens to be.
fn laid_out(split_diff: bool) -> ReviewOptions {
    ReviewOptions {
        split_diff,
        ..ReviewOptions::default()
    }
}

/// The same, with the renderer's options spelled out — for the tests that care
/// what a review opens as before the reader has chosen.
fn open_app_with_opts(
    r: &TestRepo,
    backend: &FakeBackend,
    store: &str,
    opts: ReviewOptions,
) -> App {
    let repo = Repo::open(Path::new(&r.root)).unwrap();
    let base = r.git(&["rev-parse", "HEAD~1"]);
    let head = r.git(&["rev-parse", "HEAD"]);
    let out = differential_testutil::grouped_output(r, &base, &head, backend, None);
    let factory = RowFactory::new(repo, out.base.clone(), out.head.clone());
    let session = ReviewSession::open(
        FsReviewStore::at(r.root.join(store)).unwrap(),
        out.document.unwrap(),
        out.view,
    )
    .unwrap();
    App::new(session, factory, opts, theme().clone())
}

/// A backend that answers with ONE focus group per class.
///
/// Four fixtures built this same closure, three of them word for word. Every
/// class staying its own group is what keeps the dependency graph's nodes
/// apart, so a fixture that quietly merged two would test the wrong thing.
fn one_group_per_class() -> FakeBackend {
    FakeBackend::new("fake", |ids| {
        let groups: Vec<String> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| json_group(&format!("Group {i}"), "focus", &[id.as_str()]))
            .collect();
        format!(r#"{{"groups": [{}]}}"#, groups.join(", "))
    })
}

/// Open an App over HEAD~1..HEAD of `r`, with the review store inside the
/// repo dir — reopening yields a resumed session over the same store.
fn open_app(r: &TestRepo) -> App {
    open_app_with_opts(
        r,
        &skim_first_backend(),
        ".dfr-test-store",
        ReviewOptions::default(),
    )
}

/// Repo with one behavioural change + a 3-file repeated edit (skim material).
fn make_app() -> (TestRepo, App) {
    let r = TestRepo::new();
    r.write("src/main.txt", b"fn main() { run_slowly() }\n");
    for n in ["a", "b", "c"] {
        r.write(
            &format!("src/{n}.txt"),
            b"use old_helper_name;\nother content here\n",
        );
    }
    r.commit_all("base");
    r.write("src/main.txt", b"fn main() { run_with_retries(3) }\n");
    for n in ["a", "b", "c"] {
        r.write(
            &format!("src/{n}.txt"),
            b"use new_helper_name;\nother content here\n",
        );
    }
    r.commit_all("head");
    let app = open_app(&r);
    (r, app)
}

/// `make_app`, with the renderer's options spelled out — for the tests that
/// care what a review opens as before the reader has chosen a layout.
fn make_app_with(opts: ReviewOptions) -> (TestRepo, App) {
    let (r, _) = make_app();
    let backend = skim_first_backend();
    let app = open_app_with_opts(&r, &backend, ".dfr-opts-store", opts);
    (r, app)
}

/// A repo with real nesting. `make_app`'s four files all sit directly under
/// `src/`, so folding a directory that CONTAINS one was never exercised —
/// which is how #98 survived.
///
/// Two sibling subdirectories under one parent is the minimum the bug needs,
/// and it needs them at both levels: `src/a` and `src/b` for a fold of `src/`,
/// `src/a/inner` and `src/a/other` for a fold of `src/a`. `docs/` proves the
/// fold stops at its own subtree.
fn app_with_nested_dirs() -> (TestRepo, App) {
    let r = TestRepo::new();
    let files = [
        "src/main.txt",
        "src/a/x.txt",
        "src/a/inner/i.txt",
        "src/a/other/o.txt",
        "src/b/z.txt",
        "docs/readme.txt",
    ];
    for path in files {
        r.write(path, b"use old_helper_name;\nother content here\n");
    }
    r.commit_all("base");
    for path in files {
        r.write(path, b"use new_helper_name;\nother content here\n");
    }
    r.commit_all("head");
    let app = open_app_with(&r, &skim_first_backend(), ".dfr-nested-store");
    (r, app)
}

/// Switch the left pane between the reading plan and the file tree. `f` acts
/// on the pane it is pressed in, so this presses it there and puts focus back.
fn switch_left_pane(app: &mut App) {
    let focus = app.focus;
    app.focus = Focus::Groups;
    app.handle_key(key('f'));
    app.focus = focus;
}

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn navigation_group_switch_and_focus() {
    let (_r, mut app) = make_app();
    assert_eq!(app.groups().len(), 2);
    assert_eq!(app.focus, Focus::Groups);

    // j in the groups pane switches group and rebuilds rows.
    let before_rows: Vec<_> = app.rows.iter().map(|r| r.kind.clone()).collect();
    app.handle_key(key('j'));
    assert_eq!(app.selected_group, 1);
    let after_rows: Vec<_> = app.rows.iter().map(|r| r.kind.clone()).collect();
    assert_ne!(before_rows, after_rows);

    // Skim group shows a fold row; z opens it.
    assert!(app.rows.iter().any(|r| r.kind == RowKind::Fold));
    app.handle_key(key('z'));
    assert!(!app.rows.iter().any(|r| r.kind == RowKind::Fold));

    // Tab moves focus to the diff pane; j moves the cursor over selectables.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Detail);
    let c0 = app.cursor;
    app.handle_key(key('j'));
    assert!(app.cursor > c0);
    assert!(app.rows[app.cursor].kind.selectable());
}

#[test]
fn space_toggles_hunk_reviewed_and_persists() {
    let (r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(' '));
    assert_eq!(app.session.reviewed_count(), 1);

    // The renderer is stateless: the mark is already on disk.
    let store = FsReviewStore::at(r.root.join(".dfr-test-store")).unwrap();
    assert_eq!(store.load_state().unwrap().reviewed_hunks.len(), 1);

    // Toggling again clears it.
    app.handle_key(key(' '));
    assert_eq!(app.session.reviewed_count(), 0);
    assert!(store.load_state().unwrap().reviewed_hunks.is_empty());
}

#[test]
fn finding_lifecycle_add_copy_delete() {
    let (_r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    // c opens the editor on the current hunk.
    app.handle_key(key('c'));
    assert!(matches!(app.mode, Mode::Editing { .. }));
    for ch in "off by one".chars() {
        app.handle_key(key(ch));
    }
    app.handle_key(ctrl('s'));
    assert_eq!(app.session.findings().len(), 1);
    assert_eq!(app.session.findings()[0].body, "off by one");
    assert!(!app.session.findings()[0].anchor.hunk_digest.is_empty());

    // The finding renders as a row and the summary contains it.
    assert!(
        app.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::Finding(_, _)))
    );
    let effects = app.handle_key(key('y'));
    match effects.first() {
        Some(Effect::CopySummary(text)) => {
            assert!(text.contains("off by one"));
            assert!(text.contains(":"));
        }
        other => panic!("expected a copied summary, got {other:?}"),
    }

    // dd on the finding row deletes it.
    let finding_row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(_, _)))
        .unwrap();
    app.cursor = finding_row;
    app.handle_key(key('d'));
    app.handle_key(key('d'));
    assert!(app.session.findings().is_empty());
}

#[test]
fn esc_discards_editor_and_empty_findings_are_dropped() {
    let (_r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('c'));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
    app.handle_key(key('c'));
    let effects = app.handle_key(ctrl('s')); // empty body
    assert!(effects.is_empty());
    assert!(app.session.findings().is_empty());
}

#[test]
fn quit_saves_cursor() {
    let (r, mut app) = make_app();
    let effects = app.handle_key(key('q'));
    assert_eq!(effects, vec![Effect::Quit]);
    let store = FsReviewStore::at(r.root.join(".dfr-test-store")).unwrap();
    assert!(store.load_state().unwrap().cursor.is_some());
}

#[test]
fn group_counts_files_and_line_totals() {
    let (_r, app) = make_app();
    // Across both groups: 4 files, 4 hunks, each hunk one line replaced.
    let files: usize = app.groups().iter().map(|g| g.n_files).sum();
    let adds: usize = app.groups().iter().map(|g| g.counts.adds).sum();
    let dels: usize = app.groups().iter().map(|g| g.counts.dels).sum();
    assert_eq!(files, 4);
    assert_eq!(adds, 4);
    assert_eq!(dels, 4);
    // The 3-file repeated edit lands in one group.
    assert!(app.groups().iter().any(|g| g.n_files == 3));
}

#[test]
fn file_view_lists_all_files_and_shares_review_marks() {
    use differential_tui::app::ViewMode;
    let (r, mut app) = make_app();
    assert_eq!(app.view_mode, ViewMode::Groups);

    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Files);
    assert_eq!(app.files().len(), 4);
    let store = FsReviewStore::at(r.root.join(".dfr-test-store")).unwrap();
    assert!(store.load_state().unwrap().file_view);

    // The right pane shows the selected file: one header + its hunks.
    assert!(
        app.rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::FileHeader(_)))
    );
    assert!(
        app.rows
            .iter()
            .any(|row| matches!(row.kind, RowKind::HunkHeader { .. }))
    );

    // space in file view marks the hunk — visible back in group view too.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(' '));
    let marked = app.session.reviewed_count();
    assert!(marked >= 1);
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Groups);
    assert!(!store.load_state().unwrap().file_view);
    assert_eq!(
        app.session.reviewed_count(),
        marked,
        "the two views read one set of marks"
    );
}

#[test]
fn file_view_shows_hunks_across_groups_with_labels() {
    // One file with two separated edits of different shapes: the two hunks
    // land in different classes and (per the fake backend) different groups.
    let r = TestRepo::new();
    r.write(
        "src/dual.txt",
        b"first_region = old_alpha\npad1\npad2\npad3\npad4\npad5\nfn second() { call_old_api() }\n",
    );
    r.commit_all("base");
    r.write(
        "src/dual.txt",
        b"first_region = new_beta_value\npad1\npad2\npad3\npad4\npad5\nfn second() { call_new_api(42) }\n",
    );
    r.commit_all("head");
    let mut app = open_app(&r);
    assert_eq!(app.groups().len(), 2, "two shapes → two groups");

    switch_left_pane(&mut app);
    let hunk_headers = app
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::HunkHeader { .. }))
        .count();
    assert_eq!(
        hunk_headers, 2,
        "file view shows the file's hunks from BOTH groups"
    );
}

#[test]
fn file_view_resume_restores_view_and_file() {
    use differential_tui::app::ViewMode;
    let (r, mut app) = make_app();
    switch_left_pane(&mut app);
    app.handle_key(key('J')); // next tree row
    let path = app.selected_path().unwrap();
    app.handle_key(key('q'));
    drop(app);

    let app2 = open_app(&r);
    assert_eq!(app2.view_mode, ViewMode::Files);
    assert_eq!(app2.selected_path().unwrap(), path);
}

#[test]
fn file_list_modal_opens_jumps_and_closes() {
    use differential_tui::app::Mode;
    let (_r, mut app) = make_app();
    // `f` opens the list from the DIFF pane; in the plan pane it switches the
    // left pane instead.
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    let (n_entries, first_path) = match &app.mode {
        Mode::FileList { entries, .. } => (entries.len(), entries[0].path.clone()),
        _ => panic!("f should open the file list"),
    };
    assert!(n_entries >= 1);

    // Enter jumps the cursor to (the first selectable after) that header.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.focus, Focus::Detail);
    let header_row = app
        .rows
        .iter()
        .position(|r| matches!(&r.kind, RowKind::FileHeader(p) if *p == first_path))
        .unwrap();
    assert!(app.cursor >= header_row);
    assert!(app.rows[app.cursor].kind.selectable());

    // Esc closes without moving.
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn split_view_toggles_and_keeps_cursor_on_hunk() {
    use differential_tui::rows::RowContent;
    let (r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    // A review nobody has chosen a layout for opens split, because that is the
    // configured default. Nothing is written until the reader presses `s`.
    assert!(
        app.rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. }))
    );
    let store = FsReviewStore::at(r.root.join(".dfr-test-store")).unwrap();
    assert_eq!(
        store.load_state().unwrap().split_diff,
        None,
        "a default is not a choice, so nothing is recorded"
    );
    let split_diff_rows = app
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Diff(_)))
        .count();

    let hunk_before = app.rows[app.cursor].kind.hunk();
    app.handle_key(key('s'));

    // Unified now, the choice persisted, and a Modified line takes two rows
    // where the split layout took one.
    assert!(
        !app.rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. }))
    );
    let unified_diff_rows = app
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Diff(_)))
        .count();
    assert!(split_diff_rows < unified_diff_rows);
    assert_eq!(app.rows[app.cursor].kind.hunk(), hunk_before);
    assert_eq!(store.load_state().unwrap().split_diff, Some(false));

    // Toggling back restores the split layout, and still as a choice.
    app.handle_key(key('s'));
    assert!(
        app.rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. }))
    );
    assert_eq!(store.load_state().unwrap().split_diff, Some(true));
}

/// The renderer's default and the config's must agree.
///
/// `ReviewOptions` deliberately takes a plain bool so the renderer never reads
/// config. That leaves two places stating one default, and nothing but this
/// test to stop them drifting apart.
#[test]
fn the_renderers_default_layout_matches_the_configs() {
    use differential_engine::config::ReviewConfig;
    assert_eq!(
        ReviewOptions::default().split_diff,
        ReviewConfig::default().diff.is_split()
    );
}

/// A recorded choice outranks the configured default, in both directions.
///
/// This is what stops a config edit moving the layout under someone who is
/// midway through a review.
#[test]
fn a_recorded_choice_wins_over_the_configured_default() {
    use differential_tui::rows::RowContent;
    for (chosen, opt_default) in [(false, true), (true, false)] {
        let (_r, mut app) = make_app_with(ReviewOptions {
            split_diff: opt_default,
            ..ReviewOptions::default()
        });
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        // Record the opposite of the default, then confirm the layout follows
        // the record rather than the option.
        app.handle_key(key('s'));
        let is_split = app
            .rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. }));
        assert_eq!(
            is_split, chosen,
            "chosen {chosen} must beat the default {opt_default}"
        );
    }
}

/// A split render carries more separators than a unified one, at any width.
///
/// This replaces a test that pressed `s` and asserted the screen contained a
/// `│` somewhere. Both halves were wrong once split became the default: `s`
/// toggled OUT of split, and `│` is drawn by the pane borders whatever the
/// layout — so it passed while rendering no split row at all.
///
/// Counting is what gives it teeth. A split row adds one separator of its own,
/// so the split render must carry strictly more than the unified one. Pinning
/// both layouts explicitly is what stops a moved default breaking it again.
#[test]
fn a_split_render_draws_more_separators_than_a_unified_one() {
    use differential_tui::rows::RowContent;

    /// The most separators any single screen row carries.
    ///
    /// Per row rather than per screen: the totals differ by only two, because
    /// the pane borders draw a hundred of them whatever the layout. A row is
    /// where the difference actually lives — a split diff row carries its own
    /// separator between the halves, on top of the borders either side.
    fn busiest_row(app: &mut App, width: u16) -> usize {
        let backend = ratatui::backend::TestBackend::new(width, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| app.draw(f)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .filter(|&x| buffer[(x, y)].symbol() == "│")
                    .count()
            })
            .max()
            .unwrap_or(0)
    }

    let (_rs, mut split) = make_app_with(laid_out(true));
    let (_ru, mut unified) = make_app_with(laid_out(false));
    // Focus the detail pane, which is what puts diff rows on screen at all.
    for app in [&mut split, &mut unified] {
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    // Both directions, so a failure names which side degenerated rather than
    // leaving the count comparison to imply it.
    assert!(
        split
            .rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. })),
        "the split app holds no split rows, so this measures nothing"
    );
    assert!(
        !unified
            .rows
            .iter()
            .any(|row| matches!(row.content, RowContent::Split { .. })),
        "the unified app holds split rows, so the two sides are not being compared"
    );

    for width in [100u16, 60] {
        let (with, without) = (
            busiest_row(&mut split, width),
            busiest_row(&mut unified, width),
        );
        assert!(
            with > without,
            "width {width}: the busiest split row carried {with} separators, \
             the busiest unified row {without}"
        );
    }
}

#[test]
fn draw_smoke_test_renders_group_label() {
    let (_r, app) = make_app();
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let content: String = buffer.content().iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Focus work"));
    assert!(content.contains("reading plan"));
    assert!(content.contains("classes reviewed"));
}

#[test]
fn reading_plan_shows_ids_and_flags_unsatisfiable_dependencies() {
    // NOT `make_app`: that fixture has no dependency edge, so the loop below
    // used to iterate nothing and the test passed on its rendering half alone.
    // The comment above `app_with_dependency_edge` already says this happened
    // once to the gutter test; it had happened here too.
    let (_r, app) = app_with_dependency_edge();

    // Every dependency names a real group id — the id column makes them
    // resolvable, which is the whole point of showing it.
    let ids: Vec<&str> = app.groups().iter().map(|g| g.id.as_str()).collect();
    let edges: usize = app.groups().iter().map(|g| g.depends_on.len()).sum();
    assert!(
        edges > 0,
        "the fixture must carry an edge, or this test asserts nothing"
    );
    for g in app.groups() {
        for d in &g.depends_on {
            assert!(
                ids.contains(&d.id.as_str()),
                "dependency {:?} is not a group id",
                d.id
            );
            // The flag must agree with the plan order: it means "this
            // dependency appears further down", i.e. a cycle the toposort
            // had to break.
            let dep_pos = app.groups().iter().position(|o| o.id == d.id).unwrap();
            let self_pos = app.groups().iter().position(|o| o.id == g.id).unwrap();
            assert_eq!(
                d.unsatisfied,
                dep_pos > self_pos,
                "cycle flag disagrees with the order"
            );
        }
    }

    // The pane renders the id, the tier, and the counts.
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    // The PLAN pane only. `content` used to be the whole 100x40 screen, and
    // the detail pane draws both "files" (its box title) and "−" (its hunk
    // header counts) — so those two assertions held with the plan pane's own
    // counts deleted.
    let plan: String = (1..39u16)
        .flat_map(|y| (1..39u16).map(move |x| (x, y)))
        .map(|(x, y)| buffer[(x, y)].symbol())
        .collect();
    assert!(
        plan.contains(&app.groups()[0].id),
        "group id missing from the plan: {plan}"
    );
    // The group's OWN numbers, not just the words around them. A bare
    // `contains("files")` matched the detail pane's box title, and a bare
    // `contains("−")` survived the `+` span being deleted.
    let g = &app.groups()[0];
    assert!(
        plan.contains(&format!("{} files", g.n_files)),
        "per-group file count missing: {plan}"
    );
    assert!(
        plan.contains(&format!("+{}", g.counts.adds)),
        "added-line count missing: {plan}"
    );
    assert!(
        plan.contains(&format!("−{}", g.counts.dels)),
        "removed-line count missing: {plan}"
    );
}

#[test]
fn space_in_the_plan_pane_marks_the_whole_group() {
    let (_r, mut app) = make_app();
    assert_eq!(app.focus, Focus::Groups);
    // Pick the group with the most hunks so "whole group" is meaningful.
    let target = app
        .groups()
        .iter()
        .enumerate()
        .max_by_key(|(_, g)| g.hunks.len())
        .map(|(i, _)| i)
        .unwrap();
    while app.selected_group != target {
        app.handle_key(key('j'));
    }
    let want = app.groups()[target].hunks.len();
    assert!(want > 1);

    app.handle_key(key(' '));
    assert_eq!(
        app.session.reviewed_count(),
        want,
        "whole group should be marked"
    );
    // Pressing again clears the whole group (set semantics, not per-hunk flip).
    app.handle_key(key(' '));
    assert_eq!(app.session.reviewed_count(), 0);

    // In the diff pane, space marks the hunk under the cursor, not its group.
    // Read that in the one-hunk group: the many-hunk group is three hunks of
    // one shape, and a mark keys on content, so there the two are one answer.
    let single = app
        .groups()
        .iter()
        .position(|g| g.hunks.len() == 1)
        .expect("the fixture has a one-hunk group");
    while app.selected_group != single {
        app.handle_key(key(if app.selected_group < single {
            'j'
        } else {
            'k'
        }));
    }
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key(' '));
    assert_eq!(app.session.reviewed_count(), 1);
}

#[test]
fn n_and_shift_n_jump_between_hunks() {
    let (_r, mut app) = make_app();
    // Move to the skim group (3 hunks of one shape) and unfold its remainder,
    // so the view holds several hunks to jump between.
    app.handle_key(key('j'));
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('g'));
    let hunk_rows: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::HunkHeader { .. }))
        .map(|(i, _)| i)
        .collect();
    assert!(hunk_rows.len() >= 2, "fixture needs multiple hunks");

    app.handle_key(key('n'));
    assert!(
        hunk_rows.contains(&app.cursor),
        "n should land on a hunk header"
    );
    let first = app.cursor;
    app.handle_key(key('n'));
    assert!(app.cursor > first);
    app.handle_key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
    assert_eq!(app.cursor, first, "N goes back");
}

/// The flat case: one directory, four files, nothing nested. A fold that
/// reaches a SUBDIRECTORY needs a fixture this one cannot carry, which is why
/// `folding_a_directory_folds_its_subdirectories_too` exists beside it.
#[test]
fn file_view_is_a_collapsible_tree() {
    use differential_tui::app::{TreeKind, ViewMode};
    let (_r, mut app) = make_app();
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Files);

    // The fixture's files all live under src/, so the tree has a src/ node
    // above them — directories are rows, files nest beneath.
    let dir_row = app
        .tree
        .iter()
        .position(|e| matches!(&e.kind, TreeKind::Dir { path, .. } if path == "src"))
        .expect("src/ directory row");
    let files_visible = |a: &differential_tui::app::App| {
        a.tree
            .iter()
            .filter(|e| matches!(e.kind, TreeKind::File { .. }))
            .count()
    };
    assert_eq!(files_visible(&app), 4, "all files visible when expanded");

    // Selecting the directory shows every hunk beneath it.
    while app.selected_file != dir_row {
        app.handle_key(key('j'));
    }
    let hunks_under_dir = app
        .rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .count();
    assert!(hunks_under_dir >= 4, "directory view spans its files");

    // z collapses it: the files disappear, the directory row stays.
    app.handle_key(key('z'));
    assert_eq!(
        files_visible(&app),
        0,
        "collapsed directory hides its files"
    );
    assert!(
        app.tree
            .iter()
            .any(|e| matches!(&e.kind, TreeKind::Dir { path, .. } if path == "src"))
    );
    app.handle_key(key('z'));
    assert_eq!(files_visible(&app), 4, "unfold restores them");
}

/// The row a tree entry stands for, as a test can read it.
fn tree_paths(app: &App) -> Vec<String> {
    use differential_tui::app::TreeKind;
    app.tree
        .iter()
        .map(|e| match &e.kind {
            TreeKind::Dir { path, .. } => format!("{path}/"),
            TreeKind::File { file_idx } => app.files()[*file_idx].path.clone(),
        })
        .collect()
}

/// Put the tree cursor on the row for `path` and fold it.
fn fold(app: &mut App, path: &str) {
    let want = format!("{path}/");
    app.focus = Focus::Groups;
    app.selected_file = tree_paths(app)
        .iter()
        .position(|p| *p == want)
        .unwrap_or_else(|| panic!("no row for {path}: {:#?}", tree_paths(app)));
    app.handle_key(key('z'));
}

/// A fold reaches the WHOLE subtree. It used to reach the files and the first
/// subdirectory branch only: the "is an ancestor folded" answer was recomputed
/// per file from the directories that file opened, so a directory carried over
/// from the previous file was skipped — and its siblings drew childless rows
/// dangling under a `▸`.
#[test]
fn folding_a_directory_folds_its_subdirectories_too() {
    let (_r, mut app) = app_with_nested_dirs();
    switch_left_pane(&mut app);
    let before = tree_paths(&app);
    assert!(
        before.contains(&"src/a/".to_string()) && before.contains(&"src/b/".to_string()),
        "the fixture needs two sibling subdirectories: {before:#?}"
    );

    fold(&mut app, "src");
    let after = tree_paths(&app);
    let under_src: Vec<&String> = after
        .iter()
        .filter(|p| p.starts_with("src/") && *p != "src/")
        .collect();
    assert!(
        under_src.is_empty(),
        "a folded directory keeps only its own row: {after:#?}"
    );
    assert!(after.contains(&"src/".to_string()), "{after:#?}");
    assert!(
        after.contains(&"docs/".to_string()) && after.contains(&"docs/readme.txt".to_string()),
        "the fold stops at its own subtree: {after:#?}"
    );

    fold(&mut app, "src");
    assert_eq!(tree_paths(&app), before, "unfold gives back the same rows");
}

/// The arm is a row's place in the tree, so it cannot come and go with a fold.
/// The last TOP-LEVEL row used to lose its `└─` — the guard that spares a
/// one-row tree an arm to nothing also caught it — and folding a directory at
/// the foot of the tree is the quickest way to make one. It then sat two
/// columns left of the siblings it belongs beside.
#[test]
fn the_last_top_level_row_keeps_its_arm() {
    let (_r, mut app) = app_with_nested_dirs();
    switch_left_pane(&mut app);
    // The left pane only: the diff's file header names a path too.
    let armed = |a: &App| {
        screen(a, 100, 24)
            .iter()
            .map(|r| r.chars().take(40).collect::<String>())
            .find(|r| r.contains("src/"))
            .unwrap_or_else(|| panic!("no src/ row"))
    };
    assert!(armed(&app).contains("└─▾ src/"), "{}", armed(&app));

    // `src/` is now the tree's last row, and still its second root.
    fold(&mut app, "src");
    assert!(
        armed(&app).contains("└─▸ src/"),
        "a fold must not cost a row its arm: {}",
        armed(&app)
    );
}

/// A fold reaches DOWN, not sideways.
#[test]
fn folding_an_inner_directory_leaves_its_siblings() {
    let (_r, mut app) = app_with_nested_dirs();
    switch_left_pane(&mut app);

    fold(&mut app, "src/a");
    let after = tree_paths(&app);
    assert!(after.contains(&"src/a/".to_string()), "{after:#?}");
    let under_a: Vec<&String> = after
        .iter()
        .filter(|p| p.starts_with("src/a/") && *p != "src/a/")
        .collect();
    assert!(
        under_a.is_empty(),
        "its files AND its own subdirectories go: {after:#?}"
    );
    assert!(
        after.contains(&"src/b/".to_string()) && after.contains(&"src/b/z.txt".to_string()),
        "its sibling stays: {after:#?}"
    );
    assert!(
        after.contains(&"src/main.txt".to_string()),
        "so does the file beside it: {after:#?}"
    );
}

/// A repo whose two groups have a real symbol def -> use edge between them, so
/// the ordering stage fills `depends_on` and the gutter has something to draw.
///
/// `make_app`'s fixture has no such edge, which is why the gutter test used to
/// pass while asserting nothing. Mirrors the engine's `def_use_repo`.
fn app_with_dependency_edge() -> (TestRepo, App) {
    let r = TestRepo::new();
    r.write("src/a_core.txt", b"placeholder\n");
    r.write("src/b_user.txt", b"placeholder\n");
    r.commit_all("base");
    r.write(
        "src/a_core.txt",
        b"placeholder\npub struct WidgetCore { pub retries: u32 }\n",
    );
    r.write(
        "src/b_user.txt",
        b"placeholder\nlet core = WidgetCore { retries: 3 };\n",
    );
    r.commit_all("head");

    // One focus group per class, so each stays its own node in the dependency graph.
    let backend = one_group_per_class();
    let app = open_app_with(&r, &backend, ".dfr-edge-store");
    (r, app)
}

#[test]
fn the_plan_gutter_links_the_selected_group_to_what_it_follows() {
    use differential_tui::app::Relation;
    let (_r, mut app) = app_with_dependency_edge();

    // `expect`, not an early return: the previous version skipped silently
    // when the fixture had no edges — which is what it did, so it passed while
    // asserting nothing at all.
    let consumer = app
        .groups()
        .iter()
        .position(|g| !g.depends_on.is_empty())
        .expect("the fixture must produce a dependency edge, or this asserts nothing");
    let follows: Vec<String> = app.groups()[consumer]
        .depends_on
        .iter()
        .map(|d| d.id.clone())
        .collect();
    let foundation = app
        .groups()
        .iter()
        .position(|g| follows.contains(&g.id))
        .expect("the fixture must have a group the consumer follows");

    // j moves down, k up — a one-directional walk would spin forever when the
    // target is above the cursor, and the foundation sits above its consumer.
    let select = |app: &mut App, want: usize| {
        app.focus = Focus::Groups;
        for _ in 0..64 {
            if app.selected_group == want {
                return;
            }
            app.handle_key(key(if app.selected_group < want { 'j' } else { 'k' }));
        }
        panic!("could not reach group {want}");
    };

    // --- selecting the consumer: what it follows is marked ------------------
    select(&mut app, consumer);
    assert_eq!(app.relation_to_selected(consumer), Relation::Selected);

    // Biconditional, so a relation that wrongly returned None fails too. The
    // previous version only checked that a claimed edge was backed by
    // depends_on, never that a real edge produced a mark.
    assert!(follows.contains(&app.groups()[foundation].id));
    for (i, g) in app
        .groups()
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != consumer)
    {
        let marked = app.relation_to_selected(i) == Relation::Dependency;
        assert_eq!(
            marked,
            follows.contains(&g.id),
            "{} marked={marked} but the selected group follows it = {}",
            g.id,
            follows.contains(&g.id)
        );
    }

    let content = drawn(&mut app);
    assert!(content.contains("◆"), "selected group marker missing");
    assert!(
        content.contains("├"),
        "the selected group follows something, so a connector must be drawn"
    );
    // The tick wears the file tree's arm and reaches the title it points at.
    let rows = plan_rows(&mut app);
    assert!(
        rows.iter().any(|r| r.starts_with("◆─")),
        "the selected group's diamond must reach its title: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|r| r.starts_with("├─") || r.starts_with("└─")),
        "a dependency tick must reach its title: {rows:?}"
    );

    // --- selecting the foundation: the group that follows IT is not marked --
    // This is the change: the reverse edge used to be drawn, in a second
    // colour of the same glyph.
    select(&mut app, foundation);
    assert_eq!(
        app.relation_to_selected(consumer),
        Relation::None,
        "the consumer follows the selected group and must not be marked"
    );
    let content = drawn(&mut app);
    assert!(content.contains("◆"));
    assert!(
        !plan_pane(&mut app).contains("├"),
        "nothing is followed from here, so no connector should be drawn"
    );
}

/// The plan pane's rows, one string each, trimmed of the pane's own border.
/// Row order, unlike `plan_pane` — a connector's arm is two adjacent cells on
/// one row, and a column-major dump puts a pane's height between them.
fn plan_rows(app: &mut App) -> Vec<String> {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (1..39u16)
        .map(|y| (1..39u16).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// Just the plan pane's columns. `├` is also a hunk box's corner over in the
/// diff pane, so an assertion about the connector has to say where it looks.
fn plan_pane(app: &mut App) -> String {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..40u16)
        .flat_map(|x| (0..40u16).map(move |y| (x, y)))
        .map(|(x, y)| buf[(x, y)].symbol().to_string())
        .collect()
}

/// Render the DETAIL pane at a fixed size and flatten the buffer to text.
///
/// Focuses it first: the right pane is a map of the selected group while the
/// plan has focus, so a test asserting on diff content has to say it wants the
/// diff. `drawn_as_is` is for the tests that are about focus itself.
fn drawn(app: &mut App) -> String {
    app.focus = Focus::Detail;
    drawn_as_is(app)
}

/// The whole screen as rows, in row order — for assertions about text that
/// has to sit on one line.
fn drawn_rows(app: &mut App) -> Vec<String> {
    screen(app, 100, 40)
}

/// The screen at `w` by `h`, one string per row.
fn screen(app: &App, w: u16, h: u16) -> Vec<String> {
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

/// Render whatever the current focus puts on screen.
fn drawn_as_is(app: &mut App) -> String {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn the_selected_plan_row_is_highlighted_edge_to_edge() {
    let (_r, app) = make_app();
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();

    // The plan pane is the left 40 columns; its border is column 0 and the
    // last inner column is 38. Find the selected row by its background, then
    // assert that background runs to the pane edge rather than stopping at
    // the end of the label.
    // `draw` paints the theme's ground over every cell first, so "has a
    // background" and "two neighbours agree" are true of EVERY row. This used
    // to find its row on those two tests and then compare ground with ground,
    // which held with the highlight deleted. The selection's own colour is the
    // only thing that identifies its row.
    let bg_of = |x: u16, y: u16| buf[(x, y)].style().bg;
    let selected = Some(theme().selected_bg);
    let selected_row = (1..39u16)
        .find(|&y| bg_of(38, y) == selected)
        .expect("a row wearing the selection colour");

    // Column 2 is the connector, which keeps its own styling; the highlight
    // starts at the label and runs UNBROKEN to the last inner column.
    let lit: Vec<u16> = (3..=38u16)
        .filter(|&x| bg_of(x, selected_row) == selected)
        .collect();
    assert_eq!(
        lit,
        (3..=38u16).collect::<Vec<_>>(),
        "the selection must run to the pane edge without a gap"
    );
    // One row, not the pane: the row after the selected block wears ground.
    assert_ne!(
        bg_of(38, selected_row + 2),
        selected,
        "only the selected group's own rows are highlighted"
    );
}

#[test]
fn scrolling_back_up_reveals_the_group_header() {
    // A file with more rows than the pane can hold: the scroll offset only
    // leaves the top when something is genuinely below the fold.
    let (_r, mut app) = app_with_a_long_file();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    // A short pane, as a small terminal would give: the rows now overflow it.
    app.set_viewport(Viewport {
        detail_rows: 8,
        detail_cols: 58,
        plan_rows: 8,
        body_rows: 8 + 2,
        ..Viewport::default()
    });

    // The header block above the first selectable row carries the label,
    // description and dependencies — the cursor can never enter it.
    let first_selectable = app
        .rows
        .iter()
        .position(|r| r.kind.selectable())
        .expect("a selectable row");
    assert!(
        first_selectable > 0,
        "fixture should have header rows on top"
    );
    assert!(matches!(app.rows[0].kind, RowKind::GroupHeader));

    // Scroll to the bottom, then all the way back up.
    app.handle_key(key('G'));
    assert!(app.scroll() > 0, "should have scrolled away from the top");
    for _ in 0..40 {
        app.handle_key(ctrl('u'));
    }
    assert_eq!(
        app.scroll(),
        0,
        "scrolling up must reach row 0, not stop below it"
    );

    // g (top) lands there too.
    app.handle_key(key('G'));
    app.handle_key(key('g'));
    assert_eq!(app.scroll(), 0);
}

/// The ref decoration runs a real `git for-each-ref` and parses its real
/// output — the hand-written bytes in the unit test happily passed while the
/// format string was wrong, so this drives the actual command.
#[test]
fn picker_reads_real_branch_and_tag_names() {
    let r = TestRepo::new();
    r.write("a.txt", b"one\n");
    let first = r.commit_all("first");
    r.git(&["tag", "v0.1.0"]);
    r.git(&["tag", "-a", "v0.2.0", "-m", "annotated"]);
    r.git(&["branch", "feature"]);
    // A remote-tracking ref, without needing a remote.
    r.git(&["update-ref", "refs/remotes/origin/main", &first]);

    let refs = differential_engine::ports::CommitHistory::refs_by_commit(&r.repo());
    let names = refs.get(&first).expect("refs for the commit");
    for want in ["main", "feature", "v0.1.0", "v0.2.0", "origin/main"] {
        assert!(
            names.iter().any(|n| n == want),
            "{want:?} missing from {names:?}"
        );
    }
}

/// The reviewer must say when a group was never classified.
///
/// The stack has always rendered the audit's back-fill as `[unclassified]`
/// (`stack.rs::backfilled_group_renders_as_unclassified`) while the TUI showed
/// it as an ordinary focus group — the same document, described two ways.
/// One projection, one answer.
#[test]
fn a_backfilled_group_renders_as_unclassified() {
    let r = TestRepo::new();
    r.write("a.txt", b"use old_name;\n");
    r.write("b.txt", b"fn main() { slow() }\n");
    r.commit_all("base");
    r.write("a.txt", b"use new_name;\n");
    r.write("b.txt", b"fn main() { fast(3) }\n");
    r.commit_all("head");

    // The model answers with only one of the two class ids; the coverage
    // audit back-fills the other into a trailing must-read group.
    let backend = FakeBackend::new("fake", |ids| {
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group("Only one", "focus", &[&ids[1]])
        )
    });
    let mut app = open_app_with(&r, &backend, ".dfr-backfill-store");

    let last = app.groups().last().expect("at least one group");
    assert!(
        last.unclassified,
        "the trailing group is the audit back-fill"
    );
    assert!(
        app.groups()[..app.groups().len() - 1]
            .iter()
            .all(|g| !g.unclassified),
        "only the back-fill is unclassified"
    );

    // And it is visible: the header says so rather than reading as focus work.
    while app.selected_group != app.groups().len() - 1 {
        app.handle_key(key('j'));
    }
    // The group's header row is in the detail pane, which shows a map of the
    // group while the plan has focus.
    app.focus = Focus::Detail;
    let text = drawn_as_is(&mut app);
    assert!(
        text.contains("unclassified"),
        "the back-fill group must be labelled, not shown as ordinary focus work"
    );
}

/// Geometry is state, not a draw-time discovery.
///
/// Before this, scroll math used `viewport_hint.max(8)` — a guess that `draw`
/// corrected one frame later — so a shrunk window kept a stale scroll offset
/// until something else happened to repaint. The clamp now runs in update,
/// with no key pressed and nothing drawn.
#[test]
fn shrinking_the_viewport_re_clamps_scroll_without_a_draw() {
    let (_r, mut app) = app_with_a_long_file();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    // Tall enough that the whole diff fits, so nothing has scrolled yet.
    let tall = app.rows.len() + SCROLL_MARGIN + 2;
    app.set_viewport(Viewport {
        detail_rows: tall,
        detail_cols: 58,
        plan_rows: tall,
        body_rows: tall + 2,
        ..Viewport::default()
    });
    app.handle_key(key('G'));
    assert_eq!(app.scroll(), 0, "everything fits, so nothing scrolled");

    // Now shrink — above MIN_VIEWPORT, so the floor cannot mask it. No key is
    // pressed and nothing is drawn between here and the assertion.
    app.set_viewport(Viewport {
        detail_rows: SHORT,
        detail_cols: 58,
        plan_rows: SHORT,
        body_rows: SHORT + 2,
        ..Viewport::default()
    });
    assert!(
        app.scroll() > 0,
        "a shrunk viewport must scroll the cursor back into view immediately, \
         not on the next repaint"
    );
    assert!(
        app.cursor >= app.scroll() && app.cursor < app.scroll() + SHORT,
        "cursor {} outside the visible rows {}..{}",
        app.cursor,
        app.scroll(),
        app.scroll() + SHORT
    );
}

/// A pane height above the app's `MIN_VIEWPORT` floor, so the clamp cannot
/// mask the shrink.
const SHORT: usize = 9;
/// Mirrors the app's own scroll margin.
const SCROLL_MARGIN: usize = 3;

// ------------------------------------------------- context expansion (ADR 0021)

/// A repo with one long file changed in two places, so a hunk has plenty of
/// context above and below it and the two windows start apart.
fn app_with_a_long_file() -> (TestRepo, App) {
    let r = TestRepo::new();
    let body = |a: &str, b: &str| -> Vec<u8> {
        let mut out = String::new();
        for i in 1..=60 {
            match i {
                20 => out.push_str(a),
                40 => out.push_str(b),
                _ => out.push_str(&format!("let filler{i} = {i};\n")),
            }
        }
        out.into_bytes()
    };
    r.write(
        "src/long.rs",
        &body("let before = 1;\n", "let also_before = 2;\n"),
    );
    r.write("src/other.rs", b"fn untouched() {}\n");
    r.commit_all("base");
    r.write(
        "src/long.rs",
        &body("let after = 99;\n", "let also_after = 98;\n"),
    );
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids.iter().map(|i| format!("{i:?}")).collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(|s| s.trim_matches('"')).collect::<Vec<_>>()
            )
        )
    });
    // Unified: the row arithmetic these tests check counts one row per line.
    let app = open_app_with_opts(&r, &backend, ".dfr-long-store", laid_out(false));
    (r, app)
}

/// A repo whose change is one long paragraph — a prose file, the case soft
/// wrap exists for.
fn app_with_a_long_line() -> (TestRepo, App) {
    let r = TestRepo::new();
    r.write("notes.md", b"# Notes\n\nshort line\n");
    r.commit_all("base");
    r.write("notes.md", format!("# Notes\n\n{PARAGRAPH}\n").as_bytes());
    r.commit_all("head");
    // The description is long on purpose: the header that states the plan is
    // the other half of what soft wrap is for.
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{{"label": "Everything", "description": "{DESCRIPTION}", "classes": [{}], "effort": "focus", "reason": "r"}}]}}"#,
            all.iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    });
    let app = open_app_with_opts(&r, &backend, ".dfr-wrap-store", laid_out(false));
    (r, app)
}

const DESCRIPTION: &str = "This group rewrites the notes file, and the sentence \
    describing why it does so is longer than the detail pane is wide.";

const PARAGRAPH: &str = "Soft wrap exists because a paragraph is one line, and a line that runs off the right edge of the pane is a line nobody can review; the rest of it is simply not there to read.";

/// Boundary rows, and how many diff rows a view is showing.
fn edges(app: &App) -> Vec<(usize, Side)> {
    app.rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::ContextEdge { hunk, side, .. } => Some((hunk, side)),
            _ => None,
        })
        .collect()
}

fn detail_rows(app: &App) -> usize {
    app.rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Diff(_)))
        .count()
}

fn put_cursor_on<F: Fn(&RowKind) -> bool>(app: &mut App, pred: F) -> usize {
    let pos = app
        .rows
        .iter()
        .position(|r| pred(&r.kind))
        .expect("no such row");
    app.cursor = pos;
    app.focus = Focus::Detail;
    pos
}

#[test]
fn context_boundary_rows_appear_and_z_expands_there() {
    let (_r, mut app) = app_with_a_long_file();

    // Each hunk sits mid-file, so both directions have lines left over.
    let before = edges(&app);
    assert!(
        before.iter().any(|(_, s)| *s == Side::Up) && before.iter().any(|(_, s)| *s == Side::Down),
        "expected a boundary at each end, got {before:?}"
    );
    let rows_before = detail_rows(&app);
    let text = drawn(&mut app);
    assert!(
        text.contains("lines hidden"),
        "the boundary says what is hidden: {text}"
    );

    // Stand on the first upward boundary and open it.
    put_cursor_on(&mut app, |k| {
        matches!(k, RowKind::ContextEdge { side: Side::Up, .. })
    });
    app.handle_key(key('z'));

    assert_eq!(
        detail_rows(&app),
        rows_before + ReviewOptions::default().context_step,
        "z pulls in exactly one step"
    );
    // Growing upward inserts rows above, so the cursor has to have followed
    // its boundary row rather than kept its index.
    assert!(matches!(
        app.rows[app.cursor].kind,
        RowKind::ContextEdge { side: Side::Up, .. }
    ));
}

#[test]
fn expanding_to_the_edge_of_the_gap_drops_the_boundary() {
    let (_r, mut app) = app_with_a_long_file();
    let hunk = match edges(&app).first() {
        Some((h, _)) => *h,
        None => panic!("no boundary to expand"),
    };

    // Nineteen lines precede the first change; keep pressing z until they are
    // all on screen and the boundary has nothing left to offer.
    for _ in 0..10 {
        if !edges(&app)
            .iter()
            .any(|(h, s)| *h == hunk && *s == Side::Up)
        {
            break;
        }
        put_cursor_on(
            &mut app,
            |k| matches!(*k, RowKind::ContextEdge { hunk: h, side: Side::Up, .. } if h == hunk),
        );
        app.handle_key(key('z'));
    }

    assert!(
        !edges(&app)
            .iter()
            .any(|(h, s)| *h == hunk && *s == Side::Up),
        "the boundary should be gone once the whole gap is shown"
    );
    assert!(
        app.status.contains("top of"),
        "the reviewer should be told why it vanished, got {:?}",
        app.status
    );
    // The cursor landed somewhere real, and line 1 of the file is now drawn.
    assert!(app.rows[app.cursor].kind.selectable());
    assert!(drawn(&mut app).contains("let filler1 ="));
}

#[test]
fn expanded_windows_that_meet_merge_into_one_block() {
    let (_r, mut app) = app_with_a_long_file();
    // Two hunks nineteen lines apart, each already showing three: expanding
    // the lower one's window upward closes the gap.
    let lower = edges(&app)
        .iter()
        .filter(|(_, s)| *s == Side::Up)
        .map(|(h, _)| *h)
        .next_back()
        .expect("a second hunk");

    // Press on whichever end of the middle gap is still offered. Once one press
    // would close it the two rows collapse to one, so "the upward one is gone"
    // is not the same as "the gap is closed".
    for _ in 0..8 {
        let mid = edges(&app)
            .into_iter()
            .find(|&(h, s)| (h == lower && s == Side::Up) || (h != lower && s == Side::Down));
        let Some((h, side)) = mid else { break };
        put_cursor_on(
            &mut app,
            |k| matches!(*k, RowKind::ContextEdge { hunk: x, side: sd, .. } if x == h && sd == side),
        );
        app.handle_key(key('z'));
    }

    // One block: one boundary at each outer end, and no line drawn twice.
    let e = edges(&app);
    assert_eq!(
        e.iter().filter(|(_, s)| *s == Side::Up).count(),
        1,
        "merged windows share one top boundary, got {e:?}"
    );
    assert_eq!(
        e.iter().filter(|(_, s)| *s == Side::Down).count(),
        1,
        "and one bottom, got {e:?}"
    );
    let text = drawn(&mut app);
    assert_eq!(
        text.matches("let filler25 =").count(),
        1,
        "a merged block must not repeat a line"
    );
    // Both hunk headers are still there, so n/N and findings still work.
    assert_eq!(
        app.rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
            .count(),
        2
    );
}

/// The property the whole windowed rebuild buys: syntect's cost tracks what is
/// drawn, not the size of the files touched.
#[test]
fn highlighting_is_windowed_not_whole_file() {
    let r = TestRepo::new();
    let big = |changed: &str| -> Vec<u8> {
        let mut out = String::new();
        for i in 1..=5_000 {
            if i == 4_900 {
                out.push_str(changed);
            } else {
                out.push_str(&format!("let filler{i} = {i};\n"));
            }
        }
        out.into_bytes()
    };
    r.write("src/big.rs", &big("let before = 1;\n"));
    r.commit_all("base");
    r.write("src/big.rs", &big("let after = 2;\n"));
    r.commit_all("head");

    let repo = Repo::open(Path::new(&r.root)).unwrap();
    let base = r.git(&["rev-parse", "HEAD~1"]);
    let head = r.git(&["rev-parse", "HEAD"]);
    let backend = skim_first_backend();
    let out = differential_testutil::grouped_output(&r, &base, &head, &backend, None);
    let factory = RowFactory::new(repo, out.base.clone(), out.head.clone());
    let session = ReviewSession::open(
        FsReviewStore::at(r.root.join(".dfr-big-store")).unwrap(),
        out.document.unwrap(),
        out.view,
    )
    .unwrap();
    let app = App::new(session, factory, ReviewOptions::default(), theme().clone());

    // One hunk at line 4,900 of a 5,000-line file. Two sides, each a window of
    // a few lines plus a bounded lookback — nowhere near the 10,000 lines the
    // whole-file pass used to parse.
    let scanned = app.highlighted_lines();
    assert!(
        scanned > 0,
        "the window really was highlighted, not skipped"
    );
    assert!(
        scanned < 400,
        "highlighting should track the window, not the file: scanned {scanned} lines"
    );
}

// ------------------------------------------------------- the lumen styling

/// The cell styles of one drawn row of the diff pane, left to right.
fn diff_pane_row(app: &App, y: u16) -> Vec<(String, Option<ratatui::style::Color>)> {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    // The diff pane starts at column 40; its border is 40 and 99, so a row's
    // content is 41..=98. A hunk's box shares those border columns rather than
    // spending any of the content.
    (41..99u16)
        .map(|x| (buf[(x, y)].symbol().to_string(), buf[(x, y)].style().bg))
        .collect()
}

/// An unstyled ratatui cell reports `Color::Reset`, which is not a colour
/// anyone painted — only an explicit RGB counts as a background here.
/// A background of the row's OWN, as opposed to the theme's ground.
///
/// The ground is painted under every cell now that a palette owns its
/// background, so "has an Rgb bg" stopped meaning anything — every cell does.
/// What these tests are asking is whether a row carries a tint the ground did
/// not give it.
fn painted(bg: Option<ratatui::style::Color>) -> Option<ratatui::style::Color> {
    match bg {
        Some(ratatui::style::Color::Rgb(..)) if bg != Some(theme().bg) => bg,
        _ => None,
    }
}

#[test]
fn a_changed_row_paints_its_background_to_the_pane_edge() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    let app = app;
    // A changed row is one whose gutter block is painted; the last inner
    // column of the pane must then carry the line's own background too.
    let row = (1..39u16)
        .find(|&y| {
            let cells = diff_pane_row(&app, y);
            // A boundary band paints every cell one colour; a changed row's
            // gutter block and line tint differ, which is the point.
            painted(cells[1].1).is_some()
                && painted(cells.last().unwrap().1).is_some()
                && painted(cells[1].1) != painted(cells.last().unwrap().1)
        })
        .expect("no changed row found in the diff pane");
    let cells = diff_pane_row(&app, row);
    assert_eq!(
        painted(cells.last().unwrap().1),
        painted(cells[cells.len() - 20].1),
        "the background stops before the pane edge"
    );
    // The gutter block is a DIFFERENT colour from the line body, which is what
    // makes it read as an edge.
    assert_ne!(
        painted(cells[1].1),
        painted(cells.last().unwrap().1),
        "the gutter should be a stronger block than the line tint"
    );
    // A context row, by contrast, is painted nowhere.
    let context = (1..39u16)
        .find(|&y| {
            let cells = diff_pane_row(&app, y);
            let text: String = cells.iter().map(|(sym, _)| sym.as_str()).collect();
            text.contains("filler") && painted(cells[1].1).is_none()
        })
        .expect("no context row found");
    assert!(
        diff_pane_row(&app, context)
            .iter()
            .all(|(_, bg)| painted(*bg).is_none()),
        "unchanged context should carry no background at all"
    );
}

#[test]
fn colour_carries_the_change_so_there_are_no_marker_columns() {
    let (_r, mut app) = app_with_a_long_file();
    let text = drawn(&mut app);
    assert!(
        text.contains("let after = 99;"),
        "the changed line is drawn"
    );
    // The old `-`/`+` gutter put a marker between the numbers and the code.
    assert!(
        !text.contains("99 + let") && !text.contains(" - let"),
        "marker columns should be gone"
    );
}

#[test]
fn the_absent_side_of_a_split_row_is_hatched() {
    let r = TestRepo::new();
    r.write("src/a.rs", b"let keep = 1;\nlet gone = 2;\nlet tail = 3;\n");
    r.commit_all("base");
    // A pure deletion: the new side has no line for it at all.
    r.write("src/a.rs", b"let keep = 1;\nlet tail = 3;\n");
    r.commit_all("head");
    let backend = skim_first_backend();
    let mut app = open_app_with_opts(&r, &backend, ".dfr-hatch-store", laid_out(true));
    let text = drawn(&mut app);
    assert!(
        text.contains('╱'),
        "a side with no line should be hatched, not left blank"
    );
    assert!(
        text.contains("old") && text.contains("new"),
        "column labels"
    );
}

/// Background colours of the cells on one drawn row, left to right.
fn row_backgrounds(app: &mut App, y: u16) -> Vec<ratatui::style::Color> {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    (0..100u16).map(|x| buf[(x, y)].bg).collect()
}

/// The brighter line-number block the cursor's row wears on a changed line.
fn is_cursor_block(c: &ratatui::style::Color) -> bool {
    *c == theme().added_gutter_cursor_bg || *c == theme().deleted_gutter_cursor_bg
}

/// The screen row the cursor is drawn on, given the pane's scroll.
fn cursor_screen_row(app: &App) -> u16 {
    (app.cursor - app.scroll()) as u16 + 1
}

/// A changed line's every span carries `added_bg`/`deleted_bg`, and a span
/// background beats a line style. The selection used to BE a line style, so
/// selecting the lines inside a hunk — which is what selecting means here —
/// changed nothing on screen. Nothing guarded it.
#[test]
fn a_selection_shows_on_a_changed_line() {
    let (_r, mut app) = app_with_a_long_file();
    let changed = |c: &ratatui::style::Color| *c == theme().added_bg || *c == theme().deleted_bg;
    let selected =
        |c: &ratatui::style::Color| *c == theme().added_sel_bg || *c == theme().deleted_sel_bg;

    // The cursor's own row no longer keeps the idle tint, so the walk looks
    // for the gutter block instead: that is what says the cursor is standing
    // on a changed line.
    put_cursor_on(&mut app, |k| matches!(k, RowKind::Diff(_)));
    for _ in 0..30 {
        let y = cursor_screen_row(&app);
        if !row_backgrounds(&mut app, y).iter().any(is_cursor_block) {
            app.handle_key(key('j'));
            continue;
        }
        // when - the anchor is this row and the cursor moves off it, so what
        // shows here is the selection's rung rather than the cursor's.
        app.handle_key(key('v'));
        app.handle_key(key('j'));
        let anchor = app.visual.expect("a selection is open");
        let at = (anchor - app.scroll()) as u16 + 1;
        let after = row_backgrounds(&mut app, at);

        // then - the row's own colour has stepped, edge to edge.
        assert!(
            after.iter().any(selected),
            "a selected changed row must wear its selected tint: {after:?}"
        );
        assert!(
            !after.iter().any(changed),
            "no cell may keep the unselected tint: {after:?}"
        );
        return;
    }
    panic!("no changed row within reach of the cursor");
}

/// The cursor had the same defect and one symptom worse. On a split row the
/// absent half is hatched and has no colour to defend, so it took the cursor's
/// line style while the half carrying the change kept its idle green — a
/// cursor on the side with no line.
#[test]
fn the_cursor_lights_a_changed_row_edge_to_edge() {
    let (_r, mut app) = app_with_a_long_file();
    let changed = |c: &ratatui::style::Color| *c == theme().added_bg || *c == theme().deleted_bg;
    let under_cursor = |c: &ratatui::style::Color| {
        *c == theme().added_cursor_bg || *c == theme().deleted_cursor_bg
    };

    put_cursor_on(&mut app, |k| matches!(k, RowKind::Diff(_)));
    for _ in 0..30 {
        let y = cursor_screen_row(&app);
        let row = row_backgrounds(&mut app, y);
        if !row.iter().any(is_cursor_block) {
            app.handle_key(key('j'));
            continue;
        }
        assert!(
            row.iter().any(under_cursor),
            "the cursor's row must step its change colour: {row:?}"
        );
        assert!(
            !row.iter().any(changed),
            "no cell may keep the idle tint: {row:?}"
        );
        return;
    }
    panic!("no changed row within reach of the cursor");
}

/// The line-style path still does its job. A context line has no colour of its
/// own, so `selected_bg` is the only thing that can mark it — and the fix must
/// not take that away.
#[test]
fn a_selection_still_shows_on_a_context_line() {
    let (_r, mut app) = app_with_a_long_file();
    let changed = |c: &ratatui::style::Color| *c == theme().added_bg || *c == theme().deleted_bg;

    put_cursor_on(&mut app, |k| matches!(k, RowKind::Diff(_)));
    for _ in 0..30 {
        let y = cursor_screen_row(&app);
        if row_backgrounds(&mut app, y).iter().any(changed) {
            app.handle_key(key('j'));
            continue;
        }
        // when - the anchor is this row and the cursor moves off it, so the
        // anchor takes the selection's colour rather than the cursor's.
        app.handle_key(key('v'));
        app.handle_key(key('j'));
        let anchor = app.visual.expect("a selection is open");
        let y = (anchor - app.scroll()) as u16 + 1;

        // then
        assert!(
            row_backgrounds(&mut app, y).contains(&theme().selected_bg),
            "a selected context row must wear the selection tint"
        );
        return;
    }
    panic!("no context row within reach of the cursor");
}

#[test]
fn the_cursor_lights_the_gutter_on_both_sides_of_a_split_row() {
    let (_r, mut app) = app_with_a_long_file();
    app.handle_key(key('s')); // that helper opens unified, so switch to split
    put_cursor_on(&mut app, |k| matches!(k, RowKind::Diff(_)));
    // A modification exists on both sides, so both gutters must light.
    for _ in 0..30 {
        let y = cursor_screen_row(&app);
        let bgs = row_backgrounds(&mut app, y);
        // The separator column splits the row into its two halves.
        if bgs[..50].iter().any(is_cursor_block) && bgs[50..].iter().any(is_cursor_block) {
            return;
        }
        app.handle_key(key('j'));
    }
    panic!("both halves of a modified split row must carry the cursor block");
}

#[test]
fn the_cursor_lights_the_absent_side_of_a_split_row_too() {
    let r = TestRepo::new();
    r.write("src/a.rs", b"let keep = 1;\nlet tail = 3;\n");
    r.commit_all("base");
    // A pure insertion: the OLD side has no line, so it is hatched.
    r.write(
        "src/a.rs",
        b"let keep = 1;\nlet fresh = 2;\nlet tail = 3;\n",
    );
    r.commit_all("head");
    let backend = skim_first_backend();
    let mut app = open_app_with_opts(&r, &backend, ".dfr-cursor-hatch-store", laid_out(true));
    put_cursor_on(&mut app, |k| matches!(k, RowKind::Diff(_)));
    for _ in 0..20 {
        let y = cursor_screen_row(&app);
        let bgs = row_backgrounds(&mut app, y);
        // The hatched half keeps a blank gutter of the same width, so the
        // cursor block lands in the same column on both sides.
        if bgs[..50].contains(&theme().cursor_bg) {
            return;
        }
        app.handle_key(key('j'));
    }
    panic!("an absent side must still carry the cursor's gutter block");
}

#[test]
fn a_context_boundary_row_is_not_a_hunk_for_marking_or_findings() {
    let (_r, mut app) = app_with_a_long_file();
    put_cursor_on(&mut app, |k| matches!(k, RowKind::ContextEdge { .. }));
    app.handle_key(key('c'));
    assert!(
        matches!(app.mode, Mode::Normal),
        "c on a boundary row should not open the finding editor"
    );
    assert!(app.status.contains("move onto a hunk"), "{}", app.status);
}

#[test]
fn space_on_context_marks_the_hunk_that_context_belongs_to() {
    let (_r, mut app) = app_with_a_long_file();
    // Merge the two windows so one block spans both hunks, then check that a
    // context row acts on the hunk it is next to rather than on the block's
    // first one.
    let lower = edges(&app)
        .iter()
        .filter(|(_, s)| *s == Side::Up)
        .map(|(h, _)| *h)
        .next_back()
        .expect("a second hunk");
    for _ in 0..4 {
        if !edges(&app)
            .iter()
            .any(|(h, s)| *h == lower && *s == Side::Up)
        {
            break;
        }
        put_cursor_on(
            &mut app,
            |k| matches!(*k, RowKind::ContextEdge { hunk: h, side: Side::Up, .. } if h == lower),
        );
        app.handle_key(key('z'));
    }

    // The last diff row of the block is trailing context below the LOWER hunk.
    let last = app
        .rows
        .iter()
        .rposition(|r| matches!(r.kind, RowKind::Diff(_)))
        .expect("a diff row");
    assert_eq!(
        app.rows[last].kind.hunk(),
        Some(lower),
        "trailing context belongs to the hunk above it"
    );
    // The first diff row is leading context, above the FIRST hunk.
    let first = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)))
        .expect("a diff row");
    assert_ne!(
        app.rows[first].kind.hunk(),
        Some(lower),
        "leading context belongs to the hunk below it"
    );
}

/// A hunk header is a band, not a `@@` line: every row already carries both
/// line numbers, so the coordinates repeated what was on screen in a notation
/// you had to decode.
#[test]
fn a_hunk_header_is_a_band_carrying_the_class_and_the_size() {
    let (_r, mut app) = app_with_a_long_file();
    // Still a selectable row, so n/N, space and c keep working on it.
    let header = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .expect("a hunk header row");
    assert!(app.rows[header].kind.selectable());
    assert!(app.rows[header].kind.hunk().is_some());

    // Idle, the header is the band and nothing else.
    let boundary = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::ContextEdge { .. }))
        .expect("a boundary row");
    app.cursor = boundary;
    app.focus = Focus::Detail;
    let idle = drawn(&mut app);
    assert!(
        !idle.contains("· C"),
        "an idle header shows no pill: {idle}"
    );

    // Move into the hunk and it says what it uniquely says: the size of the
    // change and the shape class. Each hunk here replaces one line with one.
    cursor_into_first_box(&mut app);
    let text = drawn(&mut app);
    assert!(
        !text.contains("@@"),
        "the diff-syntax coordinates should be gone"
    );
    assert!(text.contains("+1"), "the added count: {text}");
    assert!(text.contains("−1"), "the removed count");
}

/// Two boundary rows describing one gap sit adjacent with no blank between
/// them, so the seam reads as one band rather than two unrelated notices.
#[test]
fn two_boundaries_over_one_gap_are_one_band() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;

    // The two hunks are far enough apart that each block bounds the same gap.
    let at: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::ContextEdge { .. }))
        .map(|(i, _)| i)
        .collect();
    let pair = at
        .windows(2)
        .find(|w| {
            matches!(
                app.rows[w[0]].kind,
                RowKind::ContextEdge {
                    side: Side::Down,
                    ..
                }
            ) && matches!(
                app.rows[w[1]].kind,
                RowKind::ContextEdge { side: Side::Up, .. }
            )
        })
        .expect("no down/up pair over one gap");
    assert_eq!(
        pair[1],
        pair[0] + 1,
        "the two rows of a band must be adjacent, with no blank between them"
    );

    // Each keeps its own button in the pane's border column — GitHub's expander
    // is two rows styled as one, so no key has to mean two directions.
    assert_eq!(app.rows[pair[0]].button, Some("↓"));
    assert_eq!(app.rows[pair[1]].button, Some("↑"));

    let buf = buffer_of(&app);
    let row_text = |y: u16| -> String { (41..99u16).map(|x| buf[(x, y)].symbol()).collect() };
    let band = (1..39u16)
        .find(|&y| row_text(y).contains("lines hidden"))
        .expect("no band row");
    let text = row_text(band);
    // A tinted body, not a rule, and none of the notation the hunk pills lost.
    assert!(
        !text.contains('\u{2508}'),
        "the dotted stub should be gone: {text:?}"
    );
    assert!(
        !text.contains("@@"),
        "`@@` was removed from headers: {text:?}"
    );
    // One tint the whole way across. Which of the two it is depends on where
    // the cursor is standing, so the assertion is about uniformity.
    let tint = buf[(41, band)].style().bg;
    assert!(
        tint == Some(theme().hint_bg) || tint == Some(theme().hint_cursor_bg),
        "the band should wear a band colour: {tint:?}"
    );
    assert!(
        (41..99u16).all(|x| buf[(x, band)].style().bg == tint),
        "the band should be tinted the whole way across: {text:?}"
    );
}

// ------------------------------------------- crossing into another group (#21)

/// A file whose two changes land in DIFFERENT groups, which is what makes one
/// of them foreign to the other's view.
fn app_with_two_groups_in_one_file() -> (TestRepo, App) {
    let r = TestRepo::new();
    let body = |a: &str, b: &str| -> Vec<u8> {
        let mut out = String::new();
        for i in 1..=40 {
            match i {
                10 => out.push_str(a),
                22 => out.push_str(b),
                _ => out.push_str(&format!("let filler{i} = {i};\n")),
            }
        }
        out.into_bytes()
    };
    r.write("src/f.rs", &body("let one = 1;\n", "let two = 2;\n"));
    r.commit_all("base");
    r.write("src/f.rs", &body("let one = 111;\n", "let two = 222;\n"));
    r.commit_all("head");

    // One class per group, so the two hunks cannot share one.
    let backend = one_group_per_class();
    let app = open_app_with(&r, &backend, ".dfr-cross-store");
    (r, app)
}

/// Has a hunk from another group been pulled in?
fn shows_a_foreign_hunk(app: &App) -> bool {
    app.rows
        .iter()
        .any(|r| matches!(r.kind, RowKind::HunkHeader { foreign: true, .. }))
}

/// Walk the Down boundary open until it offers the hunk beyond, pressing `z`.
fn press_z_on_down_boundary(app: &mut App) -> bool {
    let Some(pos) = app.rows.iter().position(|r| {
        matches!(
            r.kind,
            RowKind::ContextEdge {
                side: Side::Down,
                ..
            }
        )
    }) else {
        return false;
    };
    app.cursor = pos;
    app.focus = Focus::Detail;
    app.handle_key(key('z'));
    true
}

#[test]
fn a_wall_is_named_rather_than_silent_and_z_crosses_it() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    assert!(
        !shows_a_foreign_hunk(&app),
        "nothing foreign is shown by default"
    );

    // Expand until the gap is spent. The boundary must NOT disappear — that is
    // the whole defect: a wall that looked like the end of the file.
    let mut crossed = false;
    for _ in 0..6 {
        let prompting = app.rows.iter().any(|r| {
            matches!(
                r.kind,
                RowKind::ContextEdge {
                    side: Side::Down,
                    crossing: true,
                    ..
                }
            )
        });
        if prompting {
            assert!(
                drawn(&mut app).contains("next:"),
                "the boundary should name what is beyond it"
            );
            press_z_on_down_boundary(&mut app);
            crossed = true;
            break;
        }
        assert!(
            press_z_on_down_boundary(&mut app),
            "boundary vanished early"
        );
    }
    assert!(crossed, "never reached the crossing prompt");
    assert!(
        shows_a_foreign_hunk(&app),
        "z on the prompt should have pulled the hunk in"
    );
}

#[test]
fn a_foreign_hunk_is_dashed_and_names_its_group() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    for _ in 0..6 {
        if shows_a_foreign_hunk(&app) {
            break;
        }
        press_z_on_down_boundary(&mut app);
    }

    // The distinction lives on the model, so assert it there rather than
    // depending on both hunks happening to share a viewport.
    let styles: Vec<BoxStyle> = app
        .rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .filter_map(|r| r.border)
        .map(|b| b.box_style)
        .collect();
    assert!(
        styles.contains(&BoxStyle::Own),
        "this group's own hunk should keep a solid edge"
    );
    assert!(
        styles.contains(&BoxStyle::Foreign),
        "the crossed hunk should be edged as foreign"
    );

    // Then look at the pixels, with the foreign hunk in view and active.
    let pos = app
        .rows
        .iter()
        .position(|r| {
            r.border.is_some_and(|b| b.box_style == BoxStyle::Foreign)
                && matches!(r.kind, RowKind::Diff(_))
        })
        .expect("a foreign hunk row");
    app.cursor = pos;
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 38,
        detail_cols: 58,
        plan_rows: 38,
        body_rows: 38 + 2,
        ..Viewport::default()
    });
    let buf = buffer_of(&app);
    let dashed: Vec<u16> = (1..39u16)
        .filter(|&y| buf[(40, y)].symbol() == "\u{254e}")
        .collect();
    assert!(!dashed.is_empty(), "a foreign hunk's edge should be dashed");

    // A foreign hunk wears the same cyan the hunk you ARE reading wears, muted:
    // same family, but plainly not on this reading list.
    assert_eq!(buf[(40, dashed[0])].style().fg, Some(theme().foreign_fg));
    assert_ne!(theme().foreign_fg, theme().header_fg);

    // And it says whose it is, by id and label.
    let text = drawn(&mut app);
    let foreign = app
        .rows
        .iter()
        .find_map(|r| match r.kind {
            RowKind::HunkHeader {
                hunk,
                foreign: true,
            } => Some(hunk),
            _ => None,
        })
        .expect("a foreign hunk");
    let owner = app
        .session
        .plan()
        .group_of_hunk(differential_engine::plan::HunkId::from_index(foreign))
        .expect("the foreign hunk belongs to a group");
    // The id, not the label: the id is what the plan pane's `after:` lines are
    // keyed by, and the label is a sentence.
    let (want, label) = (format!("\u{b7} {}", owner.id), owner.label.clone());
    assert!(
        text.contains(&want),
        "a foreign header must name its group by id; looked for {want:?} in:\n{text}"
    );
    let header = drawn_rows(&mut app)
        .into_iter()
        .find(|r| r.contains(&want))
        .expect("the foreign header's row");
    assert!(
        !header.contains(&label),
        "the group's label belongs in the plan pane, not on a hunk header: {header:?}"
    );
}

/// The active pill already names the owning group, right after the class. The
/// marks appended after it must not name it a second time: `· C31 · g1  g1`
/// reads as two facts about the hunk rather than one said twice.
#[test]
fn an_active_foreign_header_names_its_group_once() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    for _ in 0..6 {
        if shows_a_foreign_hunk(&app) {
            break;
        }
        press_z_on_down_boundary(&mut app);
    }

    // Put the cursor IN the foreign hunk: the pill only appears on the hunk
    // the cursor is in, and the duplication was in that pill alone.
    let pos = app
        .rows
        .iter()
        .position(|r| {
            r.border.is_some_and(|b| b.box_style == BoxStyle::Foreign)
                && matches!(r.kind, RowKind::Diff(_))
        })
        .expect("a foreign hunk row");
    app.cursor = pos;
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 38,
        detail_cols: 58,
        plan_rows: 38,
        body_rows: 38 + 2,
        ..Viewport::default()
    });

    let foreign = app
        .rows
        .iter()
        .find_map(|r| match r.kind {
            RowKind::HunkHeader {
                hunk,
                foreign: true,
            } => Some(hunk),
            _ => None,
        })
        .expect("a foreign hunk");
    let id = app
        .session
        .plan()
        .group_of_hunk(differential_engine::plan::HunkId::from_index(foreign))
        .expect("the foreign hunk belongs to a group")
        .id
        .clone();

    let header = drawn_rows(&mut app)
        .into_iter()
        .find(|r| r.contains(&format!("\u{b7} {id}")))
        .expect("the active foreign header's row");
    // Word boundaries matter: `g1` is a prefix of `g11`, so count tokens.
    let times = header
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| *t == id)
        .count();
    assert_eq!(
        times, 1,
        "the active foreign header should name its group once, not {times} times: {header:?}"
    );
}

/// A box borrows the pane's border columns rather than spending content ones,
/// so a line number inside a box sits where a line number outside one sits.
#[test]
fn a_box_costs_the_content_no_columns() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    cursor_into_first_box(&mut app);
    let buf = buffer_of(&app);
    // Content is 41..=98; the box lives in the pane's own border columns (40
    // and 99), so it costs the content nothing.
    let row_text = |y: u16| -> String { (41..99u16).map(|x| buf[(x, y)].symbol()).collect() };
    // Pick the framed row from the MODEL: the pane's own border is `│` too, so
    // a glyph test would happily match a row outside every box and compare it
    // with itself.
    let y_of = |i: usize| 1 + (i - app.scroll()) as u16;
    let framed_row = y_of(
        app.rows
            .iter()
            .position(|r| r.border.is_some() && matches!(r.kind, RowKind::Diff(_)))
            .expect("no row inside a box"),
    );
    let framed = row_text(framed_row);
    assert!(
        framed.contains("let "),
        "framed row has no code: {framed:?}"
    );
    let context = (1..39u16)
        .map(row_text)
        .find(|t| t.contains("let filler"))
        .expect("no context row");

    // Line numbers are right-aligned in a fixed field, so what must match is
    // where the CODE starts. By CHARACTER: `str::find` counts bytes.
    let code_col = |t: &str| {
        let at = t.find("let ").expect("no code on the row");
        t[..at].chars().count()
    };
    assert_eq!(
        code_col(&framed),
        code_col(&context),
        "a box must cost the content no columns:\n{framed}\n{context}"
    );

    // And the box side really is the pane's border column, not a cell inside it.
    assert_eq!(buf[(40, framed_row)].symbol(), "│");
    assert_ne!(
        buf[(41, framed_row)].symbol(),
        "│",
        "there should be no second vertical line beside the pane border"
    );
}

#[test]
fn n_skips_a_foreign_hunk_but_space_still_marks_it() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    for _ in 0..6 {
        if shows_a_foreign_hunk(&app) {
            break;
        }
        press_z_on_down_boundary(&mut app);
    }

    // n never lands on a foreign header: it is context the reviewer asked for,
    // not an entry on this group's reading list.
    app.cursor = 0;
    app.focus = Focus::Detail;
    for _ in 0..8 {
        app.handle_key(key('n'));
        assert!(
            !matches!(
                app.rows[app.cursor].kind,
                RowKind::HunkHeader { foreign: true, .. }
            ),
            "n landed on a foreign hunk header"
        );
    }

    // But space still marks it — the mark keys on class content and is shared
    // across groups, so reading it here is reading it everywhere.
    let (pos, hunk) = app
        .rows
        .iter()
        .enumerate()
        .find_map(|(i, r)| match r.kind {
            RowKind::HunkHeader {
                hunk,
                foreign: true,
            } => Some((i, hunk)),
            _ => None,
        })
        .expect("a foreign header");
    let before = app.session.reviewed_count();
    app.cursor = pos;
    app.handle_key(key(' '));
    assert_eq!(
        app.session.reviewed_count(),
        before + 1,
        "space on a foreign hunk should mark its class"
    );
    assert!(app.session.reviewed_hunks().contains(&hunk));
}

/// Put the cursor inside the first hunk's box, so that box is the active one.
fn cursor_into_first_box(app: &mut App) {
    let pos = app
        .rows
        .iter()
        .position(|r| r.border.is_some() && matches!(r.kind, RowKind::Diff(_)))
        .expect("no row inside a box");
    app.cursor = pos;
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 38,
        detail_cols: 58,
        plan_rows: 38,
        body_rows: 38 + 2,
        ..Viewport::default()
    });
}

fn buffer_of(app: &App) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    terminal.backend().buffer().clone()
}

/// A hunk is marked by an EDGE, not a box. Closing it top and bottom cut the
/// file into slabs; a vertical run down one side says where a hunk begins and
/// ends without chopping up the page.
#[test]
fn a_hunks_edge_runs_down_the_panes_own_border_column() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    cursor_into_first_box(&mut app);
    let buf = buffer_of(&app);

    let lit: Vec<u16> = (1..39u16)
        .filter(|&y| {
            buf[(40, y)].style().fg == Some(theme().header_fg)
                && buf[(40, y)].symbol() == "\u{2502}"
        })
        .collect();
    assert!(lit.len() > 1, "the edge should run, not mark a single row");
    assert!(
        lit.windows(2).all(|w| w[1] == w[0] + 1),
        "the edge should be continuous, got rows {lit:?}"
    );
    for y in &lit {
        assert_eq!(buf[(40, *y)].symbol(), "\u{2502}");
    }

    // No horizontal rule closes it, and the right-hand border is left alone.
    let text: String = lit
        .iter()
        .flat_map(|&y| (41..99u16).map(move |x| (x, y)))
        .map(|(x, y)| buf[(x, y)].symbol())
        .collect();
    assert!(
        !text.contains('\u{2500}'),
        "a hunk should not be ruled off: {text:?}"
    );
    assert_ne!(buf[(99, lit[0])].style().fg, Some(theme().skim_fg));
}

/// Only the hunk the cursor is in wears a colour. Every hunk accented at once
/// is no accent at all.
#[test]
fn only_the_active_hunks_edge_is_coloured() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    // With the cursor on a boundary row, no hunk is active and nothing is lit.
    let boundary = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::ContextEdge { .. }))
        .expect("a boundary row");
    app.cursor = boundary;
    app.focus = Focus::Detail;
    let buf = buffer_of(&app);
    assert!(
        (1..39u16).all(|y| buf[(40, y)].style().fg != Some(theme().header_fg)),
        "no hunk is under the cursor, so no edge should be lit"
    );
    // The edges are still there — muted, not missing.
    assert!(
        app.rows.iter().any(|r| r.border.is_some()),
        "a muted edge is still an edge"
    );

    // Move into a hunk and exactly that edge lights up.
    cursor_into_first_box(&mut app);
    let active = app.rows[app.cursor].border.unwrap().hunk;
    let buf = buffer_of(&app);
    let lit: Vec<u16> = (1..39u16)
        .filter(|&y| buf[(40, y)].style().fg == Some(theme().header_fg))
        .collect();
    assert!(!lit.is_empty(), "the hunk under the cursor should be lit");
    let rows_on_screen = &app.rows[app.scroll()..];
    for y in lit {
        let row = &rows_on_screen[(y - 1) as usize];
        assert_eq!(
            row.border.map(|b| b.hunk),
            Some(active),
            "a hunk other than the active one is lit at row {y}"
        );
    }
}

/// The band says the two things a reviewer can act on: how much is hidden, and
/// what stands beyond it once the gap is spent.
#[test]
fn a_band_says_what_is_hidden_and_what_is_beyond() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Detail;
    let text = drawn_as_is(&mut app);
    assert!(
        text.contains("lines hidden"),
        "a band should say how much is hidden: {text}"
    );

    // Spend the gap and the band names the hunk beyond instead.
    for _ in 0..6 {
        if app
            .rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ContextEdge { crossing: true, .. }))
        {
            break;
        }
        press_z_on_down_boundary(&mut app);
    }
    let text = drawn_as_is(&mut app);
    assert!(
        text.contains("next: C"),
        "a spent gap should name what stands beyond it: {text}"
    );

    // When the whole gap fits in one press there is no direction to choose.
    let one_press = app.rows.iter().any(|r| r.button == Some("↕"));
    assert!(
        one_press || app.rows.iter().any(|r| r.button.is_some()),
        "a band always carries a button"
    );
}

/// A hunk's pill keeps ONE palette. The cursor being in it lights the pill's
/// leading cell, in the same colour as the edge below — so the marker and the
/// run read as one thing without a block of colour the eye goes to first.
#[test]
fn the_lit_hunk_pill_is_a_leading_bar_not_a_fill() {
    let (_r, mut app) = app_with_two_groups_in_one_file();

    // The pill's rows: a tinted row carrying `·` that is not a boundary band.
    let pill_rows = |buf: &ratatui::buffer::Buffer| -> Vec<u16> {
        (1..39u16)
            .filter(|&y| {
                let t: String = (41..99u16).map(|x| buf[(x, y)].symbol()).collect();
                t.contains('·') && !t.contains("hidden") && !t.contains("next:")
            })
            .collect()
    };
    let pill_bg = |buf: &ratatui::buffer::Buffer| -> Option<Color> {
        pill_rows(buf)
            .into_iter()
            .flat_map(|y| (41..99u16).map(move |x| (x, y)))
            .find_map(|(x, y)| buf[(x, y)].style().bg.filter(|b| *b != Color::Reset))
    };

    // Nothing active: no pill at all, just the hatched band.
    let boundary = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::ContextEdge { .. }))
        .expect("a boundary row");
    app.cursor = boundary;
    app.focus = Focus::Detail;
    let buf = buffer_of(&app);
    assert_eq!(pill_bg(&buf), None, "an idle header carries no pill");

    // Cursor in the hunk: the pill appears, muted fill, with one lit cell at
    // its head in the colour the edge beside it wears.
    cursor_into_first_box(&mut app);
    let buf = buffer_of(&app);
    assert_eq!(
        pill_bg(&buf),
        Some(theme().button_bg),
        "a lit pill keeps the muted fill; only its leading cell changes"
    );
    let edge = (1..39u16)
        .find_map(|y| buf[(40, y)].style().fg.filter(|c| *c == theme().header_fg))
        .expect("no lit edge");
    let bar = pill_rows(&buf)
        .into_iter()
        .flat_map(|y| (41..99u16).map(move |x| (x, y)))
        .find(|&(x, y)| {
            buf[(x, y)].symbol() == "▌" && buf[(x, y)].style().bg == Some(theme().button_bg)
        })
        .expect("no lit bar at the head of the pill");
    assert_eq!(
        buf[bar].style().fg,
        Some(edge),
        "the bar should wear the edge's colour"
    );
}

/// The counts say added and removed in one pair, everywhere. They used to need
/// a second, darker pair because a lit pill filled with the hunk's accent and
/// the bright inks vanished on it; a lit pill is one cell now, so it does not.
#[test]
fn the_counts_keep_one_pair_of_colours() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    let inks = |app: &App| -> Vec<Color> {
        let buf = buffer_of(app);
        (1..39u16)
            .filter(|&y| {
                let t: String = (41..99u16).map(|x| buf[(x, y)].symbol()).collect();
                t.contains('·') && !t.contains("hidden") && !t.contains("next:")
            })
            .flat_map(|y| (41..99u16).map(move |x| (x, y)))
            .filter(|&(x, y)| matches!(buf[(x, y)].symbol(), "+" | "−"))
            .filter_map(|(x, y)| buf[(x, y)].style().fg)
            .collect()
    };

    cursor_into_first_box(&mut app);
    let lit = inks(&app);
    assert!(lit.contains(&theme().add_fg), "no + colour: {lit:?}");
    assert!(lit.contains(&theme().del_fg), "no − colour: {lit:?}");
}

// -------------------------------------------------- the overview surfaces

/// A deep document whose selected group touches exactly one file, `src/target.rs`
/// — the one group whose map has a folded chain above it AND folded siblings
/// beside it. Returns the app parked on that group, and its drawn screen.
fn app_on_the_group_that_owns_target(store: &str) -> (TestRepo, App, Vec<String>) {
    let r = TestRepo::new();
    // Every file changes, so every one is a row in the document's tree. Only
    // ONE of them lands in the group the map is drawn for.
    let files = [
        "deep/a/b/c/buried.rs",
        "src/one.rs",
        "src/two.rs",
        "src/three.rs",
        "src/four.rs",
        "src/five.rs",
        "src/target.rs",
    ];
    // Structurally distinct, so each file lands in its own shape class and so
    // in its own group — the map then has six files it must fold.
    let shapes = [
        ("fn f() { g(); }\n", "fn f() { h(); }\n"),
        ("let x = 1;\n", "let x = 2;\n"),
        ("struct S { a: u8 }\n", "struct S { a: u16 }\n"),
        ("use a::b;\n", "use a::c;\n"),
        ("const K: u8 = 1;\n", "const K: u8 = 2;\n"),
        ("impl T for S {}\n", "impl U for S {}\n"),
        ("enum E { A, B }\n", "enum E { A, C }\n"),
    ];
    for (path, (before, _)) in files.iter().zip(shapes) {
        r.write(path, before.as_bytes());
    }
    r.commit_all("base");
    for (path, (_, after)) in files.iter().zip(shapes) {
        r.write(path, after.as_bytes());
    }
    r.commit_all("head");

    // One class per group, so the selected group touches exactly one file.
    let backend = one_group_per_class();
    let mut app = open_app_with(&r, &backend, store);
    app.focus = Focus::Groups;

    let mut rows = drawn_rows(&mut app);
    for _ in 0..files.len() {
        if rows.iter().any(|l| l.contains("● target.rs")) {
            break;
        }
        app.handle_key(key('j'));
        rows = drawn_rows(&mut app);
    }
    (r, app, rows)
}

/// The map folds on the GROUP: a directory the group never enters is one row,
/// and the files it does not touch inside one it does enter are a count. A
/// document of any size then fits the float instead of running past it.
#[test]
fn the_group_map_folds_what_the_group_does_not_touch() {
    let (_r, _app, rows) = app_on_the_group_that_owns_target(".dfr-map-fold-store");

    // The chain the group never enters is ONE row, with its path joined.
    assert!(
        rows.iter().any(|l| l.contains("▸ deep/a/b/c/")),
        "an untouched chain must fold to one joined row: {rows:#?}"
    );
    assert!(
        !rows.iter().any(|l| l.contains("buried.rs")),
        "a folded directory must not list its files: {rows:#?}"
    );
    // The changed file is lit, and its siblings are a count.
    assert!(
        rows.iter().any(|l| l.contains("● target.rs")),
        "the group's own file must still be lit: {rows:#?}"
    );
    assert!(
        rows.iter().any(|l| l.contains("more")),
        "the files the group misses must fold to a count: {rows:#?}"
    );
    assert!(
        !rows.iter().any(|l| l.contains("three.rs")),
        "a folded file must not be named: {rows:#?}"
    );
}

/// The map folds on the group and on NOTHING ELSE. It used to read the file
/// view's tree, so a directory the reader had put away with `z` arrived here
/// already folded — and the map, whose whole job is to show what the group
/// spans, hid the group's own file behind a `▸ src/ 6`.
#[test]
fn the_group_map_is_not_folded_by_the_file_view() {
    let (_r, mut app, before) = app_on_the_group_that_owns_target(".dfr-map-indep-store");
    let map_before: Vec<&String> = before.iter().filter(|l| l.contains("▸ ")).collect();
    assert!(
        !map_before.is_empty(),
        "the map needs a folded row: {before:#?}"
    );

    // Fold `src/` in the file view — the directory the group's file lives in.
    switch_left_pane(&mut app);
    fold(&mut app, "src");
    assert!(
        !tree_paths(&app).iter().any(|p| p == "src/target.rs"),
        "the file view really did fold it"
    );

    // Back to the plan: the map is unmoved.
    switch_left_pane(&mut app);
    let after = drawn_rows(&mut app);
    assert!(
        after.iter().any(|l| l.contains("● target.rs")),
        "the group's own file stays lit: {after:#?}"
    );
    assert!(
        after.iter().any(|l| l.contains("▸ deep/a/b/c/")),
        "and the map's own folds are unchanged: {after:#?}"
    );
    assert!(
        !after.iter().any(|l| l.contains("▸ src/")),
        "the reader's fold must not reach the map: {after:#?}"
    );
}

/// Reading the plan, a map of the selected group FLOATS over the FOOT of the
/// detail pane at full pane width — the shape the file list takes at the foot
/// of the plan pane. Its height is capped against the group's header, so the
/// full label survives the plan pane's 40 columns, and the diff carries on
/// above it as a preview of what entering the group will show.
#[test]
fn the_group_map_floats_over_the_diff_when_the_plan_is_focused() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Groups;
    let text = drawn_as_is(&mut app);

    assert!(
        text.contains("files in g"),
        "the float names the group: {text}"
    );
    assert!(text.contains("f.rs"), "the tree should list files: {text}");
    assert!(
        text.contains('●'),
        "no file is marked as the group's: {text}"
    );
    // Tree guides, not bare indentation.
    assert!(
        text.contains('└') || text.contains('├'),
        "no tree guides: {text}"
    );

    // The group's header is above the float, and the diff above it too.
    assert!(
        text.contains("[focus] Group 0"),
        "the group's title should be uncovered: {text}"
    );
    assert!(
        text.contains("let filler"),
        "the diff should carry on above the float: {text}"
    );

    // Geometry, not just content: the box starts on the detail pane's own left
    // border column and sits in the pane's lower half. A 100x40 terminal makes
    // the detail pane x 40..100 over a 39-row body.
    let buf = buffer_of(&app);
    let top = (0..39u16)
        .find(|&y| {
            (40..100u16)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains("files in g")
        })
        .expect("the float's title row");
    assert_eq!(
        buf[(40, top)].symbol(),
        "┌",
        "the float should span the detail pane, border to border"
    );
    assert!(
        top >= 39 / 2,
        "the float should sit at the pane's foot, not its head: row {top}"
    );
    assert_eq!(
        buf[(40, 38)].symbol(),
        "└",
        "the float's foot should land on the pane's bottom border row"
    );

    // Tab and the float is gone.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let text = drawn_as_is(&mut app);
    assert!(
        !text.contains("files in g"),
        "the float should lift: {text}"
    );
    assert!(text.contains("let filler"));
}

/// The float grows upward from the foot, and the group's header block is what
/// stops it. A short pane and a group touching every file is the case that
/// would swallow the label the 40-column plan pane already truncates.
#[test]
fn the_group_map_never_covers_the_groups_header() {
    let (_r, mut app) = app_with_many_files();
    app.focus = Focus::Groups;

    // Short on purpose: 13 body rows against a map of nine. Without the cap the
    // float would start at the pane's second content row and swallow the
    // description under the title.
    let backend = ratatui::backend::TestBackend::new(100, 14);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let row_text = |y: u16| {
        (40..100u16)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
    };

    let top = (0..13u16)
        .find(|&y| row_text(y).contains("files in g"))
        .expect("the float should still be drawn");
    // The header block is the title, its description and the blank under them.
    assert!(
        top >= 3,
        "the cap must leave the group's header block uncovered: the float starts at row {top}"
    );
    assert!(
        row_text(1).contains("[focus] Everything"),
        "the group's title should be on screen: {}",
        row_text(1)
    );

    // Shorter still: too short for the header block AND a box. The box is what
    // gives way. A floor of three rows that beat the cap would land on the
    // description and cover the very label the cap exists to protect.
    let backend = ratatui::backend::TestBackend::new(100, 6);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let short_row = |y: u16| {
        (40..100u16)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
    };
    let text: String = buf.content().iter().map(|c| c.symbol()).collect();
    assert!(
        !text.contains("files in g"),
        "the float should lift rather than cover the header: {text}"
    );
    assert!(
        short_row(1).contains("[focus] Everything"),
        "the group's title must survive any pane height: {}",
        short_row(1)
    );
    assert!(
        short_row(2).trim_matches(|c| c == '│' || c == ' ') == "d",
        "the description must survive with it: {}",
        short_row(2)
    );
}

/// A count beside a file names what the reader is being SHOWN of that file.
/// `src/f.rs` carries two hunks in two groups, `+1 −1` each; the map printed
/// the file's own `+2 −2` whichever group was selected, which is a number
/// about something the reader is not looking at.
#[test]
fn the_group_map_counts_the_groups_part_of_a_file() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Groups;
    let rows = drawn_rows(&mut app);
    let row = rows
        .iter()
        .find(|l| l.contains("● f.rs"))
        .unwrap_or_else(|| panic!("the group's file is not lit in the map: {rows:#?}"));
    assert!(
        row.contains("+1 −1"),
        "the map must count the group's part of the file: {row}"
    );
    assert!(
        !row.contains("+2"),
        "the file's own totals are the other question: {row}"
    );
}

/// The same rule in the diff pane's file list: reading a group, a row counts
/// that group's part of the file, and the ✓ asks the same question the number
/// does.
#[test]
fn the_file_list_counts_the_groups_part_of_a_file() {
    use differential_tui::app::Mode;
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    let Mode::FileList { entries, .. } = &app.mode else {
        panic!("f should open the file list");
    };
    assert_eq!(
        (entries[0].adds, entries[0].dels),
        (1, 1),
        "the list must count what is in view: {:?}",
        entries[0].path
    );
}

/// And the whole file is the right answer where the whole file is on screen.
/// The file view's left pane IS the document's tree, so nothing is scoping it
/// to a group.
#[test]
fn the_file_list_counts_the_whole_file_in_the_file_view() {
    use differential_tui::app::Mode;
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Groups;
    app.handle_key(key('f'));
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    let Mode::FileList { entries, .. } = &app.mode else {
        panic!("f should open the file list");
    };
    assert_eq!(
        (entries[0].adds, entries[0].dels),
        (2, 2),
        "the file view shows the whole file, so it counts the whole file"
    );
}

/// A folded skim group shows one file of its three, so both file lists say
/// how many the fold holds back — the group map counted three, and the reader
/// must not be left to wonder where two went (issue 151).
#[test]
fn the_file_lists_count_the_files_a_skim_fold_holds_back() {
    let (_r, mut app) = make_app();
    app.handle_key(key('j'));
    assert!(app.rows.iter().any(|r| r.kind == RowKind::Fold));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));

    let text = drawn_as_is(&mut app);
    assert!(
        text.contains("file 1 of 1 (2 safe to skip)"),
        "the float does not name the fold: {text}"
    );
    app.handle_key(key('f'));
    assert!(
        drawn_as_is(&mut app).contains(" files (2 safe to skip) — "),
        "the modal does not name the fold"
    );
    app.handle_key(key('f'));

    // Unfolded, nothing is held back and neither list says otherwise.
    app.focus = Focus::Groups;
    app.handle_key(key('z'));
    app.focus = Focus::Detail;
    let text = drawn_as_is(&mut app);
    assert!(text.contains("file 1 of 3"), "unfolded list: {text}");
    assert!(!text.contains("safe to skip"), "unfolded list: {text}");

    // A focus group holds nothing back either.
    app.focus = Focus::Groups;
    app.handle_key(key('k'));
    app.focus = Focus::Detail;
    assert!(!drawn_as_is(&mut app).contains("safe to skip"));

    // The file view's rows are not a group's, so there is no fold to count.
    app.focus = Focus::Groups;
    app.handle_key(key('j'));
    app.handle_key(key('z'));
    assert!(app.rows.iter().any(|r| r.kind == RowKind::Fold));
    switch_left_pane(&mut app);
    app.focus = Focus::Detail;
    assert!(!drawn_as_is(&mut app).contains("safe to skip"));
}

/// Reading the detail, a list of the files in view floats over the foot of the
/// plan pane to say where you are and how much is left.
#[test]
fn the_file_list_floats_over_the_plan_when_the_detail_is_focused() {
    let (_r, mut app) = app_with_two_groups_in_one_file();

    app.focus = Focus::Groups;
    assert!(
        !drawn_as_is(&mut app).contains("file 1 of"),
        "the list belongs to the detail pane's focus"
    );

    app.focus = Focus::Detail;
    let text = drawn_as_is(&mut app);
    assert!(text.contains("file 1 of 1"), "no file list drawn: {text}");
    assert!(text.contains("f.rs"), "the file is not listed: {text}");

    // The current file is marked by the row being lit edge to edge, not by a
    // glyph in a column of its own.
    let buf = buffer_of(&app);
    let row = (1..39u16)
        .find(|&y| {
            (1..39u16)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains("f.rs")
        })
        .expect("the file's row");
    assert!(
        (1..39u16).all(|x| buf[(x, row)].bg == theme().selected_bg),
        "the current file's row should be lit the whole way across"
    );
}

/// Both overviews FLOAT, so focus never changes a pane's height. This is the
/// guarantee `spec/tui.md` opens with, and the reason splitting a pane on focus
/// was the wrong shape.
#[test]
fn focus_never_changes_a_pane_height() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.set_viewport(Viewport {
        detail_rows: 30,
        detail_cols: 58,
        plan_rows: 30,
        body_rows: 30 + 2,
        ..Viewport::default()
    });
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Detail);

    // The claim is about the DRAWN panes, not about a field. `viewport` is
    // only ever written by `set_viewport`, which `handle_key` never calls —
    // so asserting it against the value this test set proved nothing. Both
    // panes have to keep their geometry across a focus change.
    let widths = |app: &App| {
        let buf = buffer_of(app);
        let border_row = 0u16;
        (0..100u16)
            .filter(|&x| {
                buf[(x, border_row)].symbol() == "┐" || buf[(x, border_row)].symbol() == "┌"
            })
            .collect::<Vec<_>>()
    };
    let after = widths(&app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Groups);
    assert_eq!(
        widths(&app),
        after,
        "a float must not take room from the pane it covers"
    );
}

/// A long file stops saying which file it is once its header scrolls away.
#[test]
fn a_file_header_sticks_while_scrolled_past_it() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 10,
        detail_cols: 58,
        plan_rows: 10,
        body_rows: 10 + 2,
        ..Viewport::default()
    });

    let header = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::FileHeader(_)))
        .expect("a file header");
    // Park the cursor well below it so the header is off-screen.
    app.cursor = app.rows.len() - 1;
    app.set_viewport(Viewport {
        detail_rows: 10,
        detail_cols: 58,
        plan_rows: 10,
        body_rows: 10 + 2,
        ..Viewport::default()
    });
    assert!(
        app.scroll() > header,
        "not actually scrolled past the header"
    );

    let buf = buffer_of(&app);
    let top: String = (41..99u16).map(|x| buf[(x, 1)].symbol()).collect();
    assert!(
        top.contains("long.rs"),
        "the filename should stick to the top row: {top:?}"
    );

    // Back at the top, nothing is stuck: row one is the group header the rows
    // actually start with, not a filename pinned over it.
    app.cursor = 0;
    app.set_viewport(Viewport {
        detail_rows: 10,
        detail_cols: 58,
        plan_rows: 10,
        body_rows: 10 + 2,
        ..Viewport::default()
    });
    assert_eq!(app.scroll(), 0);
    let buf = buffer_of(&app);
    let top: String = (41..99u16).map(|x| buf[(x, 1)].symbol()).collect();
    assert!(
        top.contains("Everything") && !top.contains("long.rs"),
        "nothing should be stuck at the top of the rows: {top:?}"
    );
}

/// The file modal's counts carry the added and removed colours.
///
/// Two things this used to get wrong. It opened the modal on a fixture whose
/// group holds ONE file, where the counted box never appears at all — and it
/// then swept the whole 100x40 screen for `add_fg`/`del_fg`, which the plan
/// pane inks for every group on every frame. So it passed with the modal's
/// counts left grey, and with no modal on screen.
///
/// The name also used to promise "and the role is a pill". Nothing here ever
/// checked a role or a pill; `the_role_pill_hangs_off_the_plan_panes_right_edge`
/// does that.
#[test]
fn counts_are_coloured_in_the_file_modal() {
    let (_r, mut app) = make_app();
    app.handle_key(key('j')); // the group with three files
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('f'));
    assert!(
        matches!(app.mode, Mode::FileList { .. }),
        "`f` must open the file list"
    );
    let buf = buffer_of(&app);

    // Inside the modal's own box, which is a float over both panes.
    let rows: Vec<String> = (0..40u16)
        .map(|y| (0..100u16).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let top = rows
        .iter()
        .position(|r| r.contains("— enter jump"))
        .expect("the file modal's title row");
    let counted = (top + 1..rows.len())
        .find(|&y| rows[y].contains('+') && rows[y].contains('−'))
        .expect("a modal row carrying counts");

    let inks: Vec<_> = (0..100u16)
        .filter_map(|x| buf[(x, counted as u16)].style().fg)
        .collect();
    assert!(
        inks.contains(&theme().add_fg),
        "the modal's added count is grey: {}",
        rows[counted]
    );
    assert!(
        inks.contains(&theme().del_fg),
        "the modal's removed count is grey: {}",
        rows[counted]
    );
}

/// The role pill hangs off the pane's right edge, so the roles read as a
/// column. Trailing the counts, each started wherever the counts happened to
/// end — a word you could only read by finding it first.
#[test]
fn the_role_pill_hangs_off_the_plan_panes_right_edge() {
    let (_r, mut app) = app_with_dependency_edge();
    app.focus = Focus::Groups;
    let rows = plan_rows(&mut app);
    let ends: Vec<usize> = rows
        .iter()
        .filter(|r| r.contains("foundation") || r.contains("consumer"))
        .map(|r| r.trim_end().chars().count())
        .collect();
    assert!(ends.len() >= 2, "the fixture needs two roles: {rows:?}");
    assert!(
        ends.windows(2).all(|w| w[0] == w[1]),
        "every role should end in the same column: {ends:?}"
    );
    // And that column is the pane's edge, not somewhere in the middle.
    let width = rows[0].chars().count();
    assert!(
        ends[0] + 2 >= width,
        "the pill should reach the right edge: ends at {} of {width}",
        ends[0]
    );
}

/// One fact, one rendering. The role was a pill in the plan pane and grey
/// suffix text on the group header three columns away.
#[test]
fn the_role_wears_the_same_pill_in_both_panes() {
    let (_r, mut app) = app_with_dependency_edge();
    app.focus = Focus::Detail;
    let buf = buffer_of(&app);
    let (_, pill_bg) = theme().pill();

    // The group header row leads the detail pane and carries the role.
    let detail = (1..39u16)
        .find(|&y| {
            (41..99u16)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains("foundation")
        })
        .expect("no role on the group header");
    assert!(
        (41..99u16).any(|x| buf[(x, detail)].style().bg == Some(pill_bg)),
        "the group header's role should be a pill, not grey text"
    );

    // And the plan pane's copy of the same fact wears the same fill.
    let plan = (1..39u16)
        .find(|&y| {
            (1..39u16)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .contains("foundation")
        })
        .expect("no role in the plan pane");
    assert!((1..39u16).any(|x| buf[(x, plan)].style().bg == Some(pill_bg)));
}

/// Drawing a document with no groups must not panic. It was harmlessly a no-op
/// before the sticky header needed to know which file it was in.
#[test]
fn an_empty_document_draws_without_panicking() {
    let (_r, mut app) = make_app();
    app.rows.clear();
    app.cursor = 0;
    app.focus = Focus::Detail;
    let _ = drawn_as_is(&mut app);
    assert!(app.file_at_cursor().is_none());
}

/// A gap wide enough to need several presses shows both of its ends. Narrow it
/// until one press would close it and there is nothing left to choose between
/// them, so it collapses to a single row.
#[test]
fn a_gap_one_press_wide_is_one_row_not_two() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;

    let edge_rows = |app: &App| -> Vec<(usize, Side)> {
        app.rows
            .iter()
            .filter_map(|r| match r.kind {
                RowKind::ContextEdge { hunk, side, .. } => Some((hunk, side)),
                _ => None,
            })
            .collect()
    };
    // Two hunks mid-file: an outer end each, plus both ends of the gap between.
    assert_eq!(
        edge_rows(&app).len(),
        4,
        "expected two outer ends and both ends of the middle gap"
    );

    // The middle gap is thirteen lines against a ten-line step, so one press
    // leaves three — close enough that the next press finishes it.
    let (upper, _) = edge_rows(&app)[0];
    put_cursor_on(
        &mut app,
        |k| matches!(*k, RowKind::ContextEdge { hunk: h, side: Side::Down, .. } if h == upper),
    );
    app.handle_key(key('z'));

    let rows = edge_rows(&app);
    assert_eq!(
        rows.len(),
        3,
        "the middle gap should now speak with one row, got {rows:?}"
    );
    assert!(
        app.rows.iter().any(|r| r.button == Some("↕")),
        "a one-press gap should offer both directions at once"
    );
    // And it still works: one more press closes it and the blocks merge.
    put_cursor_on(
        &mut app,
        |k| matches!(*k, RowKind::ContextEdge { hunk: h, side: Side::Down, .. } if h == upper),
    );
    app.handle_key(key('z'));
    assert_eq!(
        edge_rows(&app).len(),
        2,
        "closing the gap should leave only the outer ends"
    );
}

/// The file view's left pane IS a file tree, so neither float belongs there: a
/// map of one group would name a group nothing is selecting, and a file list
/// would be the pane behind it.
#[test]
fn neither_float_appears_in_the_file_view() {
    let (_r, mut app) = app_with_two_groups_in_one_file();

    app.focus = Focus::Groups;
    assert!(
        drawn_as_is(&mut app).contains("files in g"),
        "no map to lose"
    );
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Files);
    let text = drawn_as_is(&mut app);
    assert!(
        !text.contains("files in g"),
        "the group map should not follow into the file view: {text}"
    );

    app.focus = Focus::Detail;
    let text = drawn_as_is(&mut app);
    assert!(
        !text.contains("file 1 of"),
        "nor should the file list: {text}"
    );

    // Both come back on the way out.
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Groups);
    assert!(drawn_as_is(&mut app).contains("file 1 of"));
}

/// The file view's tree gets the same connectors the floating map draws.
#[test]
fn the_file_view_tree_is_drawn_with_guides() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    switch_left_pane(&mut app);
    let text = drawn_as_is(&mut app);
    assert!(
        text.contains('└') || text.contains('├'),
        "the file tree should have guides: {text}"
    );
}

/// Two blocks either side of one unlisted hunk both name it as what comes
/// next, and pressing either crosses the same hunk — so one row says it.
#[test]
fn one_hunk_between_two_blocks_is_offered_once_not_twice() {
    let r = TestRepo::new();
    // Three changes, the middle one far enough from both to stay its own block.
    let body = |a: &str, b: &str, c: &str| -> Vec<u8> {
        let mut out = String::new();
        for i in 1..=60 {
            match i {
                10 => out.push_str(a),
                30 => out.push_str(b),
                50 => out.push_str(c),
                _ => out.push_str(&format!("let filler{i} = {i};\n")),
            }
        }
        out.into_bytes()
    };
    r.write(
        "src/f.rs",
        &body("let a = 1;\n", "let b = 2;\n", "let c = 3;\n"),
    );
    r.commit_all("base");
    r.write(
        "src/f.rs",
        &body("let a = 11;\n", "let b = 22;\n", "let c = 33;\n"),
    );
    r.commit_all("head");
    // The outer two share a group; the middle one is its own, so it is foreign
    // to the view and sits between two blocks.
    let backend = FakeBackend::new("fake", |ids| {
        let mut outer: Vec<&str> = ids.iter().map(String::as_str).collect();
        let middle = outer.remove(1);
        format!(
            r#"{{"groups": [{}, {}]}}"#,
            json_group("Outer", "focus", &outer),
            json_group("Middle", "focus", &[middle])
        )
    });
    let mut app = open_app_with(&r, &backend, ".dfr-between-store");
    app.focus = Focus::Detail;

    // Open both inner gaps until each names the hunk between them.
    for _ in 0..12 {
        let spent = app.rows.iter().enumerate().find_map(|(i, r)| {
            matches!(
                r.kind,
                RowKind::ContextEdge {
                    crossing: false,
                    ..
                }
            )
            .then_some(i)
        });
        let Some(i) = spent else { break };
        app.cursor = i;
        app.handle_key(key('z'));
    }

    let naming: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::ContextEdge { crossing: true, .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        naming.len(),
        1,
        "one hunk between two blocks should be offered once, got {} rows",
        naming.len()
    );
    // And that row says it reaches both ways.
    assert_eq!(app.rows[naming[0]].button, Some("↕"));
    // Pressing it still works: the hunk arrives, marked foreign.
    app.cursor = naming[0];
    app.handle_key(key('z'));
    assert!(shows_a_foreign_hunk(&app), "z should have crossed it");
}

/// The finding editor floats over the diff and names what it annotates: a note
/// whose subject you cannot see is one you have to trust yourself about.
#[test]
fn the_finding_editor_floats_and_names_its_subject() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    put_cursor_on(&mut app, |k| matches!(k, RowKind::HunkHeader { .. }));
    app.handle_key(key('c'));
    assert!(matches!(app.mode, Mode::Editing { .. }));

    let text = drawn_as_is(&mut app);
    assert!(text.contains("long.rs · L"), "no file·line title: {text}");
    // The keys are in a footer inside the box: `enter` saves, and a newline is
    // shift+enter where the terminal reports it or a trailing `\` where it
    // does not.
    assert!(text.contains("enter save"), "no save key shown: {text}");
    assert!(text.contains("esc"), "no cancel key shown: {text}");
    assert!(text.contains("newline"), "no newline key shown: {text}");

    // It floats: the diff is still there around it.
    assert!(
        text.contains("let filler"),
        "the editor should float over the diff, not replace it: {text}"
    );
}

/// `enter` saves. A newline is shift+enter, or a trailing `\` before `enter`
/// for the terminals that report shift+enter as plain enter — which is most of
/// them without the keyboard enhancements this reviewer does not ask for.
#[test]
fn the_composer_saves_on_enter_and_takes_a_newline_two_ways() {
    let open = |app: &mut App| {
        put_cursor_on(app, |k| matches!(k, RowKind::HunkHeader { .. }));
        app.handle_key(key('c'));
        assert!(matches!(app.mode, Mode::Editing { .. }));
    };
    let typed = |app: &mut App, text: &str| {
        for c in text.chars() {
            app.handle_key(key(c));
        }
    };
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);

    // Plain enter saves.
    let (_r, mut app) = app_with_a_long_file();
    open(&mut app);
    typed(&mut app, "one line");
    app.handle_key(enter);
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.session.findings().len(), 1);
    assert_eq!(app.session.findings()[0].body, "one line");

    // shift+enter makes a second line, and enter then saves both.
    let (_r, mut app) = app_with_a_long_file();
    open(&mut app);
    typed(&mut app, "first");
    app.handle_key(shift_enter);
    typed(&mut app, "second");
    app.handle_key(enter);
    assert_eq!(app.session.findings()[0].body, "first\nsecond");

    // A trailing `\` before enter does the same, and the `\` is not kept.
    let (_r, mut app) = app_with_a_long_file();
    open(&mut app);
    typed(&mut app, "first\\");
    app.handle_key(enter);
    assert!(matches!(app.mode, Mode::Editing { .. }));
    assert!(
        matches!(app.mode, Mode::Editing { .. }),
        "the box should stay open"
    );
    typed(&mut app, "second");
    app.handle_key(enter);
    assert_eq!(app.session.findings()[0].body, "first\nsecond");

    // A `\` the reader went BACK to a line to leave is not a newline request:
    // the key looks at the character before the cursor, and `delete_char`
    // takes what the cursor sits after.
    let (_r, mut app) = app_with_a_long_file();
    open(&mut app);
    typed(&mut app, "ends with a slash\\");
    for _ in 0..5 {
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }
    app.handle_key(enter);
    assert!(
        matches!(app.mode, Mode::Normal),
        "enter mid-line still saves"
    );
    assert_eq!(
        app.session.findings()[0].body,
        "ends with a slash\\",
        "nothing should have been deleted"
    );

    // `ctrl-s` still saves, for whoever's terminal passes it.
    let (_r, mut app) = app_with_a_long_file();
    open(&mut app);
    typed(&mut app, "by ctrl-s");
    app.handle_key(ctrl('s'));
    assert_eq!(app.session.findings()[0].body, "by ctrl-s");
}

/// Bracketed paste is enabled so a multi-line paste arrives whole. The event
/// was dropped, so pasting into the composer did nothing.
#[test]
fn a_paste_lands_in_the_composer() {
    let (_r, mut app) = app_with_a_long_file();
    put_cursor_on(&mut app, |k| matches!(k, RowKind::HunkHeader { .. }));
    app.handle_key(key('c'));
    app.handle_paste("pasted\nover two lines");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.session.findings()[0].body, "pasted\nover two lines");

    // In normal mode there is no field for it, so it does nothing.
    let (_r, mut app) = app_with_a_long_file();
    let before = app.rows.len();
    app.handle_paste("stray");
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.rows.len(), before);
}

/// `c` on a diff row annotates THAT line, not the whole hunk it sits in.
#[test]
fn c_on_a_line_anchors_to_that_line() {
    let (_r, mut app) = app_with_a_long_file();
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    let at = app.rows[row].line.clone().expect("its line");
    app.cursor = row;
    app.focus = Focus::Detail;

    app.handle_key(key('c'));
    app.handle_key(key('x'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let f = &app.session.findings()[0];
    assert_eq!(f.anchor.side, at.side);
    assert_eq!(f.anchor.line, at.line, "the anchor is the cursor's line");
    assert_eq!(f.anchor.end_line, at.line, "one line, not a range");
    assert_eq!(f.anchor.line_span(), at.line.to_string());
}

/// `V` starts a selection the cursor extends; `c` then annotates the run.
#[test]
fn v_selects_lines_and_c_annotates_the_run() {
    let (_r, mut app) = app_with_a_long_file();
    // Three consecutive rows that are all lines of the same side.
    let start = app
        .rows
        .windows(3)
        .position(|w| {
            w.iter()
                .all(|r| r.line.as_ref().is_some_and(|l| l.side == "new"))
        })
        .expect("three new-side rows in a row");
    app.cursor = start;
    app.focus = Focus::Detail;

    app.handle_key(key('v'));
    assert_eq!(app.visual, Some(start));
    app.handle_key(key('j'));
    app.handle_key(key('j'));

    app.handle_key(key('c'));
    assert_eq!(app.visual, None, "writing the finding ends the selection");
    app.handle_key(key('x'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let first = app.rows[start].line.clone().unwrap();
    let last = app.rows[app.cursor].line.clone().unwrap();
    let f = &app.session.findings()[0];
    assert_eq!(f.anchor.line, first.line);
    assert_eq!(f.anchor.end_line, last.line);
    assert_eq!(f.anchor.span, last.line - first.line);
    assert_eq!(
        f.anchor.line_span(),
        format!("{}-{}", first.line, last.line)
    );

    // `esc` drops a selection rather than doing anything else.
    app.handle_key(key('v'));
    assert!(app.visual.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.visual, None);
}

/// A finding is drawn under the line it annotates, not under the hunk header.
#[test]
fn a_finding_sits_under_the_line_it_annotates() {
    let (_r, mut app) = app_with_a_long_file();
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    app.cursor = row;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_key(key('x'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let at = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("a finding row");
    let above = app.rows[at - 1]
        .line
        .clone()
        .expect("the row above a finding is the line it annotates");
    assert_eq!(above.line, app.session.findings()[0].anchor.end_line);

    // `dd` still deletes it from wherever it landed.
    app.cursor = at;
    app.handle_key(key('d'));
    app.handle_key(key('d'));
    assert!(app.session.findings().is_empty());
}

/// A note is prose about the code above it, so it is drawn as a quoted panel:
/// every line behind a muted rail, in muted italics.
#[test]
fn a_finding_is_a_quoted_panel_of_all_its_lines() {
    let (_r, mut app) = app_with_a_long_file();
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    app.cursor = row;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_paste("first line\nsecond line");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let at: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::Finding(..)))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(at.len(), 2, "one row per line of the note");
    assert!(
        at.windows(2).all(|w| w[1] == w[0] + 1),
        "the panel is contiguous"
    );

    let text = drawn_rows(&mut app);
    let panel: Vec<&String> = text.iter().filter(|r| r.contains("line")).collect();
    assert!(
        text.iter().any(|r| r.contains("▍ first line")),
        "no rail on the note: {panel:?}"
    );
    assert!(
        text.iter().any(|r| r.contains("▍ second line")),
        "the second line is dropped: {panel:?}"
    );
    assert!(
        !text.iter().any(|r| r.contains("◆ first")),
        "the marker glyph is gone from the note itself"
    );

    // `dd` deletes the note from ANY of its lines.
    app.cursor = at[1];
    app.handle_key(key('d'));
    app.handle_key(key('d'));
    assert!(app.session.findings().is_empty());
}

/// `c` on a line that already carries a note opens THAT note. Two notes on one
/// line would each be half the story, and there was no way to fix a typo but
/// delete and retype.
#[test]
fn c_on_a_commented_line_rewrites_the_note() {
    let (_r, mut app) = app_with_a_long_file();
    let line = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    app.cursor = line;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_paste("frist draft");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let id = app.session.findings()[0].id.clone();

    // From the line, and from the note's own row: both open the same note,
    // with its text already in the box.
    let note = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("a note row");
    for at in [line, note] {
        app.cursor = at;
        app.handle_key(key('c'));
        let Mode::Editing {
            editor, rewriting, ..
        } = &app.mode
        else {
            panic!("the box should be open");
        };
        assert_eq!(rewriting.as_deref(), Some(id.as_str()), "from row {at}");
        assert_eq!(editor.lines().join("\n"), "frist draft", "from row {at}");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }

    // The box opens at the END of the note, so a second thought is typed
    // rather than prepended. Rewriting keeps the id and the anchor.
    let before = app.session.findings()[0].anchor.line;
    let clear = |app: &mut App, n: usize| {
        for _ in 0..n {
            app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        }
    };
    app.cursor = line;
    app.handle_key(key('c'));
    app.handle_paste(", on reflection");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.session.findings().len(), 1, "no second note was filed");
    assert_eq!(app.session.findings()[0].id, id, "the id is a handle");
    assert_eq!(app.session.findings()[0].body, "frist draft, on reflection");
    assert_eq!(app.session.findings()[0].anchor.line, before);

    // Emptying the box leaves the note alone: `dd` is how a note is deleted,
    // and that is a deliberate press.
    app.cursor = line;
    app.handle_key(key('c'));
    clear(&mut app, 64);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.session.findings().len(), 1);
    assert_eq!(app.session.findings()[0].body, "frist draft, on reflection");

    // A selection is the exception: it asks for a note about the run.
    app.cursor = line;
    app.handle_key(key('v'));
    app.handle_key(key('j'));
    app.handle_key(key('c'));
    assert!(
        matches!(
            &app.mode,
            Mode::Editing {
                rewriting: None,
                ..
            }
        ),
        "a selection files a new note"
    );
}

/// A selection has to cross a hunk. Only a gap the reader never opened stops
/// it — a hunk's header and its removed and added rows are one continuous
/// stretch of one file.
#[test]
fn a_selection_crosses_a_hunk_from_either_side() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    // The removed half of the modification: an OLD-side row, and the one the
    // run used to get stuck on, since every row after it is new-side.
    let removed = app
        .rows
        .iter()
        .position(|r| r.line.as_ref().is_some_and(|l| l.side == "old"))
        .expect("a removed row");
    let old_line = app.rows[removed].line.clone().unwrap().line;

    app.cursor = removed;
    app.handle_key(key('v'));
    for _ in 0..3 {
        app.handle_key(key('j'));
    }
    app.handle_key(key('c'));
    let Mode::Editing { lines: Some(l), .. } = &app.mode else {
        panic!("no lines picked");
    };
    assert_eq!(l.side, "old", "the anchor's side is the run's side");
    assert_eq!(l.start, old_line);
    assert!(
        l.end > old_line,
        "an old-side run must reach the context below the hunk: {l:?}"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // And the same downward, from the context above through the hunk.
    let above = app.rows[..removed]
        .iter()
        .rposition(|r| r.line.as_ref().is_some_and(|l| l.side == "new"))
        .expect("a context row above");
    app.cursor = above;
    app.handle_key(key('v'));
    for _ in 0..4 {
        app.handle_key(key('j'));
    }
    app.handle_key(key('c'));
    let Mode::Editing { lines: Some(l), .. } = &app.mode else {
        panic!("no lines picked");
    };
    let from = app.rows[above].line.clone().unwrap().line;
    assert_eq!((l.side.as_str(), l.start), ("new", from));
    // Past the hunk's header, its removed row and its added row: three rows
    // that are not two consecutive new-side lines, and used to end the run.
    assert!(
        l.end >= from + 2,
        "the run should reach past the hunk: {l:?}"
    );
}

/// `v` is how a reader gets into a selection, so it is the key their hand is
/// on to get out of one.
#[test]
fn v_toggles_the_selection_off() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    app.cursor = app
        .rows
        .iter()
        .position(|r| r.line.is_some())
        .expect("a line row");

    app.handle_key(key('v'));
    assert!(app.visual.is_some());
    app.handle_key(key('v'));
    assert_eq!(app.visual, None, "a second v drops it");

    // And it starts a fresh one rather than staying off.
    app.handle_key(key('v'));
    assert_eq!(app.visual, Some(app.cursor));
}

/// A selection stops where the file's line numbers do. Dragging from line 23
/// across `13 lines hidden` to line 37 used to file a note claiming fifteen
/// lines, thirteen of which were never on screen.
#[test]
fn a_selection_stops_at_a_gap_it_never_opened() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    let boundary = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::ContextEdge { .. }) && r.button == Some("↓"))
        .expect("a downward boundary");
    let above = app.rows[..boundary]
        .iter()
        .rposition(|r| r.line.is_some())
        .expect("a line above it");
    let last_seen = app.rows[above].line.clone().unwrap();

    // Select from that line and walk down past the gap onto a line beyond it.
    app.cursor = above;
    app.handle_key(key('v'));
    for _ in 0..6 {
        app.handle_key(key('j'));
        if app.cursor > boundary && app.rows[app.cursor].line.is_some() {
            break;
        }
    }
    let beyond = app.rows[app.cursor]
        .line
        .clone()
        .expect("a line past the gap");
    assert!(
        beyond.line > last_seen.line + 1,
        "the fixture needs a real gap: {} to {}",
        last_seen.line,
        beyond.line
    );

    app.handle_key(key('c'));
    let Mode::Editing { lines: Some(l), .. } = &app.mode else {
        panic!("no lines picked");
    };
    assert_eq!(
        (l.start, l.end),
        (last_seen.line, last_seen.line),
        "the selection should have stopped at the last line the reader saw"
    );
}

/// A note over a RANGE is drawn under its last line, so the run above it is
/// not adjacent to it. Standing anywhere in the run lights the whole thing.
#[test]
fn a_ranged_note_lights_every_line_it_covers() {
    let (_r, mut app) = app_with_a_long_file();
    let start = app
        .rows
        .windows(3)
        .position(|w| {
            w.iter()
                .all(|r| r.line.as_ref().is_some_and(|l| l.side == "new"))
        })
        .expect("three new-side rows in a row");
    app.cursor = start;
    app.focus = Focus::Detail;

    app.handle_key(key('v'));
    app.handle_key(key('j'));
    app.handle_key(key('j'));
    app.handle_key(key('c'));
    app.handle_paste("about all three");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let a = app.session.findings()[0].anchor.clone();
    assert_eq!(
        a.end_line - a.line,
        2,
        "the fixture needs a three-line note"
    );

    // The rows of the run, and the note's own row after the last of them.
    let covered: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.line
                .as_ref()
                .is_some_and(|l| (a.line..=a.end_line).any(|n| l.holds(&a.side, n)))
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(covered.len(), 3, "three lines: {covered:?}");
    let note = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("a note row");
    assert_eq!(note, covered[2] + 1, "the note hangs off the LAST line");

    // Standing anywhere in the run, or on the note: the whole cluster lights.
    let lit = |app: &mut App, rows: &[usize]| -> bool {
        let buf = buffer_of(app);
        rows.iter().all(|&i| {
            let y = (i - app.scroll()) as u16 + 1;
            buf[(40, y)].style().fg == Some(theme().finding_fg)
        })
    };
    let cluster: Vec<usize> = covered
        .iter()
        .copied()
        .chain(std::iter::once(note))
        .collect();
    for at in cluster.clone() {
        app.cursor = at;
        assert!(
            lit(&mut app, &cluster),
            "standing on row {at} should light every row of the note"
        );
    }

    // And `c` from the FIRST line of the run rewrites that note.
    app.cursor = covered[0];
    app.handle_key(key('c'));
    assert!(
        matches!(
            &app.mode,
            Mode::Editing {
                rewriting: Some(_),
                ..
            }
        ),
        "the first line of a run is in the note about that run"
    );
}

/// A note is drawn under its line, and the only sign the two belonged together
/// was that they were adjacent. Standing on either lights both, in the border
/// column and on the note's own rail.
#[test]
fn standing_on_a_note_or_its_line_lights_both() {
    let (_r, mut app) = app_with_a_long_file();
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    app.cursor = row;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_paste("first\nsecond");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let note = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("a note row");
    let line = note - 1;

    // The border column of the line and of both note rows, and the rails.
    let lit = |app: &mut App| -> (Vec<bool>, Vec<bool>) {
        let buf = buffer_of(app);
        let y = |i: usize| (i - app.scroll()) as u16 + 1;
        let border = (line..=note + 1)
            .map(|i| buf[(40, y(i))].style().fg == Some(theme().finding_fg))
            .collect();
        let rails = (note..=note + 1)
            .map(|i| {
                (41..99u16).any(|x| {
                    buf[(x, y(i))].symbol() == "▍"
                        && buf[(x, y(i))].style().fg == Some(theme().finding_fg)
                })
            })
            .collect();
        (border, rails)
    };

    // The cursor is elsewhere: nothing is lit.
    app.cursor = app
        .rows
        .iter()
        .enumerate()
        .position(|(i, r)| !(line..=note + 1).contains(&i) && r.kind.selectable())
        .expect("a row outside the cluster");
    let (border, rails) = lit(&mut app);
    assert!(
        !border.iter().any(|b| *b),
        "nothing should be lit from away"
    );
    assert!(!rails.iter().any(|b| *b), "the rail stays muted from away");

    // On the line, then on each row of the note: all three light every time.
    for at in [line, note, note + 1] {
        app.cursor = at;
        let (border, rails) = lit(&mut app);
        assert!(
            border.iter().all(|b| *b),
            "the border should run findings-coloured down the cluster, from row {at}"
        );
        assert!(
            rails.iter().all(|b| *b),
            "both rails should be findings-coloured, from row {at}"
        );
    }
}

/// The summary is pasted where nothing knows what `g7` was, so it carries the
/// file, the lines and the note — and no group.
#[test]
fn the_findings_summary_names_no_group() {
    let (_r, mut app) = app_with_a_long_file();
    let row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Diff(_)) && r.line.is_some())
        .expect("a diff row with a line");
    let at = app.rows[row].line.clone().unwrap();
    app.cursor = row;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_paste("look here");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let summary = app.findings_summary();
    assert!(
        summary.contains(&format!("src/long.rs:{}: look here", at.line)),
        "the summary should be file:lines: note — got {summary:?}"
    );
    for label in app.groups().iter().map(|g| g.label.clone()) {
        assert!(
            !summary.contains(&label),
            "the group's label leaked: {summary:?}"
        );
    }
}

/// A finding filed from a hunk header has no line, so it annotates the hunk —
/// and it is drawn where every finding used to be, under that header.
#[test]
fn a_finding_from_a_header_anchors_the_hunk_and_sits_under_it() {
    let (_r, mut app) = app_with_a_long_file();
    let header = put_cursor_on(&mut app, |k| matches!(k, RowKind::HunkHeader { .. }));
    let hunk = app.rows[header].kind.hunk().expect("its hunk");
    app.handle_key(key('c'));
    app.handle_key(key('x'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let f = &app.session.findings()[0];
    assert_eq!(
        f.anchor.offset, 0,
        "a header annotates the hunk's first line"
    );
    assert_eq!(f.anchor.span, 0);
    assert_eq!(f.anchor.hunk_digest, app.session.doc().hunks[hunk].digest);

    let at = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("a finding row");
    assert!(
        app.rows[..at]
            .iter()
            .rev()
            .find_map(|r| r.line.as_ref().map(|l| l.line))
            .is_some_and(|l| l == f.anchor.end_line)
            || matches!(app.rows[at - 1].kind, RowKind::HunkHeader { .. }),
        "it belongs under its line or, failing that, under its header"
    );
}

/// Write a note on the first row that can carry one, and return its row.
fn note_on(
    app: &mut App,
    pred: impl Fn(&differential_tui::rows::Row) -> bool,
    body: &str,
) -> usize {
    let at = app.rows.iter().position(&pred).expect("no row matches");
    app.cursor = at;
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    app.handle_paste(body);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    at
}

fn findings_modal(app: &App) -> (usize, usize) {
    match &app.mode {
        Mode::Findings {
            entries, selected, ..
        } => (entries.len(), *selected),
        _ => panic!("the findings modal should be open"),
    }
}

/// A note is written on a line and drawn under it, which is no help at all in
/// answering "what have I found". `F` is the list.
#[test]
fn f_opens_every_finding_in_one_list() {
    let (_r, mut app) = app_with_a_long_file();
    note_on(&mut app, |r| r.line.is_some(), "the first thing");
    let second = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.line.is_some() && !matches!(r.kind, RowKind::Finding(..)))
        .map(|(i, _)| i)
        .nth(4)
        .expect("a second line");
    app.cursor = second;
    app.handle_key(key('c'));
    app.handle_paste("the second thing");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let before = app.cursor;
    app.handle_key(key('F'));
    assert_eq!(findings_modal(&app), (2, 0));

    let rows = drawn_rows(&mut app);
    for want in ["findings · 2", "the first thing", "the second thing"] {
        assert!(
            rows.iter().any(|r| r.contains(want)),
            "{want:?} missing from the list: {rows:#?}"
        );
    }
    // Each note says where it is, so the list answers without the diff — and
    // the location column is as wide as the longest of them, so a long path
    // still leaves a gap rather than butting against its note.
    let listed: Vec<&String> = rows.iter().filter(|r| r.contains("src/long.rs:")).collect();
    assert_eq!(listed.len(), 2, "both notes should say where they are");
    for row in listed {
        let at = row.find("src/long.rs:").expect("the location");
        let note = row.find("the ").expect("the note");
        let gap = &row[at..note];
        assert!(
            gap.ends_with("  "),
            "no gap between the location and the note: {row:?}"
        );
    }

    // `esc` closes it and moves nothing.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(app.cursor, before, "closing the list should move nothing");

    // And it opens from the plan pane too: it is about the review, not a pane.
    app.focus = Focus::Groups;
    app.handle_key(key('F'));
    assert!(matches!(app.mode, Mode::Findings { .. }));
}

/// `enter` puts the cursor on the note — including one in a group the reader
/// is not in, whose rows do not exist until that group is selected.
#[test]
fn enter_reaches_a_note_in_another_group() {
    let (_r, mut app) = app_with_two_groups_in_one_file();
    app.focus = Focus::Detail;
    note_on(&mut app, |r| r.line.is_some(), "in the first group");
    let wrote_in = app.selected_group;

    // Walk to a group that does not hold it: its rows are gone entirely.
    app.handle_key(key('J'));
    assert_ne!(app.selected_group, wrote_in, "the fixture needs two groups");
    assert!(
        !app.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::Finding(..))),
        "the note should not be in this group's rows"
    );

    app.handle_key(key('F'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(
        app.selected_group, wrote_in,
        "it should go to the note's group"
    );
    assert_eq!(app.focus, Focus::Detail);
    let note = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("the note's row, once its group is selected");
    assert_eq!(app.cursor, note, "the cursor should land on the note");
}

/// The same across FILES, in the file view: a note's rows exist only while its
/// own file is the selected tree row.
#[test]
fn enter_reaches_a_note_in_another_file() {
    use differential_tui::app::TreeKind;
    let r = TestRepo::new();
    for (name, before) in [("src/one.rs", "alpha = 1\n"), ("src/two.rs", "beta = 1\n")] {
        r.write(name, before.as_bytes());
    }
    r.commit_all("base");
    r.write("src/one.rs", b"alpha = 2\n");
    r.write("src/two.rs", b"beta = 2\n");
    r.commit_all("head");

    let backend = skim_first_backend();
    let mut app = open_app_with(&r, &backend, ".dfr-jump-file-store");
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Files);

    // Stand on the first file and write a note in it.
    app.selected_file = app
        .tree
        .iter()
        .position(|e| matches!(&e.kind, TreeKind::File { .. }))
        .expect("a file row");
    app.rebuild_rows();
    let wrote_in = app.selected_file;
    note_on(&mut app, |r| r.line.is_some(), "in the first file");

    // Move to the other file: the note's rows go with it.
    app.focus = Focus::Groups;
    let other = app
        .tree
        .iter()
        .enumerate()
        .filter(|(i, e)| *i != wrote_in && matches!(&e.kind, TreeKind::File { .. }))
        .map(|(i, _)| i)
        .next()
        .expect("a second file");
    app.selected_file = other;
    app.rebuild_rows();
    assert!(
        !app.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::Finding(..))),
        "the note should not be in this file's rows"
    );

    app.handle_key(key('F'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.selected_file, wrote_in,
        "it should go to the note's file"
    );
    let note = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::Finding(..)))
        .expect("the note's row, once its file is selected");
    assert_eq!(app.cursor, note);
}

/// `dd` deletes the selected note and the list stays open/// `dd` deletes the selected note and the list stays open — a reviewer
/// clearing up has more than one to clear.
#[test]
fn dd_in_the_list_deletes_one_and_stays() {
    let (_r, mut app) = app_with_a_long_file();
    for (n, body) in ["one", "two", "three"].iter().enumerate() {
        let at = app
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.line.is_some() && !matches!(r.kind, RowKind::Finding(..)))
            .map(|(i, _)| i)
            .nth(n * 3)
            .expect("a line");
        app.cursor = at;
        app.focus = Focus::Detail;
        app.handle_key(key('c'));
        app.handle_paste(body);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    assert_eq!(app.session.findings().len(), 3);

    app.handle_key(key('F'));
    app.handle_key(key('j'));
    assert_eq!(findings_modal(&app), (3, 1));
    let Mode::Findings {
        entries, selected, ..
    } = &app.mode
    else {
        unreachable!()
    };
    let doomed = entries[*selected].body.clone();

    // One `d` is a latch, not a delete.
    app.handle_key(key('d'));
    assert_eq!(app.session.findings().len(), 3, "one d deletes nothing");
    app.handle_key(key('d'));

    assert_eq!(app.session.findings().len(), 2);
    assert!(
        !app.session.findings().iter().any(|f| f.body == doomed),
        "the wrong note went"
    );
    assert_eq!(
        findings_modal(&app).0,
        2,
        "the list stays open, one shorter"
    );
    // And the diff lost it too, not just the list.
    let notes = app
        .rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::Finding(..)))
        .count();
    assert_eq!(notes, 2, "the diff should carry the two that are left");
}

/// Clearing every note is the only irreversible thing in this app, so it asks.
#[test]
fn d_clears_everything_but_only_after_a_yes() {
    let (_r, mut app) = app_with_a_long_file();
    note_on(&mut app, |r| r.line.is_some(), "the only one");
    app.handle_key(key('F'));

    // `D` alone deletes nothing; it asks, and the box says so.
    app.handle_key(key('D'));
    assert_eq!(app.session.findings().len(), 1, "D alone deletes nothing");
    assert!(
        drawn_rows(&mut app)
            .iter()
            .any(|r| r.contains("delete this note?")),
        "the confirmation should be on screen"
    );

    // Anything but `y` is a slip, and a slip must not empty the store.
    app.handle_key(key('n'));
    assert_eq!(app.session.findings().len(), 1);
    assert!(app.status.contains("nothing deleted"));

    // A BARE `y`. Some terminals report ctrl-y as `Char('y')` with a modifier,
    // and the one irreversible action here must not answer to a chord. The
    // list is still open — cancelling closed the question, not the list.
    app.handle_key(key('D'));
    app.handle_key(ctrl('y'));
    assert_eq!(app.session.findings().len(), 1, "ctrl-y is not a yes");
    assert!(app.status.contains("nothing deleted"));

    app.handle_key(key('D'));
    app.handle_key(key('y'));
    assert!(app.session.findings().is_empty(), "D then y clears the lot");
    assert!(matches!(app.mode, Mode::Normal), "an empty list closes");
}

/// An orphaned note — one whose code is gone — has no row anywhere: it matches
/// no line and no hunk digest, so `place_findings` emits nothing for it. Before
/// this list its body could not be read in the app at all, and `dd` could not
/// reach it. It is the one thing here the list is not a convenience for.
#[test]
fn the_list_is_the_only_door_to_an_orphaned_note() {
    use differential_engine::review_state::{Anchor, Finding};

    let r = TestRepo::new();
    r.write("src/f.txt", b"alpha = 1\nbeta = 2\n");
    r.commit_all("base");
    r.write("src/f.txt", b"alpha = 11\nbeta = 2\n");
    r.commit_all("head");

    // A note the current plan can re-anchor to nothing: no hunk carries that
    // digest, and no hunk carries that text.
    let store_dir = r.root.join(".dfr-orphan-store");
    let store = FsReviewStore::at(store_dir.clone()).unwrap();
    let orphan = Finding::new(
        1,
        "the code this was about is gone".into(),
        "an older plan".into(),
        Anchor {
            file: "src/vanished.txt".into(),
            side: "new".into(),
            line: 12,
            end_line: 12,
            hunk_digest: "a digest no hunk has".into(),
            line_text: "a line no file has".into(),
            ..Anchor::default()
        },
    );
    store.save_findings(&[orphan]).unwrap();

    let backend = skim_first_backend();
    let mut app = open_app_with(&r, &backend, ".dfr-orphan-store");
    assert_eq!(app.session.findings().len(), 1);
    assert_eq!(
        app.session.findings()[0].status,
        differential_engine::review_state::FindingStatus::Orphaned
    );

    // It has no row, so nothing in the diff pane can reach it.
    assert!(
        !app.rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::Finding(..))),
        "an orphan has no row to stand on"
    );

    // The list has it, under its own rule, with its body readable.
    app.handle_key(key('F'));
    let rows = drawn_rows(&mut app);
    for want in [
        "1 orphaned",
        "orphaned ──",
        "the code this was about is gone",
    ] {
        assert!(
            rows.iter().any(|r| r.contains(want)),
            "{want:?} missing: {rows:#?}"
        );
    }

    // And `dd` reaches it, which nothing else does.
    app.handle_key(key('d'));
    app.handle_key(key('d'));
    assert!(app.session.findings().is_empty(), "dd must reach an orphan");
}

/// More notes than the box is tall. The file-list modal scrolls too, but a
/// review has more notes than it has files, so this list is where it shows.
#[test]
fn the_list_scrolls_to_keep_the_selection_on_screen() {
    let (_r, mut app) = app_with_a_long_file();
    app.set_viewport(Viewport {
        detail_rows: 4,
        detail_cols: 58,
        plan_rows: 4,
        body_rows: 4 + 2,
        ..Viewport::default()
    });
    let lines: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.line.is_some())
        .map(|(i, _)| i)
        .collect();
    assert!(lines.len() >= 6, "the fixture needs six lines to annotate");
    for (n, at) in lines.iter().take(6).enumerate() {
        // Each note adds a row, so re-find the line rather than trusting `at`.
        let at = app
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.line.is_some())
            .map(|(i, _)| i)
            .nth(n)
            .unwrap_or(*at);
        app.cursor = at;
        app.focus = Focus::Detail;
        app.handle_key(key('c'));
        app.handle_paste(&format!("note {n}"));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    let total = app.session.findings().len();
    assert!(total >= 5, "wrote {total} notes, wanted at least five");

    app.handle_key(key('F'));
    for _ in 0..total {
        app.handle_key(key('j'));
    }
    let Mode::Findings {
        selected, scroll, ..
    } = &app.mode
    else {
        panic!("the list should be open");
    };
    assert_eq!(*selected, total - 1, "j should reach the last note");
    assert!(
        *scroll > 0,
        "the window should have moved to keep it on screen"
    );
    assert!(*scroll <= *selected, "and never past the selection");
}

/// `?` answers for where the reader is standing, and nothing else. A list of
/// every key was a list nobody read to the end (issue 30).
#[test]
fn the_help_modal_names_the_place_and_its_keys() {
    let (_r, mut app) = make_app();
    app.handle_key(key('?'));
    assert!(matches!(app.mode, Mode::Help(_)));

    let rows = drawn_rows(&mut app);
    let at = |needle: &str| {
        rows.iter()
            .position(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} missing from help"))
    };
    // The plan pane's own keys, then the keys that work anywhere, then
    // getting about at the bottom. Movement is one run of rows.
    let anywhere = at("anywhere");
    let moving = at("moving");
    assert!(at("the plan pane") < anywhere);
    assert!(anywhere < moving, "acting reads before moving");
    assert!(at("press any key") > moving);
    for k in [
        "switch group",
        "next / previous hunk",
        "top / bottom",
        "tab",
    ] {
        assert!(at(k) > moving, "{k:?} is a movement key");
    }
    assert!(
        (anywhere..moving).contains(&at("ctrl-c")),
        "quitting works anywhere"
    );
    // The diff pane's keys are not the plan pane's answer.
    for absent in ["start a line selection", "mark this hunk's class"] {
        assert!(
            !rows.iter().any(|r| r.contains(absent)),
            "{absent:?} is a diff-pane key: {rows:?}"
        );
    }
    for prose in [
        "reading the panes",
        "plan row",
        "no -/+ columns",
        "floats a map",
    ] {
        assert!(
            !rows.iter().any(|r| r.contains(prose)),
            "{prose:?} is a legend line and should not be in the help modal"
        );
    }
}

/// The diff pane gets the diff pane's keys.
#[test]
fn the_help_modal_follows_the_reader_into_the_diff() {
    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;
    app.handle_key(key('?'));
    let rows = drawn_rows(&mut app);
    let has = |needle: &str| rows.iter().any(|r| r.contains(needle));
    assert!(has("the diff pane"));
    assert!(has("move over rows"), "j/k says what it does in THIS pane");
    assert!(!has("switch group"), "that is the plan pane's j/k");
}

/// A selection open is the place, not the pane it is in.
#[test]
fn the_help_modal_answers_for_an_open_selection() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    while !matches!(app.rows[app.cursor].kind, RowKind::Diff(_)) {
        app.handle_key(key('j'));
    }
    app.handle_key(key('v'));
    app.handle_key(key('?'));
    let rows = drawn_rows(&mut app);
    assert!(rows.iter().any(|r| r.contains("a line selection")));
    assert!(rows.iter().any(|r| r.contains("extend the selection")));
}

/// Inside a modal, the keys that work in the review behind it do not work —
/// so help does not name them.
#[test]
fn help_inside_a_modal_names_that_modal_only() {
    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    app.handle_key(key('?'));

    let rows = drawn_rows(&mut app);
    assert!(rows.iter().any(|r| r.contains("the file list")));
    assert!(
        !rows.iter().any(|r| r.contains("anywhere")),
        "those keys do not work in a modal: {rows:?}"
    );
    // And the list is still there when help closes.
    app.handle_key(key('x'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
}

/// `?` in the findings list comes back to the findings list, on the entry the
/// reader had selected.
#[test]
fn help_over_the_findings_list_gives_the_list_back() {
    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;
    while !matches!(app.rows[app.cursor].kind, RowKind::Diff(_)) {
        app.handle_key(key('j'));
    }
    app.handle_key(key('c'));
    for ch in "one".chars() {
        app.handle_key(key(ch));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(key('F'));
    let Mode::Findings { selected, .. } = &app.mode else {
        panic!("the list should be open");
    };
    let was = *selected;

    app.handle_key(key('?'));
    assert!(matches!(app.mode, Mode::Help(_)));
    let rows = drawn_rows(&mut app);
    assert!(rows.iter().any(|r| r.contains("the findings list")));

    app.handle_key(key('j'));
    let Mode::Findings { selected, .. } = &app.mode else {
        panic!("the list should be back");
    };
    assert_eq!(
        *selected, was,
        "the key that closed help is not a key in it"
    );
}

/// `y` copies from the list, as `P` sends from it: the list is where the
/// reader sees what is not yet on the request.
#[test]
fn y_copies_the_summary_from_the_findings_list() {
    let (_r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('c'));
    for ch in "off by one".chars() {
        app.handle_key(key(ch));
    }
    app.handle_key(ctrl('s'));

    app.handle_key(key('F'));
    let effects = app.handle_key(key('y'));
    let [Effect::CopySummary(text)] = &effects[..] else {
        panic!("y in the list should copy: {effects:?}");
    };
    assert!(text.contains("off by one"), "{text:?}");
    assert!(
        matches!(app.mode, Mode::Findings { .. }),
        "and the list stays open"
    );
}

/// The help modal is keys a reader can press. The wheel is not one of them.
#[test]
fn the_help_modal_does_not_name_the_mouse() {
    let (_r, mut app) = make_app();
    app.handle_key(key('?'));
    let rows = drawn_rows(&mut app);
    assert!(
        !rows.iter().any(|r| r.contains("mouse")),
        "the mouse is not a key: {rows:?}"
    );
}

/// One key that quits from anywhere, the composer included.
#[test]
fn ctrl_c_quits_from_every_mode() {
    let (_r, mut app) = make_app();
    assert_eq!(app.handle_key(ctrl('c')), vec![Effect::Quit]);

    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    assert_eq!(app.handle_key(ctrl('c')), vec![Effect::Quit]);

    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    while !matches!(app.rows[app.cursor].kind, RowKind::Diff(_)) {
        app.handle_key(key('j'));
    }
    app.handle_key(key('c'));
    for ch in "half a thought".chars() {
        app.handle_key(key(ch));
    }
    assert_eq!(
        app.handle_key(ctrl('c')),
        vec![Effect::Quit],
        "a draft in the box is not a reason to be stuck"
    );
}

#[test]
fn a_float_keeps_the_themes_ground_rather_than_the_terminals() {
    use differential_engine::config::ThemeName;
    let (_r, mut app) = make_app();
    app.set_theme(Theme::named(ThemeName::SolarizedLight));
    let ground = Some(Theme::named(ThemeName::SolarizedLight).bg);
    let buf = buffer_of(&app);

    let cells = || (0..40u16).flat_map(|y| (0..100u16).map(move |x| (x, y)));

    // Nothing anywhere falls back to the terminal's own background.
    let stray: Vec<String> = cells()
        .filter(|&(x, y)| buf[(x, y)].style().bg == Some(ratatui::style::Color::Reset))
        .map(|(x, y)| format!("({x},{y})"))
        .take(5)
        .collect();
    assert!(stray.is_empty(), "cells left to the terminal: {stray:?}");

    // And the float itself is on the ground rather than a hole in it.
    let row = (0..40u16)
        .find(|&y| {
            (0..100u16)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .contains("files in")
        })
        .expect("the group map float is not on screen");
    assert!(
        (0..100u16).any(|x| buf[(x, row)].style().bg == ground),
        "the float carries none of the theme's ground"
    );
}

/// The cursor's bar sits just inside the frame on EVERY selectable row — a
/// header, a fold and a boundary have no line-number block to brighten, and
/// the row tint alone was too faint to find.
#[test]
fn the_cursor_bar_shows_on_rows_that_have_no_gutter() {
    // The column just inside the detail pane's left border, at width 100.
    const BAR_X: usize = 41;
    type Case = (&'static str, fn(&RowKind) -> bool);
    let cases: [Case; 2] = [
        ("a context boundary", |k| {
            matches!(k, RowKind::ContextEdge { .. })
        }),
        ("a hunk header", |k| matches!(k, RowKind::HunkHeader { .. })),
    ];
    let (_r, mut app) = app_with_a_long_file();
    for (name, pred) in cases {
        put_cursor_on(&mut app, pred);
        let rows = drawn_rows(&mut app);
        let y = cursor_screen_row(&app) as usize;
        assert_eq!(
            rows[y].chars().nth(BAR_X),
            Some('▌'),
            "no cursor bar on {name}: {:?}",
            rows[y]
        );
    }

    // A fold row: the skim group's remainder, which no fixture above has.
    let (_r, mut app) = make_app();
    app.handle_key(key('j'));
    put_cursor_on(&mut app, |k| *k == RowKind::Fold);
    let rows = drawn_rows(&mut app);
    let y = cursor_screen_row(&app) as usize;
    assert_eq!(
        rows[y].chars().nth(BAR_X),
        Some('▌'),
        "no cursor bar on a fold row: {:?}",
        rows[y]
    );
}

/// The footer is the tallies, then the keys of where the reader is standing,
/// then `? help`. `q` is not among them: `?` is where it is written down
/// (issue 30).
#[test]
fn the_footer_is_pills_then_the_keys_of_this_place() {
    let (_r, mut app) = make_app();
    let footer = |app: &mut App| drawn_rows(app).last().expect("no footer").clone();

    let plan = footer(&mut app);
    assert!(
        plan.contains("classes reviewed") && plan.contains("0 findings(F)"),
        "the tallies must still be there, and the findings pill names its key: {plan:?}"
    );
    assert!(
        plan.trim_end().ends_with("? help"),
        "`? help` sits against the right edge: {plan:?}"
    );
    for k in ["enter open", "space reviewed", "f tree"] {
        assert!(plan.contains(k), "the plan pane's keys: {plan:?}");
    }
    assert!(
        !plan.contains("q quit"),
        "`q` lives in the help modal now: {plan:?}"
    );

    // The diff pane is a different place, and says so.
    app.focus = Focus::Detail;
    let diff = footer(&mut app);
    for k in ["c note", "space reviewed", "v select"] {
        assert!(diff.contains(k), "the diff pane's keys: {diff:?}");
    }
    assert!(
        !diff.contains("enter open"),
        "that was the plan pane: {diff:?}"
    );

    // A pill, not a run of grey words: the tally sits on the pill's fill.
    let backend = ratatui::backend::TestBackend::new(100, 40);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buf = terminal.backend().buffer().clone();
    let (_, fill) = theme().pill();
    assert!(
        (0..100u16).any(|x| buf[(x, 39)].bg == fill),
        "the tallies must wear the pill's fill"
    );
}

/// A selection is what the next key acts on, so the footer names its keys
/// rather than the pane's.
#[test]
fn the_footer_follows_a_selection_and_a_modal() {
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    while !matches!(app.rows[app.cursor].kind, RowKind::Diff(_)) {
        app.handle_key(key('j'));
    }
    app.handle_key(key('v'));
    let selecting = screen(&app, 120, 30)[29].clone();
    for k in ["selecting 1 line", "j/k extend", "c note", "esc drop"] {
        assert!(selecting.contains(k), "{k:?} missing: {selecting:?}");
    }

    // A modal names its own keys, so the window footer only points at help.
    app.handle_key(key('v'));
    app.handle_key(key('f'));
    let listing = drawn_rows(&mut app).last().expect("no footer").clone();
    assert!(listing.trim_end().ends_with("? help"), "{listing:?}");
    assert!(
        !listing.contains("c note"),
        "the diff's keys do not work here: {listing:?}"
    );

    // A box the caret owns has no key for `?` at all.
    app.handle_key(KeyCode::Esc.into());
    app.handle_key(key('c'));
    let composing = drawn_rows(&mut app).last().expect("no footer").clone();
    assert!(
        !composing.contains("? help"),
        "`?` is a character in the composer: {composing:?}"
    );
}

/// A narrow terminal keeps the tallies and the way to the full list.
#[test]
fn a_narrow_footer_drops_the_keys_before_the_tallies() {
    let (_r, app) = make_app();
    let narrow = screen(&app, 46, 20)[19].clone();
    assert!(narrow.contains("classes reviewed"), "{narrow:?}");
    assert!(narrow.trim_end().ends_with("? help"), "{narrow:?}");
    assert!(
        !narrow.contains("enter open"),
        "no room for them: {narrow:?}"
    );
}

/// The window footer's keys are buttons too, as a modal's are.
#[test]
fn a_click_on_a_footer_key_presses_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let panes = layout(SCREEN, DEFAULT_PLAN_COLS);
    let (hints, x0) = app.status_hints(panes.status);
    let help = hints.last().expect("`? help` is always there");
    assert!(!help.presses.is_empty());
    let x = x0 + u16::try_from(hints_width(&hints[..hints.len() - 1])).unwrap();

    app.handle_mouse(click(x, panes.status.y));
    assert!(
        matches!(app.mode, Mode::Help(_)),
        "a click on `? help` opens it"
    );
}

/// Standing ON a hunk's header lights its pill's leading cell in the hunk's
/// own accent. The cursor's bar sits in that same column, so drawing it there
/// would repaint green (reviewed) or muted cyan (foreign) as plain cyan — and
/// say the hunk was neither.
#[test]
fn the_cursor_on_a_hunk_header_keeps_the_hunks_own_accent() {
    let (_r, mut app) = app_with_a_long_file();
    // Reviewed, so the accent is green: an unread own hunk wears the cursor's
    // own cyan already, and the collision would prove nothing.
    let header = put_cursor_on(&mut app, |k| matches!(k, RowKind::HunkHeader { .. }));
    app.handle_key(key(' '));
    app.cursor = header;
    let buf = buffer_of(&app);
    let y = cursor_screen_row(&app);
    assert_eq!(buf[(41, y)].symbol(), "▌", "no marker on the header");
    assert_eq!(
        buf[(41, y)].style().fg,
        Some(theme().reviewed_fg),
        "the header's leading cell must keep the hunk's accent"
    );
}

/// `z` acts on the pane it is pressed in. `self.cursor` is a DIFF row wherever
/// the focus is, so a press in the file tree used to open whatever the diff's
/// cursor happened to be parked on.
#[test]
fn z_in_the_file_tree_folds_the_tree_not_the_diff() {
    use differential_tui::app::TreeKind;
    let (_r, mut app) = make_app();
    switch_left_pane(&mut app);
    assert_eq!(app.view_mode, ViewMode::Files);

    // Park the diff's cursor on a boundary row, then press `z` in the tree.
    let boundary = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::ContextEdge { .. }));
    if let Some(b) = boundary {
        app.cursor = b;
    }
    let rows_before = app.rows.len();
    let tree_before = app.tree.len();

    app.focus = Focus::Groups;
    app.selected_file = app
        .tree
        .iter()
        .position(|e| matches!(e.kind, TreeKind::Dir { .. }))
        .expect("a directory row");
    app.handle_key(key('z'));

    assert!(
        app.tree.len() < tree_before,
        "the directory should have folded"
    );
    assert_eq!(
        app.rows.len(),
        rows_before,
        "the diff should not have moved"
    );
}

/// A control has to say how to work it — but a screenful of bands each naming
/// the same key is a wall. The key shows on the cursor's row only.
#[test]
fn a_boundary_names_its_key_on_the_cursors_row_only() {
    let (_r, mut app) = app_with_a_long_file();
    let pos = put_cursor_on(&mut app, |k| matches!(k, RowKind::ContextEdge { .. }));
    let rows = drawn_rows(&mut app);
    let here = cursor_screen_row(&app) as usize;
    assert!(
        rows[here].contains("lines hidden") && rows[here].contains("z shows"),
        "the cursor's band must name its key: {:?}",
        rows[here]
    );

    // Every other band says what it hides and nothing more.
    let elsewhere: Vec<&String> = rows
        .iter()
        .enumerate()
        .filter(|(y, r)| *y != here && r.contains("lines hidden"))
        .map(|(_, r)| r)
        .collect();
    assert!(!elsewhere.is_empty(), "the fixture needs a second band");
    for row in elsewhere {
        assert!(
            !row.contains("z shows"),
            "a band off the cursor names no key: {row:?}"
        );
    }

    // The key follows the label rather than sitting out at the pane's edge.
    let text = &rows[here];
    let label = text.find("lines hidden").expect("the label");
    let key = text.find("z shows").expect("the key");
    assert!(key > label, "the key follows the label: {text:?}");
    assert!(
        key - label < 24,
        "the key should sit with the label, not a screen away: {text:?}"
    );

    // The band still fills the pane: the hint eats padding, not width.
    let width = text.chars().count();
    app.cursor = app
        .rows
        .iter()
        .enumerate()
        .position(|(i, r)| i != pos && r.kind.selectable())
        .expect("another selectable row");
    let after = drawn_rows(&mut app);
    assert_eq!(
        after[here].chars().count(),
        width,
        "showing the key must not change the row's width"
    );
}

/// A context boundary is a control, and its band carries its own colour the
/// whole way across — so the row tint that marks the cursor elsewhere never
/// showed through it. On the cursor's row the band lightens instead.
#[test]
fn a_context_boundary_band_lightens_under_the_cursor() {
    let (_r, mut app) = app_with_a_long_file();
    let pos = put_cursor_on(&mut app, |k| matches!(k, RowKind::ContextEdge { .. }));
    let y = cursor_screen_row(&app);
    let lit = row_backgrounds(&mut app, y);
    assert!(
        lit.contains(&theme().hint_cursor_bg),
        "the band under the cursor must lighten: {lit:?}"
    );

    // The same row, with the cursor elsewhere, keeps the muted band.
    app.cursor = app
        .rows
        .iter()
        .enumerate()
        .position(|(i, r)| i != pos && r.kind.selectable())
        .expect("another selectable row");
    let muted = row_backgrounds(&mut app, y);
    assert!(
        !muted.contains(&theme().hint_cursor_bg),
        "only the cursor's band lightens: {muted:?}"
    );
}

/// A note stays on the line it was written on when the layout changes.
///
/// A modification is TWO rows in unified — the removed line and the added one
/// — and ONE in split. A note written on the removed half anchors to the old
/// side, and in split there was no old-side row to hold it, so it fell back to
/// the hunk's header on every `s`.
#[test]
fn a_note_survives_the_diff_layout_it_was_written_in() {
    let r = TestRepo::new();
    // 18 lines inserted at the top, so old and new numbers differ throughout.
    let body = |lead: usize, change: &str| -> Vec<u8> {
        let mut out = String::new();
        for i in 1..=lead {
            out.push_str(&format!("inserted{i} = {i}\n"));
        }
        for i in 1..=40 {
            if i == 30 {
                out.push_str(change);
            } else {
                out.push_str(&format!("keep{i} = {i}\n"));
            }
        }
        out.into_bytes()
    };
    r.write("src/f.rs", &body(0, "target = 1\n"));
    r.commit_all("base");
    r.write("src/f.rs", &body(18, "target = 2\n"));
    r.commit_all("head");

    let backend = skim_first_backend();
    let mut app = open_app_with_opts(&r, &backend, ".dfr-layout-note-store", laid_out(false));
    app.focus = Focus::Detail;

    // The two halves of the modification, in the unified layout.
    let halves: Vec<usize> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.line
                .as_ref()
                .is_some_and(|l| l.text.starts_with("target"))
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(halves.len(), 2, "a modification is two rows in unified");
    let removed = app.rows[halves[0]].line.clone().unwrap();
    assert_eq!(removed.side, "old", "the first half is the removed line");

    // Write a note on the removed half, then switch layout.
    app.cursor = halves[0];
    app.handle_key(key('c'));
    app.handle_paste("about the old line");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let anchor = app.session.findings()[0].anchor.clone();
    assert_eq!((anchor.side.as_str(), anchor.line), ("old", removed.line));

    let sits_on_its_line = |app: &App| {
        let at = app
            .rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::Finding(..)))
            .expect("a note row");
        app.rows[at - 1]
            .line
            .as_ref()
            .is_some_and(|l| l.holds(&anchor.side, anchor.end_line))
    };
    assert!(sits_on_its_line(&app), "unified: the note left its line");
    app.handle_key(key('s'));
    assert!(
        sits_on_its_line(&app),
        "split: the note should still sit on the line it annotates"
    );
    app.handle_key(key('s'));
    assert!(sits_on_its_line(&app), "and back again");
}

// ---------------------------------------------------------------- chrome

/// A message answers "what did that key just do". The next key is when the
/// answer stops being wanted — and 35 places wrote one while a single place
/// cleared it, so every one-off message was the footer's content for good.
#[test]
fn a_message_lasts_exactly_one_keypress() {
    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;

    // `v` on a row that is not a line refuses, and says so.
    let not_a_line = app
        .rows
        .iter()
        .position(|r| r.line.is_none() && r.kind.selectable())
        .expect("a selectable row that is not a line");
    app.cursor = not_a_line;
    app.handle_key(key('v'));
    assert_eq!(app.status, "move onto a line first");

    // Any next key clears it, including one that does nothing at all.
    app.handle_key(key('!'));
    assert!(
        app.status.is_empty(),
        "the message should be gone: {:?}",
        app.status
    );
}

/// The file list had no scroll at all: the surplus files were built, dropped
/// off the bottom of the box, and the cursor walked into rows that were not on
/// screen.
#[test]
fn the_file_list_scrolls_to_keep_the_selection_on_screen() {
    use differential_tui::app::Mode;
    let (_r, mut app) = app_with_many_files();
    app.focus = Focus::Detail;
    // Small enough that the list cannot fit — the box is the entry count plus
    // a border pair, capped by the body.
    app.set_viewport(Viewport {
        detail_rows: 2,
        detail_cols: 58,
        plan_rows: 2,
        body_rows: 4,
        ..Viewport::default()
    });
    app.handle_key(key('f'));

    let n = match &app.mode {
        Mode::FileList { entries, .. } => entries.len(),
        _ => panic!("f should open the file list"),
    };
    assert!(n > 2, "this fixture needs more files than the box is tall");

    for _ in 0..n {
        app.handle_key(key('j'));
    }
    match &app.mode {
        Mode::FileList {
            selected, scroll, ..
        } => {
            assert_eq!(*selected, n - 1, "j should reach the last file");
            assert!(*scroll > 0, "the window has to have moved");
            assert!(
                *selected >= *scroll && *selected - *scroll < 2,
                "the selection must be inside the drawn window: \
                 selected {selected}, scroll {scroll}"
            );
        }
        _ => panic!("the file list should still be open"),
    }
}

/// A repo with more changed files than a short modal can show at once, all in
/// one group, so `f` lists them all.
fn app_with_many_files() -> (TestRepo, App) {
    let r = TestRepo::new();
    let names: Vec<String> = (0..8).map(|i| format!("src/file{i}.rs")).collect();
    for n in &names {
        r.write(n, b"fn go() { old() }\n");
    }
    r.commit_all("base");
    for n in &names {
        r.write(n, b"fn go() { new() }\n");
    }
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<&str> = ids.iter().map(String::as_str).collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group("Everything", "focus", &all)
        )
    });
    let app = open_app_with(&r, &backend, ".dfr-many-store");
    (r, app)
}

/// A repo whose one changed file sits under a long run of directories.
fn app_with_a_deep_path() -> (TestRepo, App) {
    let deep = "src/one/two/three/four/five/six/seven/module.rs";
    let r = TestRepo::new();
    r.write(deep, b"fn a() { old() }\nfn b() { same() }\n");
    r.commit_all("base");
    r.write(deep, b"fn a() { new() }\nfn b() { same() }\n");
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<&str> = ids.iter().map(String::as_str).collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group("Everything", "focus", &all)
        )
    });
    let app = open_app_with(&r, &backend, ".dfr-deep-store");
    (r, app)
}

/// The file list drew paths whole against a fixed box, so a deep one ran off
/// the border and took the file NAME with it — the one part worth reading.
#[test]
fn the_file_list_keeps_the_name_however_deep_the_path() {
    use differential_tui::app::Mode;
    // Deeper than any box the pane can hold at 100 columns.
    let deep = "src/alpha/bravo/charlie/delta/echo/foxtrot/golf/hotel/india/juliet/\
kilo/lima/mike/november/oscar/papa/quebec/distinctive_name.rs";
    let shallow = "src/near.rs";
    let r = TestRepo::new();
    for f in [deep, shallow] {
        r.write(f, b"fn go() { old() }\n");
    }
    r.commit_all("base");
    for f in [deep, shallow] {
        r.write(f, b"fn go() { new() }\n");
    }
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<&str> = ids.iter().map(String::as_str).collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group("Everything", "focus", &all)
        )
    });
    let mut app = open_app_with(&r, &backend, ".dfr-deepfile-store");
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));

    let rows = drawn_rows(&mut app);
    assert!(
        rows.iter().any(|l| l.contains("distinctive_name.rs")),
        "the name has to survive the directories above it"
    );
    // A path that fits is shown whole — the cut is a last resort, not a style.
    assert!(
        rows.iter().any(|l| l.contains(shallow)),
        "a path with room to spare keeps every segment"
    );
    // And the deep one says it was cut rather than pretending to be complete.
    let cut = rows
        .iter()
        .find(|l| l.contains("distinctive_name.rs"))
        .unwrap();
    assert!(cut.contains('…'), "the cut has to be visible: {cut:?}");
    assert!(
        !cut.contains("src/alpha"),
        "the leading directories are what goes: {cut:?}"
    );
}

/// `{:<width$}` pads and never truncates, so a long path used to overflow its
/// column and leave the note three or four words. The name and the line number
/// identify a finding; the leading directories do not.
#[test]
fn the_findings_list_cuts_the_path_at_its_head_not_the_note() {
    let (_r, mut app) = app_with_a_deep_path();
    note_on(
        &mut app,
        |r| r.line.is_some(),
        "this note has to survive a very deep path",
    );
    app.handle_key(key('F'));

    // The pane behind the modal draws the note too, so pick the modal's row:
    // the one that carries a location beside the note.
    let row = drawn_rows(&mut app)
        .into_iter()
        .find(|l| l.contains("this note has to survive") && l.contains("module.rs:"))
        .expect("the list should show the note beside its location");
    assert!(row.contains("…/"), "the head of the path goes: {row:?}");
    assert!(
        !row.contains("src/one/two"),
        "the leading directories are what gets cut: {row:?}"
    );
}

/// `v` used to write a passing message into the same grey slot that
/// "finding saved" uses. One describes a MODE the reader is in; the other
/// describes something already over.
#[test]
fn a_selection_wears_a_pill_for_as_long_as_it_lasts() {
    let (_r, mut app) = app_with_a_long_file();
    let start = app
        .rows
        .windows(2)
        .position(|w| {
            w.iter()
                .all(|r| r.line.as_ref().is_some_and(|l| l.side == "new"))
        })
        .expect("two new-side rows in a row");
    app.cursor = start;
    app.focus = Focus::Detail;

    let footer = |app: &mut App| drawn_rows(app).last().expect("no footer").clone();
    assert!(
        !footer(&mut app).contains("selecting"),
        "no pill before a selection"
    );

    app.handle_key(key('v'));
    assert!(
        footer(&mut app).contains("selecting 1 line"),
        "the pill counts the lines: {:?}",
        footer(&mut app)
    );
    app.handle_key(key('j'));
    assert!(
        footer(&mut app).contains("selecting 2 lines"),
        "the count follows the run: {:?}",
        footer(&mut app)
    );

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !footer(&mut app).contains("selecting"),
        "the pill goes when the mode does"
    );
}

/// `y` hands the loop the text and nothing else. The projection itself is the
/// ENGINE's, so `dfr findings --summary` prints exactly what `y` copies.
#[test]
fn y_carries_the_same_summary_the_cli_prints() {
    let (r, mut app) = make_app();
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('c'));
    app.handle_paste("off by one");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let effects = app.handle_key(key('y'));
    match effects.first() {
        Some(Effect::CopySummary(text)) => {
            // Pinned against the FORMAT, not against the call that produced
            // it. `Effect::CopySummary` carries `session.findings_summary()`,
            // so comparing the two compared one call with another and held
            // whatever either printed. `dfr findings --summary` prints these
            // same bytes, which is the parity this test is named for.
            assert_eq!(*text, "- src/main.txt:1: off by one\n", "{text:?}");
        }
        other => panic!("expected a copied summary, got {other:?}"),
    }
    assert!(
        !r.root.join(".dfr-test-store/summary.md").exists(),
        "`y` writes no file — the summary is a command away, not a path"
    );
}

/// A long line is cut at the pane edge and `w` is not always the answer — in
/// split the column is half a pane, which is what #74 is about.
#[test]
fn l_shifts_the_diff_pane_and_h_brings_it_back() {
    let (_r, mut app) = app_with_a_long_line();
    let head = "Soft wrap exists because";
    let tail = "runs off the right edge";
    let before = wrapped_pane(&mut app);
    assert!(
        before.iter().any(|r| r.contains(head)),
        "the line starts at the left edge: {before:#?}"
    );

    // when - the pane shifts right far enough to reach the middle of the line
    for _ in 0..6 {
        app.handle_key(key('l'));
    }
    let after = wrapped_pane(&mut app);

    // then - what was off the edge is on screen, and the head has gone
    assert!(
        after.iter().any(|r| r.contains(tail)),
        "the middle of the line should be readable: {after:#?}"
    );
    assert!(!after.iter().any(|r| r.contains(head)));

    // and - `h` walks back, `0` goes home in one
    app.handle_key(key('h'));
    assert!(!wrapped_pane(&mut app).iter().any(|r| r.contains(head)));
    app.handle_key(key('0'));
    assert_eq!(wrapped_pane(&mut app), before, "0 restores the left edge");
}

/// The line-number cell is what the cursor block lands in, so it never moves.
#[test]
fn the_line_numbers_do_not_move_when_the_pane_shifts() {
    let (_r, mut app) = app_with_a_long_line();
    // The footer is not the pane: it grows a pill as the pane shifts, which
    // is the point of the pill.
    let gutters = |app: &mut App| -> Vec<String> {
        let rows = wrapped_pane(app);
        rows[..rows.len() - 1]
            .iter()
            .map(|r| r.chars().take(11).collect())
            .collect()
    };
    let before = gutters(&mut app);
    for _ in 0..4 {
        app.handle_key(key('l'));
    }
    assert_eq!(gutters(&mut app), before, "the gutter is pinned");
}

/// Past the widest line there is nothing left to reveal, so the pane stops
/// rather than walking into blank space.
#[test]
fn the_shift_stops_at_the_widest_line() {
    let (_r, mut app) = app_with_a_long_line();
    // Draw once so the pane's width is measured before any key is handled,
    // and so the keys land in the pane they act on.
    wrapped_pane(&mut app);
    for _ in 0..60 {
        app.handle_key(key('l'));
    }
    let pane = wrapped_pane(&mut app);
    assert!(
        pane.iter().any(|r| r.contains("not there to read.")),
        "the end of the line must still be on screen: {pane:#?}"
    );
}

/// `w` and the shift answer the same question, and only one of them can be
/// right at a time. A press that silently did nothing would read as a key
/// that does not work.
#[test]
fn soft_wrap_and_the_shift_do_not_both_apply() {
    let (_r, mut app) = app_with_a_long_line();
    app.handle_key(key('w'));
    let wrapped = wrapped_pane(&mut app);

    app.handle_key(key('l'));

    // The footer carries the refusal, so compare the pane and not the bar.
    let after = wrapped_pane(&mut app);
    assert_eq!(after[..after.len() - 1], wrapped[..wrapped.len() - 1]);
    assert!(app.status.contains("soft wrap is on"), "{}", app.status);
}

/// The other order. `w` while shifted homed the pane, because a wrapped row
/// reads no offset — but the offset stayed in the model, so the footer went on
/// claiming a shift that was not happening.
#[test]
fn turning_wrap_on_drops_the_shift() {
    let (_r, mut app) = app_with_a_long_line();
    wrapped_pane(&mut app);
    for _ in 0..3 {
        app.handle_key(key('l'));
    }
    // The pill leads the footer, which `wrapped_pane` slices away.
    let footer = |app: &mut App| drawn_rows(app).last().cloned().unwrap_or_default();
    let shifted = footer(&mut app);
    assert!(
        shifted.contains("cols"),
        "the footer should say where the pane stands: {shifted:?}"
    );

    // when
    app.handle_key(key('w'));

    // then - the pill goes with the shift it described
    let after = footer(&mut app);
    assert!(
        !after.contains("cols"),
        "the pill must go when the shift does: {after:?}"
    );
    // and - turning wrap off again leaves the pane at the left edge
    app.handle_key(key('w'));
    let off = wrapped_pane(&mut app);
    assert!(
        off.iter().any(|r| r.contains("Soft wrap exists because")),
        "the pane should be home, not where it was: {off:#?}"
    );
}

/// Both columns move together, or the two sides stop being comparable — which
/// is the whole reason to read a diff side by side.
#[test]
fn a_split_row_shifts_both_halves() {
    let (_r, mut app) = app_with_a_long_line();
    wrapped_pane(&mut app);
    app.handle_key(key('s')); // that helper opens unified, so switch to split
    // 58 columns of content, so the separator sits at 28 and the halves are
    // everything either side of it.
    let halves = |app: &mut App| -> Vec<(String, String)> {
        let rows = wrapped_pane(app);
        rows[..rows.len() - 1]
            .iter()
            .map(|r| {
                let c: Vec<char> = r.chars().collect();
                (c[..28].iter().collect(), c[29..].iter().collect())
            })
            .collect()
    };
    let before = halves(&mut app);
    app.handle_key(key('l'));
    let after = halves(&mut app);
    assert_ne!(after, before, "the pane must move at all");

    // Every row that moved moved on BOTH sides, or on neither.
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(
            b.0 == a.0,
            b.1 == a.1,
            "one half moved without the other: {b:?} -> {a:?}"
        );
    }
}

/// The bound moves when the rows do. A split column is half a pane, so it
/// reaches further along a line than a unified one; coming back to unified
/// with the shift left where it was would leave the pane blank.
#[test]
fn the_shift_comes_back_inside_a_wider_layout() {
    let (_r, mut app) = app_with_a_long_line();
    wrapped_pane(&mut app);
    app.handle_key(key('s'));
    for _ in 0..60 {
        app.handle_key(key('l'));
    }
    app.handle_key(key('s'));
    let pane = wrapped_pane(&mut app);
    assert!(
        pane.iter().any(|r| r.contains("not there to read.")),
        "the end of the line must still be on screen after s: {pane:#?}"
    );
}

/// The pane at a known width, as rows of text.
fn wrapped_pane(app: &mut App) -> Vec<String> {
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 22,
        detail_cols: 58,
        plan_rows: 22,
        body_rows: 24,
        ..Viewport::default()
    });
    drawn_rows(app)
        .into_iter()
        // The detail pane's CONTENT only: 40 columns of plan pane, then the
        // two panes' borders.
        .map(|row| row.chars().skip(41).take(58).collect())
        .collect()
}

#[test]
fn w_wraps_a_long_line_and_leaves_the_row_count_alone() {
    // given - a paragraph wider than the pane
    let (_r, mut app) = app_with_a_long_line();
    let before = wrapped_pane(&mut app);
    assert!(
        before.iter().any(|r| r.contains("...")),
        "the line should be cut before w is pressed: {before:#?}"
    );
    let rows = app.rows.len();
    let cursor = app.cursor;

    // when
    app.handle_key(key('w'));
    let after = wrapped_pane(&mut app);

    // then - the tail is on screen, on its own line
    assert!(
        after
            .iter()
            .any(|r| r.contains("simply not there to read.")),
        "the end of the paragraph should be visible: {after:#?}"
    );
    assert!(!after.iter().any(|r| r.contains("...")));

    // and - a wrapped line is still ONE row, and the cursor has not moved
    assert_eq!(app.rows.len(), rows, "wrapping must not change row COUNT");
    assert_eq!(app.cursor, cursor);
}

#[test]
fn a_continuation_line_has_no_line_number_of_its_own() {
    // given
    let (_r, mut app) = app_with_a_long_line();

    // when
    app.handle_key(key('w'));
    let pane = wrapped_pane(&mut app);

    // then - the number cell is blank on the continuation, and the same width,
    // so the content still starts in one column.
    let first = pane
        .iter()
        .position(|r| r.contains("Soft wrap exists"))
        .expect("the wrapped line");
    let head = &pane[first][..11];
    let tail = &pane[first + 1][..11];
    assert!(head.trim().ends_with('3'), "line number missing: {head:?}");
    assert_eq!(tail, " ".repeat(11), "continuation should have no number");
}

#[test]
fn prose_wraps_whether_or_not_w_is_on() {
    // given - a group description longer than the pane, and w untouched
    let (_r, mut app) = app_with_a_long_line();

    // when
    let pane = wrapped_pane(&mut app);

    // then - the description reads to its end, indented under itself
    assert!(pane.iter().any(|r| r.contains("This group rewrites")));
    let tail = pane
        .iter()
        .find(|r| r.contains("pane is wide."))
        .expect("the end of the description");
    assert!(
        tail.starts_with("   "),
        "continuation lost its indent: {tail:?}"
    );
}

#[test]
fn a_wrapped_note_stays_inside_its_rail() {
    // given - a note longer than the pane
    let (_r, mut app) = app_with_a_long_line();
    app.focus = Focus::Detail;
    app.handle_key(key('c'));
    for ch in "a note about this line that is longer than the pane is wide".chars() {
        app.handle_key(key(ch));
    }
    app.handle_key(ctrl('s'));

    // when
    let pane = wrapped_pane(&mut app);

    // then - every line of the panel keeps the rail
    let rails: Vec<&String> = pane.iter().filter(|r| r.contains('▍')).collect();
    assert!(rails.len() >= 2, "the note should wrap: {pane:#?}");
    assert!(
        rails.last().is_some_and(|r| r.contains("is wide")),
        "the end of the note should sit behind the rail: {rails:#?}"
    );
}

#[test]
fn the_scroll_budget_counts_screen_lines_not_rows() {
    // given - a long file in a pane narrow enough that its lines wrap
    let (_r, mut app) = app_with_a_long_file();
    app.focus = Focus::Detail;
    app.set_viewport(Viewport {
        detail_rows: 12,
        detail_cols: 24,
        plan_rows: 12,
        body_rows: 14,
        ..Viewport::default()
    });
    app.handle_key(key('w'));

    // when - the cursor goes to the bottom
    app.handle_key(key('G'));

    // then - the window really does hold wrapped rows, so counting rows and
    // counting lines are different numbers here
    let rows = app.cursor + 1 - app.scroll();
    let lines: usize = (app.scroll()..=app.cursor).map(|i| app.row_height(i)).sum();
    assert!(
        lines > rows,
        "the window should hold a wrapped row: {lines} lines over {rows} rows"
    );

    // and - it is the LINES that fit the pane. Counting rows would overflow it.
    assert!(lines <= 12, "the view must hold the cursor: {lines} lines");
    assert!(app.scroll() <= app.cursor);
}

#[test]
fn the_wrap_choice_is_persisted() {
    // given
    let (r, mut app) = app_with_a_long_line();
    assert!(!app.session.wrap().unwrap_or(false));

    // when
    app.handle_key(key('w'));

    // then - it is on disk, not just in the model
    assert_eq!(app.session.wrap(), Some(true));
    let state = std::fs::read_to_string(r.root.join(".dfr-wrap-store/state.json")).unwrap();
    assert!(state.contains("\"wrap\": true"), "{state}");
}

// ------------------------------------------------------------ forge threads

mod forge_threads {
    //! The forge's review threads in the reviewer (ADR 0029, spec/forge.md):
    //! fetched on a worker, drawn under their lines, replied to, resolved.

    use std::sync::{Arc, Mutex};

    use differential_engine::forge::{
        Batch, Forge, ForgeError, ForgeKind, Published, RemoteComment, RemoteThread, Request, Sent,
    };
    use differential_tui::app::ForgeLink;

    use super::*;

    /// A forge that answers from memory. Allowed where a fake git is not: the
    /// forge is a `dyn` seam chosen at run time, and nothing here is an
    /// invariant that would compare the fake with itself.
    struct FakeForge {
        threads: Mutex<Vec<RemoteThread>>,
        resolved: Mutex<Vec<(String, bool)>>,
        /// What `request` reports as the head. The review's own by default;
        /// a test moves it to stand for a push since the review was built.
        head: Mutex<String>,
        published: Mutex<Vec<Batch>>,
        /// When set, `threads` fails: a refetch that breaks after a publish.
        fail_threads: Mutex<bool>,
        /// When set, `publish` records the batch and creates nothing: a
        /// send the forge silently dropped.
        swallow: Mutex<bool>,
        /// When set, `publish` creates nothing and reports a failure after
        /// the fact: a forge that broke part-way.
        stop_part_way: Mutex<bool>,
        /// When set, `publish` refuses outright with a long error: a forge
        /// that said no, at length.
        refuse: Mutex<bool>,
    }

    impl FakeForge {
        fn new(threads: Vec<RemoteThread>, head: &str) -> Arc<Self> {
            Arc::new(FakeForge {
                threads: Mutex::new(threads),
                resolved: Mutex::new(Vec::new()),
                head: Mutex::new(head.to_string()),
                published: Mutex::new(Vec::new()),
                fail_threads: Mutex::new(false),
                swallow: Mutex::new(false),
                stop_part_way: Mutex::new(false),
                refuse: Mutex::new(false),
            })
        }
    }

    impl Forge for FakeForge {
        fn kind(&self) -> ForgeKind {
            ForgeKind::Github
        }
        fn whoami(&self) -> Result<String, ForgeError> {
            Ok("me".into())
        }
        fn request(&self, _id: Option<&str>) -> Result<Request, ForgeError> {
            Ok(Request {
                head: self.head.lock().unwrap().clone(),
                ..github_request("7")
            })
        }
        fn threads(&self, _req: &Request) -> Result<Vec<RemoteThread>, ForgeError> {
            if *self.fail_threads.lock().unwrap() {
                return Err(ForgeError::NoRequest("the forge is down".into()));
            }
            Ok(self.threads.lock().unwrap().clone())
        }
        /// Takes everything: each new comment becomes a thread of its own on
        /// the same line, each reply a comment in its thread — what the real
        /// forge's refetch would then show, marker read and stripped like a
        /// real adapter does. Answers with NOTHING, as GitHub did the first
        /// time this ran for real: the reviewer must learn what landed from
        /// the threads, not from this answer.
        fn publish(&self, _req: &Request, batch: &Batch) -> Result<Sent, ForgeError> {
            use differential_engine::forge::strip_marker;
            if *self.refuse.lock().unwrap() {
                return Err(ForgeError::Failed {
                    command: "fakeforge api --method POST projects/:id/merge_requests/1/discussions/abc/notes --input -".into(),
                    code: Some(1),
                    output: "{\"message\":\"400 Bad Request - the forge's own words, at length, which the footer could never hold\"}".into(),
                });
            }
            self.published.lock().unwrap().push(batch.clone());
            if *self.swallow.lock().unwrap() {
                return Ok(Sent::default());
            }
            if *self.stop_part_way.lock().unwrap() {
                // The forge took the batch and then broke before it could say
                // what it made of it. Nothing to name; the error to report.
                return Ok(Sent {
                    published: Vec::new(),
                    threads: None,
                    failed: Some(ForgeError::NoRequest("the forge fell over after".into())),
                });
            }
            let mut threads = self.threads.lock().unwrap();
            for c in &batch.comments {
                let (tid, cid) = (format!("T-{}", c.finding), format!("C-{}", c.finding));
                let mut t = thread(&tid, &cid);
                t.comments.truncate(1);
                let (body, finding) = strip_marker(&c.body);
                t.comments[0].body = body;
                t.comments[0].finding = finding;
                t.comments[0].author = "me".into();
                t.line = Some(c.line);
                threads.push(t);
            }
            for r in &batch.replies {
                let cid = format!("C-{}", r.finding);
                if let Some(t) = threads.iter_mut().find(|t| t.id == r.thread) {
                    let (body, finding) = strip_marker(&r.body);
                    t.comments.push(RemoteComment {
                        id: cid.clone(),
                        author: "me".into(),
                        created: "2026-09-04T09:00:00Z".into(),
                        body,
                        finding,
                    });
                }
            }
            Ok(Sent::default())
        }
        fn set_resolved(
            &self,
            _req: &Request,
            thread: &str,
            resolved: bool,
        ) -> Result<(), ForgeError> {
            self.resolved
                .lock()
                .unwrap()
                .push((thread.to_string(), resolved));
            Ok(())
        }
        fn edit_comment(
            &self,
            _req: &Request,
            thread: &str,
            comment: &str,
            body: &str,
        ) -> Result<(), ForgeError> {
            use differential_engine::forge::strip_marker;
            let mut threads = self.threads.lock().unwrap();
            let c = threads
                .iter_mut()
                .find(|t| t.id == thread)
                .and_then(|t| t.comments.iter_mut().find(|c| c.id == comment))
                .ok_or_else(|| ForgeError::NoRequest("no such comment".into()))?;
            let (text, finding) = strip_marker(body);
            c.body = text;
            c.finding = finding;
            Ok(())
        }
        fn delete_comment(
            &self,
            _req: &Request,
            thread: &str,
            comment: &str,
        ) -> Result<(), ForgeError> {
            let mut threads = self.threads.lock().unwrap();
            let t = threads
                .iter_mut()
                .find(|t| t.id == thread)
                .ok_or_else(|| ForgeError::NoRequest("no such thread".into()))?;
            t.comments.retain(|c| c.id != comment);
            threads.retain(|t| !t.comments.is_empty());
            Ok(())
        }
    }

    /// A thread on the one changed line of `src/main.txt`.
    fn thread(id: &str, root_comment: &str) -> RemoteThread {
        RemoteThread {
            id: id.to_string(),
            resolved: false,
            outdated: false,
            path: "src/main.txt".into(),
            side: "new".into(),
            line: Some(1),
            start_line: None,
            line_text: Some("fn main() { run_with_retries(3) }".into()),
            anchor: None,
            comments: vec![
                remote_comment(root_comment, "alice", "2026-09-03T20:53:12Z", "why three?"),
                remote_comment(
                    &format!("{root_comment}-r"),
                    "bob",
                    "2026-09-03T21:00:00Z",
                    "it was two before",
                ),
            ],
        }
    }

    /// Wait for the one forge call that is out to land. The fake answers at
    /// once; the wait is for the worker thread to be scheduled.
    fn settle(app: &mut App) {
        for _ in 0..400 {
            if app.poll_forge() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the forge call never landed");
    }

    /// `make_app` with a fake forge linked and its threads fetched, parked on
    /// the group that holds `src/main.txt`.
    fn app_with_threads(threads: Vec<RemoteThread>) -> (TestRepo, App, Arc<FakeForge>) {
        let (r, mut app) = make_app();
        let fake = FakeForge::new(threads, &app.session.doc().source.head);
        app.link_forge(ForgeLink {
            forge: Arc::clone(&fake) as Arc<dyn Forge>,
            request: github_request("7"),
        });
        app.start_fetch();
        assert!(app.syncing());
        settle(&mut app);
        assert!(!app.syncing());
        assert!(app.status.contains("review thread"), "{}", app.status);
        select_group_of(&mut app, "src/main.txt");
        (r, app, fake)
    }

    /// Park the reviewer on the group that holds `path`, by pressing keys in
    /// the plan pane: the plan's order is the engine's to decide.
    fn select_group_of(app: &mut App, path: &str) {
        let want = app
            .groups()
            .iter()
            .position(|g| {
                g.hunks
                    .iter()
                    .any(|h| app.session.doc().hunks[h.index()].file == path)
            })
            .expect("a group holds the file");
        app.focus = Focus::Groups;
        for _ in 0..app.groups().len() {
            app.handle_key(key('k'));
        }
        for _ in 0..want {
            app.handle_key(key('j'));
        }
        assert_eq!(app.selected_group, want);
        app.focus = Focus::Detail;
    }

    /// Park the cursor on the one changed line of `src/main.txt`.
    fn cursor_to_changed_line(app: &mut App) {
        app.cursor = app
            .rows
            .iter()
            .position(|r| r.line.as_ref().is_some_and(|l| l.holds("new", 1)))
            .unwrap();
    }

    fn thread_rows(app: &App, id: &str) -> Vec<usize> {
        app.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(&r.kind, RowKind::Thread { thread: t, .. } if t == id))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn threads_arrive_on_a_worker_and_sit_under_their_line() {
        let (_r, app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        assert_eq!(app.session.threads().len(), 1);

        let rows = thread_rows(&app, "T1");
        // Root header, body, a spacer, then the reply's header and body.
        assert_eq!(rows.len(), 5, "{rows:?}");
        let first = rows[0];
        let above = &app.rows[first - 1];
        assert!(matches!(above.kind, RowKind::Diff(_)));
        assert!(above.line.as_ref().unwrap().holds("new", 1));
        // A thread is a selectable row like a note, and belongs to the hunk.
        assert!(app.rows[first].kind.selectable());
        assert_eq!(app.rows[first].kind.hunk(), above.kind.hunk());
    }

    #[test]
    fn r_on_a_thread_drafts_a_reply_that_sits_under_it() {
        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        let rows = thread_rows(&app, "T1");
        app.cursor = rows[0];
        app.handle_key(key('r'));
        assert!(matches!(app.mode, Mode::Editing { reply_to: Some(ref t), .. } if t == "T1"));
        app.handle_paste("agreed, two was enough");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let reply = app
            .session
            .findings()
            .iter()
            .find(|f| f.reply_to.as_deref() == Some("T1"))
            .expect("the reply is a finding");
        assert_eq!(reply.body, "agreed, two was enough");
        assert!(app.status.contains("P publishes"), "{}", app.status);

        // Directly after the thread's last row, stepped in like a reply.
        let last_thread = *thread_rows(&app, "T1").last().unwrap();
        let note = &app.rows[last_thread + 1];
        assert!(matches!(&note.kind, RowKind::Finding(id, _) if id == &reply.id));
        // And nowhere else: a reply is not also a loose note on the line.
        let notes = app
            .rows
            .iter()
            .filter(|r| matches!(&r.kind, RowKind::Finding(id, _) if id == &reply.id))
            .count();
        assert_eq!(notes, 1);
        // `y` counts it: it is not yet on the request.
        assert!(app.findings_summary().contains("agreed, two was enough"));
    }

    #[test]
    fn a_comment_body_renders_as_markdown() {
        let mut t = thread("T1", "C1");
        t.comments[0].body = "**bold** and `code`".into();
        let (_r, app, _fake) = app_with_threads(vec![t]);
        let pairs: Vec<(ratatui::style::Style, String)> = thread_rows(&app, "T1")
            .iter()
            .flat_map(|&i| match &app.rows[i].content {
                differential_tui::rows::RowContent::Unified(h) => h.pairs.clone(),
                _ => Vec::new(),
            })
            .collect();
        assert!(
            pairs.iter().any(|(s, txt)| txt.contains("bold")
                && s.add_modifier.contains(ratatui::style::Modifier::BOLD)),
            "bold span: {pairs:?}"
        );
        assert!(
            pairs
                .iter()
                .any(|(s, txt)| txt == "code" && s.fg == Some(theme().hint_fg)),
            "code span: {pairs:?}"
        );
        // The markers themselves are gone.
        let text: String = pairs.iter().map(|(_, t)| t.as_str()).collect();
        assert!(!text.contains("**") && !text.contains('`'), "{text}");
    }

    #[test]
    fn x_resolves_the_thread_through_the_forge_and_dims_it_once_answered() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        app.cursor = thread_rows(&app, "T1")[0];
        app.handle_key(key('x'));
        assert!(app.syncing(), "the forge is asked on a worker");
        assert!(
            !app.session.threads()[0].resolved,
            "not before the forge says so"
        );
        settle(&mut app);
        assert_eq!(
            fake.resolved.lock().unwrap().as_slice(),
            &[("T1".to_string(), true)]
        );
        assert!(app.session.threads()[0].resolved);
        assert_eq!(app.status, "thread resolved");
        // The header row now says so.
        let first = thread_rows(&app, "T1")[0];
        let text: String = match &app.rows[first].content {
            differential_tui::rows::RowContent::Unified(half) => {
                half.pairs.iter().map(|(_, t)| t.as_str()).collect()
            }
            _ => String::new(),
        };
        assert!(text.contains("resolved"), "{text}");

        // And back.
        app.cursor = first;
        app.handle_key(key('x'));
        settle(&mut app);
        assert!(!app.session.threads()[0].resolved);
        assert_eq!(app.status, "thread reopened");
    }

    #[test]
    fn x_off_a_thread_says_what_it_is_for() {
        let (_r, mut app, _fake) = app_with_threads(vec![]);
        app.cursor = app.rows.iter().position(|r| r.line.is_some()).unwrap();
        app.handle_key(key('x'));
        assert!(!app.syncing());
        assert!(app.status.contains("x resolves"), "{}", app.status);
    }

    /// The plain text of a row.
    fn row_text(app: &App, i: usize) -> String {
        match &app.rows[i].content {
            differential_tui::rows::RowContent::Unified(h) => {
                h.pairs.iter().map(|(_, t)| t.as_str()).collect()
            }
            _ => String::new(),
        }
    }

    #[test]
    fn a_resolved_thread_is_collapsed_until_z_opens_it() {
        let mut t = thread("T1", "C1");
        t.resolved = true;
        let (_r, mut app, _fake) = app_with_threads(vec![t]);
        // Collapsed: one header row, naming the count and the key that opens it.
        let rows = thread_rows(&app, "T1");
        assert_eq!(rows.len(), 1, "collapsed to its header");
        let header = row_text(&app, rows[0]);
        assert!(
            header.contains("resolved") && header.contains("z to open"),
            "{header}"
        );
        assert!(!header.contains("why three?"), "body is withheld: {header}");

        // z opens it: the body shows.
        app.cursor = rows[0];
        app.handle_key(key('z'));
        assert_eq!(app.status, "thread expanded");
        let open = thread_rows(&app, "T1");
        assert!(open.len() > 1, "expanded shows the body");
        assert!(
            open.iter()
                .any(|&i| row_text(&app, i).contains("why three?")),
            "the body is shown when open"
        );

        // z again collapses it.
        app.cursor = thread_rows(&app, "T1")[0];
        app.handle_key(key('z'));
        assert_eq!(app.status, "thread collapsed");
        assert_eq!(thread_rows(&app, "T1").len(), 1);
    }

    #[test]
    fn r_fetches_again_and_the_answer_replaces_the_cache() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        fake.threads.lock().unwrap().push(thread("T2", "C2"));
        app.handle_key(key('R'));
        assert!(app.syncing());
        // A second call while one is out is refused, not queued.
        app.handle_key(key('R'));
        assert_eq!(app.status, "still syncing with the forge");
        settle(&mut app);
        assert_eq!(app.session.threads().len(), 2);
        assert_eq!(thread_rows(&app, "T2").len(), 5);
    }

    #[test]
    fn a_published_note_draws_as_its_fetched_twin_not_twice() {
        let (_r, mut app) = make_app();
        select_group_of(&mut app, "src/main.txt");
        cursor_to_changed_line(&mut app);
        app.handle_key(key('c'));
        app.handle_paste("why three?");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let id = app.session.findings()[0].id.clone();
        app.session
            .mark_published(&[Published {
                finding: id.clone(),
                thread: "T1".into(),
                comment: "C1".into(),
                url: None,
            }])
            .unwrap();
        // Published but not yet fetched: the note still shows.
        app.rebuild_rows();
        assert!(
            app.rows
                .iter()
                .any(|r| matches!(&r.kind, RowKind::Finding(f, _) if f == &id))
        );

        let fake = FakeForge::new(vec![thread("T1", "C1")], &app.session.doc().source.head);
        app.link_forge(ForgeLink {
            forge: fake as Arc<dyn Forge>,
            request: github_request("7"),
        });
        app.start_fetch();
        settle(&mut app);
        assert!(
            !app.rows
                .iter()
                .any(|r| matches!(&r.kind, RowKind::Finding(f, _) if f == &id)),
            "the thread is the note now"
        );
        assert_eq!(thread_rows(&app, "T1").len(), 5);
        // Published: not in the summary either.
        assert!(!app.findings_summary().contains("why three?"));
    }

    /// Standing on somebody else's thread, the footer names the keys that
    /// work on a thread — and the help modal answers for it too (issue 30).
    #[test]
    fn the_footer_and_help_follow_the_cursor_onto_a_thread() {
        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        app.focus = Focus::Detail;
        let row = *thread_rows(&app, "T1").first().expect("no thread rows");
        app.cursor = row;

        let footer = screen(&app, 120, 30)[29].clone();
        for k in ["r reply", "x resolve"] {
            assert!(footer.contains(k), "{k:?} missing: {footer:?}");
        }
        assert!(
            !footer.contains("c note"),
            "a thread is the forge's: {footer:?}"
        );

        app.handle_key(key('?'));
        let rows = screen(&app, 120, 30);
        assert!(rows.iter().any(|r| r.contains("a review thread")));
        assert!(rows.iter().any(|r| r.contains("not yours")));
    }

    #[test]
    fn the_footer_counts_threads_only_on_a_request_review() {
        let footer = |app: &mut App| -> String { screen(app, 120, 30)[29].clone() };
        let (_r, mut plain) = make_app();
        assert!(!footer(&mut plain).contains("thread"));

        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        let line = footer(&mut app);
        assert!(line.contains("1 thread"), "{line}");
        assert!(!line.contains("syncing"), "{line}");
        app.handle_key(key('R'));
        assert!(footer(&mut app).contains("syncing"));
        settle(&mut app);
    }

    // ---------------------------------------------------------------- publish

    /// Write a note on the changed line and a reply under the thread.
    fn draft_two(app: &mut App) {
        cursor_to_changed_line(app);
        app.handle_key(key('c'));
        app.handle_paste("three is a magic number");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.cursor = thread_rows(app, "T1")[0];
        app.handle_key(key('r'));
        app.handle_paste("agreed");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn a_click_on_the_publish_footers_y_sends() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        sized(&mut app);
        draft_two(&mut app);
        app.handle_key(key('P'));
        assert!(matches!(app.mode, Mode::Publish { .. }));
        // `publish_lines` with nothing excluded: a blank, the sentence, a
        // blank and the footer.
        let area = publish_area(layout(SCREEN, DEFAULT_PLAN_COLS).body, 4);
        let row = footer_row(area);
        // `publish_footer`: `y publishes`, then the clause about any other key.
        let hints = publish_footer();
        let yes = on_hint(&hints, 0, row.x, row);
        let keep = on_hint(&hints, 1, row.x, row);

        // A click in the box, off the footer, is nothing.
        let (x, y) = on_line(area, 1);
        app.handle_mouse(click(x, y));
        assert!(matches!(app.mode, Mode::Publish { .. }));

        app.handle_mouse(click(keep.0, keep.1));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.status, "nothing published");
        assert!(fake.published.lock().unwrap().is_empty());

        app.handle_key(key('P'));
        app.handle_mouse(click(yes.0, yes.1));
        assert!(app.syncing(), "the `y` sends");
        settle(&mut app);
        assert_eq!(fake.published.lock().unwrap().len(), 1);
    }

    #[test]
    fn p_shows_the_plan_and_only_y_sends_it() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        assert_eq!(app.session.unpublished().count(), 2);

        app.handle_key(key('P'));
        let Mode::Publish { plan } = &app.mode else {
            panic!("P opens the publish modal");
        };
        assert_eq!(
            (plan.batch.comments.len(), plan.batch.replies.len()),
            (1, 1)
        );
        assert!(plan.excluded.is_empty());
        // Any other key keeps it all local.
        app.handle_key(key('n'));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.status, "nothing published");
        assert!(fake.published.lock().unwrap().is_empty());
        assert!(!app.syncing());

        app.handle_key(key('P'));
        app.handle_key(key('y'));
        assert!(app.syncing(), "the send is a worker call");
        assert!(app.status.starts_with("publishing 2"), "{}", app.status);
        settle(&mut app);
        assert_eq!(fake.published.lock().unwrap().len(), 1);
        assert_eq!(app.status, "published 2 comments");
        // Both findings carry their upstream address; neither is open work.
        assert_eq!(app.session.unpublished().count(), 0);
        assert!(app.session.findings().iter().all(|f| f.upstream.is_some()));
        assert_eq!(app.findings_summary().trim(), "(no open findings)");
        // The refetch shows the twins: the note is a thread now, and the
        // reply is the thread's third comment.
        assert!(
            !app.rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::Finding(..)))
        );
        assert_eq!(app.session.threads().len(), 2);
        assert_eq!(app.session.thread("T1").unwrap().comments.len(), 3);
        // A second P has nothing left to send.
        app.handle_key(key('P'));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(
            app.status.starts_with("nothing to publish"),
            "{}",
            app.status
        );
    }

    #[test]
    fn a_moved_head_refuses_the_whole_publish() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        *fake.head.lock().unwrap() = "f".repeat(40);
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert!(
            fake.published.lock().unwrap().is_empty(),
            "nothing was sent"
        );
        assert!(
            app.status.starts_with("nothing published"),
            "{}",
            app.status
        );
        assert!(
            matches!(&app.mode, Mode::Notice { text, .. } if text.contains("moved to ffffffffffff")),
            "the whole answer is on screen"
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            app.session.unpublished().count(),
            2,
            "still the reader's to send"
        );
    }

    #[test]
    fn a_note_the_diff_cannot_hold_stays_local_and_is_named() {
        let (_r, mut app, fake) = app_with_threads(vec![]);
        // Far from any change: line 40 of a one-line file.
        let h = app
            .session
            .doc()
            .hunks
            .iter()
            .position(|h| h.file == "src/main.txt")
            .unwrap();
        app.session
            .add_finding(
                h,
                Some(differential_engine::review_state::Lines {
                    side: "new".into(),
                    start: 40,
                    end: 40,
                    start_text: String::new(),
                    end_text: String::new(),
                }),
                "far away".into(),
            )
            .unwrap();
        app.rebuild_rows();
        app.handle_key(key('P'));
        assert!(
            matches!(app.mode, Mode::Normal),
            "nothing sendable: no modal"
        );
        assert!(app.status.contains("cannot hold"), "{}", app.status);

        // With one sendable note beside it, the modal lists the one that stays.
        draft_one_note(&mut app);
        app.handle_key(key('P'));
        let Mode::Publish { plan } = &app.mode else {
            panic!("P opens the publish modal");
        };
        assert_eq!(plan.batch.len(), 1);
        assert_eq!(plan.excluded.len(), 1);
        assert_eq!(plan.excluded[0].lines, "40");
        let screen = screen(&app, 120, 30);
        assert!(
            screen
                .iter()
                .any(|l| l.contains("1 new comment go to the pull request")),
            "{screen:#?}"
        );
        assert!(
            screen.iter().any(|l| l.contains("1 stay local")),
            "{screen:#?}"
        );
        assert!(
            screen.iter().any(|l| l.contains("src/main.txt:40")),
            "{screen:#?}"
        );
        app.handle_key(key('y'));
        settle(&mut app);
        assert_eq!(fake.published.lock().unwrap()[0].comments.len(), 1);
        assert_eq!(
            app.session.unpublished().count(),
            1,
            "the far note is still open"
        );
    }

    /// Write one note on the changed line and nothing under the thread.
    fn draft_one_note(app: &mut App) {
        cursor_to_changed_line(app);
        app.handle_key(key('c'));
        app.handle_paste("on the change");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }

    #[test]
    fn p_off_a_request_review_says_so() {
        let (_r, mut app) = make_app();
        app.handle_key(key('P'));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.status, "this review is not of a pull request");
    }

    // ---------------------------------------------------------- the F list

    #[test]
    fn the_findings_list_holds_threads_too_and_enter_reaches_one() {
        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_one_note(&mut app);
        app.cursor = 0;
        app.handle_key(key('F'));
        let Mode::Findings { entries, .. } = &app.mode else {
            panic!("F opens the list");
        };
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].thread, "notes first");
        assert!(entries[1].thread);
        assert_eq!(entries[1].id, "T1");
        assert!(
            entries[1].body.starts_with("alice: why three?"),
            "{}",
            entries[1].body
        );
        assert_eq!(entries[1].at, "src/main.txt:1");

        // The rule between the sections, and the title, are drawn.
        let screen = screen(&app, 120, 30);
        assert!(
            screen.iter().any(|l| l.contains("review threads")),
            "{screen:#?}"
        );
        assert!(
            screen
                .iter()
                .any(|l| l.contains("findings · 1 · threads · 1")),
            "{screen:#?}"
        );
        assert!(
            screen.iter().any(|l| l.contains("P publish")),
            "{screen:#?}"
        );

        // enter on the thread lands on its rows.
        app.handle_key(key('j'));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(
            matches!(&app.rows[app.cursor].kind, RowKind::Thread { thread: t, .. } if t == "T1")
        );

        // dd on a thread in the list refuses, as it does in the diff.
        app.handle_key(key('F'));
        app.handle_key(key('j'));
        app.handle_key(key('d'));
        app.handle_key(key('d'));
        assert_eq!(app.session.threads().len(), 1);
        assert!(app.status.contains("not your comment"), "{}", app.status);
    }

    #[test]
    fn p_from_the_findings_list_publishes_everything_unpublished() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        app.handle_key(key('F'));
        app.handle_key(key('P'));
        assert!(
            matches!(app.mode, Mode::Publish { .. }),
            "the same float as P in the diff"
        );
        app.handle_key(key('y'));
        settle(&mut app);
        assert_eq!(fake.published.lock().unwrap()[0].len(), 2);
        assert_eq!(app.session.unpublished().count(), 0);
    }

    #[test]
    fn a_lost_publish_answer_is_recovered_from_the_markers() {
        // The fake answers the publish with nothing, as the real forge did.
        // The refetched threads carry each finding's marker, so the reviewer
        // still knows what landed, hides the notes and sends nothing twice.
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert_eq!(
            app.status, "published 2 comments",
            "counted from the threads"
        );
        assert!(app.session.findings().iter().all(|f| f.upstream.is_some()));
        assert!(
            !app.rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::Finding(..)))
        );
        // The published finding is listed once, as its thread.
        app.handle_key(key('F'));
        let Mode::Findings { entries, .. } = &app.mode else {
            panic!("F opens the list");
        };
        assert!(
            entries.iter().all(|e| e.thread),
            "{:?}",
            entries.iter().map(|e| &e.body).collect::<Vec<_>>()
        );
        assert_eq!(entries.len(), 2);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // And the bodies the forge holds carry the marker, not the reader.
        let sent = &fake.published.lock().unwrap()[0];
        assert!(sent.comments[0].body.contains("<!-- differential:finding "));
        assert!(
            !app.session.thread("T1").unwrap().comments[2]
                .body
                .contains("differential:finding")
        );
        // A second publish has nothing to send.
        app.handle_key(key('P'));
        assert!(
            app.status.starts_with("nothing to publish"),
            "{}",
            app.status
        );
    }

    #[test]
    fn capital_d_clears_local_notes_only_and_counts_only_those() {
        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        published_and_parked(&mut app);
        draft_one_note(&mut app);
        // One local note, two published (one a reply), one foreign thread.
        assert_eq!(app.session.unpublished().count(), 1);
        assert_eq!(app.session.findings().len(), 3);

        app.handle_key(key('F'));
        app.handle_key(key('D'));
        let screen = screen(&app, 120, 30).join("\n");
        assert!(
            screen.contains("delete this note? (2 on the request stay)"),
            "{screen}"
        );
        assert!(
            !screen.contains("findings?"),
            "threads and published notes are not counted"
        );

        app.handle_key(key('y'));
        assert_eq!(app.session.unpublished().count(), 0);
        assert_eq!(
            app.session.findings().len(),
            2,
            "the published records stay"
        );
        assert_eq!(app.session.threads().len(), 2, "threads are untouched");
        assert!(app.status.contains("1 note deleted"), "{}", app.status);
        assert!(
            app.status.contains("2 on the request kept"),
            "{}",
            app.status
        );

        // Nothing local left: D says so instead of asking.
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.handle_key(key('F'));
        app.handle_key(key('D'));
        assert!(matches!(
            &app.mode,
            Mode::Findings {
                confirming: false,
                ..
            }
        ));
        assert!(
            app.status.starts_with("nothing local to delete"),
            "{}",
            app.status
        );
    }

    #[test]
    fn a_failed_refetch_after_a_publish_keeps_what_was_sent() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        // The fake creates the comments and then cannot list them back. It
        // answers the publish with nothing, so only the markers in a refetch
        // could confirm — and the refetch fails. Nothing must be lost or
        // resent: the comments are on the request.
        *fake.fail_threads.lock().unwrap() = true;
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert!(
            app.status.contains("threads could not be fetched back"),
            "{}",
            app.status
        );
        assert!(app.status.contains("R to retry"), "{}", app.status);
        assert_eq!(fake.published.lock().unwrap().len(), 1);
        // The next fetch reconciles by marker and the plan is empty.
        *fake.fail_threads.lock().unwrap() = false;
        app.handle_key(key('R'));
        settle(&mut app);
        assert_eq!(app.session.unpublished().count(), 0);
        app.handle_key(key('P'));
        assert!(
            app.status.starts_with("nothing to publish"),
            "{}",
            app.status
        );
        assert_eq!(fake.published.lock().unwrap().len(), 1, "sent once");
    }

    #[test]
    fn the_published_count_is_of_this_batch_not_of_every_batch_before() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        // A second note, and a forge that takes the send and creates nothing.
        draft_one_note(&mut app);
        *fake.swallow.lock().unwrap() = true;
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert!(app.status.starts_with("published 0 of 1"), "{}", app.status);
        assert_eq!(
            app.session.unpublished().count(),
            1,
            "still the reader's to send"
        );
    }

    #[test]
    fn a_published_note_whose_twin_is_not_fetched_yet_is_still_editable() {
        let (_r, mut app, fake) = app_with_threads(vec![]);
        draft_one_note(&mut app);
        let id = app.session.findings()[0].id.clone();
        // Published, address recorded, but the reviewer has not fetched the
        // thread back: the note still draws as a note.
        let mut on_forge = my_unmarked_thread("T", "on the change");
        on_forge.comments[0].id = "C".into();
        fake.threads.lock().unwrap().push(on_forge);
        app.session
            .mark_published(&[Published {
                finding: id.clone(),
                thread: "T".into(),
                comment: "C".into(),
                url: None,
            }])
            .unwrap();
        app.rebuild_rows();
        let row = app
            .rows
            .iter()
            .position(|r| matches!(&r.kind, RowKind::Finding(f, _) if f == &id))
            .expect("drawn as a note");
        app.cursor = row;
        app.handle_key(key('c'));
        assert!(matches!(&app.mode, Mode::Editing { own: Some(own), .. } if own.comment == "C"));
        app.handle_paste(" — edited");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        settle(&mut app);
        assert_eq!(app.status, "comment rewritten on the request");
        assert_eq!(app.session.findings()[0].body, "on the change — edited");
        assert_eq!(
            fake.threads.lock().unwrap()[0].root().unwrap().body,
            "on the change — edited"
        );
        // And dd from the note's row.
        app.cursor = app
            .rows
            .iter()
            .position(|r| matches!(&r.kind, RowKind::Finding(f, _) if f == &id))
            .unwrap();
        app.handle_key(key('d'));
        app.handle_key(key('d'));
        assert!(
            matches!(&app.mode, Mode::DeleteComment { own } if own.finding.as_deref() == Some(id.as_str()))
        );
        app.handle_key(key('y'));
        settle(&mut app);
        assert!(app.session.findings().is_empty());
        assert!(fake.threads.lock().unwrap().is_empty());
    }

    #[test]
    fn a_forge_that_stops_part_way_is_reported_and_what_landed_is_kept() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_two(&mut app);
        *fake.stop_part_way.lock().unwrap() = true;
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert!(app.status.contains("stopped part-way"), "{}", app.status);
        assert!(app.status.contains("P to send the rest"), "{}", app.status);
        assert!(matches!(&app.mode, Mode::Notice { text, .. } if text.contains("fell over")));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // Nothing was named and the fake made nothing, so both notes are
        // still the reader's — and the next P offers exactly them.
        assert_eq!(app.session.unpublished().count(), 2);
        *fake.stop_part_way.lock().unwrap() = false;
        app.handle_key(key('P'));
        assert!(matches!(&app.mode, Mode::Publish { plan } if plan.batch.len() == 2));
    }

    #[test]
    fn a_threads_comment_wraps_whatever_w_says() {
        let long = "this comment runs on and on well past the width of any pane a reviewer \
                    would open, and every word of it has to be readable end to end";
        let mut t = thread("T1", "C1");
        t.comments.truncate(1);
        t.comments[0].body = long.into();
        let (_r, app, _fake) = app_with_threads(vec![t]);
        assert!(!app.wrap_on_for_test(), "soft wrap is off for code");
        let screen = screen(&app, 100, 30);
        let first = screen
            .iter()
            .position(|l| l.contains("this comment runs on"))
            .expect("the comment starts");
        assert!(
            screen[first + 1..first + 4]
                .iter()
                .any(|l| l.contains("end to end")),
            "the tail is on a following line, not cut: {:?}",
            &screen[first..first + 4]
        );
        // The continuation lines keep the rail, so the panel stays a panel.
        assert!(screen[first + 1].contains('▍'), "{}", screen[first + 1]);
    }

    #[test]
    fn a_long_note_in_the_composer_stays_above_the_key_footer() {
        let (_r, mut app, _fake) = app_with_threads(vec![]);
        cursor_to_changed_line(&mut app);
        app.handle_key(key('c'));
        let text: Vec<String> = (1..=30)
            .map(|i| format!("line {i} of a long note"))
            .collect();
        app.handle_paste(&text.join("\n"));
        let screen = screen(&app, 120, 30);
        let footer = screen
            .iter()
            .position(|l| l.contains("enter") && l.contains("save"))
            .expect("the key footer is drawn");
        assert!(
            !screen[footer].contains("of a long note"),
            "the footer row carries no text: {}",
            screen[footer]
        );
        assert!(
            screen[footer - 1].contains("of a long note"),
            "the row above the footer is the note's last visible line: {}",
            screen[footer - 1]
        );
        // The box grew to the body: more of the note is visible than the old
        // ten-row box could show.
        let shown = screen
            .iter()
            .filter(|l| l.contains("of a long note"))
            .count();
        assert!(shown > 6, "{shown} lines shown");
    }

    #[test]
    fn r_replies_and_c_edits_within_your_own_thread() {
        // The reader's root, published from here; a reply by someone else;
        // the cursor on that reply.
        let (_r, mut app, fake) = app_with_threads(vec![]);
        draft_one_note(&mut app);
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        let mine = app.session.findings()[0].id.clone();
        let tid = format!("T-{mine}");
        {
            let mut threads = fake.threads.lock().unwrap();
            let t = threads.iter_mut().find(|t| t.id == tid).unwrap();
            t.comments.push(RemoteComment {
                id: "theirs".into(),
                author: "bob".into(),
                created: "2026-09-08T09:00:00Z".into(),
                body: "are you sure?".into(),
                finding: None,
            });
        }
        app.handle_key(key('R'));
        settle(&mut app);
        let rows = thread_rows(&app, &tid);
        assert_eq!(
            rows.len(),
            5,
            "root header, body, spacer, reply header, reply body"
        );
        // On their reply: not mine, so r replies (c would error).
        app.cursor = rows[3];
        app.handle_key(key('r'));
        assert!(
            matches!(&app.mode, Mode::Editing { reply_to: Some(t), own: None, .. } if t == &tid),
            "a reply to the thread, not an edit of my root"
        );
        app.handle_paste("yes, because");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let reply = app
            .session
            .findings()
            .iter()
            .find(|f| f.reply_to.as_deref() == Some(tid.as_str()))
            .expect("filed as a reply");
        assert_eq!(reply.body, "yes, because");
        // Drawn stepped in under their reply, not as a loose note on the line.
        let last = *thread_rows(&app, &tid).last().unwrap();
        assert!(matches!(&app.rows[last + 1].kind, RowKind::Finding(id, _) if id == &reply.id));
        // And on my own root, c edits.
        app.cursor = thread_rows(&app, &tid)[0];
        app.handle_key(key('c'));
        assert!(
            matches!(&app.mode, Mode::Editing { own: Some(o), .. } if o.comment == format!("C-{mine}"))
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // Publishing the reply sends it into the thread, not as a new comment.
        app.handle_key(key('P'));
        let Mode::Publish { plan } = &app.mode else {
            panic!("P offers the reply");
        };
        assert_eq!(
            (plan.batch.comments.len(), plan.batch.replies.len()),
            (0, 1)
        );
        assert_eq!(plan.batch.replies[0].thread, tid);
    }

    #[test]
    fn a_long_line_in_the_composer_wraps_instead_of_running_off() {
        let (_r, mut app, _fake) = app_with_threads(vec![]);
        cursor_to_changed_line(&mut app);
        app.handle_key(key('c'));
        let long = (1..=30)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        app.handle_paste(&long);
        let screen = screen(&app, 100, 30);
        let first = screen
            .iter()
            .position(|l| l.contains("word1 "))
            .expect("the note starts");
        assert!(
            !screen[first].contains("word30"),
            "one row cannot hold it all at this width: {}",
            screen[first]
        );
        assert!(
            screen[first + 1..first + 4]
                .iter()
                .any(|l| l.contains("word30")),
            "the tail is on a following row, not off the edge: {:?}",
            &screen[first..first + 4]
        );
    }

    #[test]
    fn a_forge_refusal_is_shown_in_full_not_cut_by_the_footer() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        draft_one_note(&mut app);
        *fake.refuse.lock().unwrap() = true;
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(&mut app);
        let Mode::Notice { title, text } = &app.mode else {
            panic!("a refusal opens the notice, mode is elsewhere");
        };
        assert_eq!(title, "nothing published");
        assert!(text.contains("exited with Some(1)"), "{text}");
        assert!(text.contains("the forge's own words, at length"), "{text}");
        assert!(
            app.status.contains("details are on screen"),
            "{}",
            app.status
        );

        // Drawn whole: the forge's words reach the screen, wrapped.
        let screen = screen(&app, 100, 30).join("\n");
        assert!(screen.contains("nothing published"), "{screen}");
        assert!(screen.contains("could never hold"), "{screen}");

        // Any key closes it; the note is still the reader's to send.
        app.handle_key(key('j'));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.session.unpublished().count(), 1);
    }

    // ------------------------------------------------------ your own comment

    /// A thread by the reader, as the forge reports it, with no marker: one
    /// sent before markers existed, or written on the forge's own page.
    fn my_unmarked_thread(id: &str, body: &str) -> RemoteThread {
        let mut t = thread(id, &format!("{id}-root"));
        t.comments.truncate(1);
        t.comments[0].author = "me".into();
        t.comments[0].body = body.into();
        t
    }

    #[test]
    fn a_comment_by_you_with_no_marker_is_yours_by_author() {
        let (_r, mut app, fake) =
            app_with_threads(vec![my_unmarked_thread("M1", "mine, from the web")]);
        app.cursor = thread_rows(&app, "M1")[0];
        assert_eq!(app.session.findings().len(), 0, "no local record at all");

        // c edits it on the forge; the cache follows, no record is invented.
        app.handle_key(key('c'));
        assert!(matches!(&app.mode, Mode::Editing { own: Some(own), .. } if own.thread == "M1"));
        app.handle_paste(" and edited here");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        settle(&mut app);
        assert_eq!(app.status, "comment rewritten on the request");
        assert_eq!(
            app.session.thread("M1").unwrap().root().unwrap().body,
            "mine, from the web and edited here"
        );
        assert_eq!(
            fake.threads.lock().unwrap()[0].root().unwrap().body,
            "mine, from the web and edited here"
        );
        assert!(app.session.findings().is_empty());

        // dd asks, then deletes it there and here.
        app.cursor = thread_rows(&app, "M1")[0];
        app.handle_key(key('d'));
        app.handle_key(key('d'));
        assert!(matches!(&app.mode, Mode::DeleteComment { own } if own.finding.is_none()));
        app.handle_key(key('y'));
        settle(&mut app);
        assert_eq!(app.status, "comment deleted on the request");
        assert!(app.session.thread("M1").is_none());
        assert!(fake.threads.lock().unwrap().is_empty());
    }

    #[test]
    fn an_unmarked_comment_by_you_heals_the_note_it_came_from() {
        // The note was published before markers existed and its answer was
        // lost: a local note with no address, and a comment by the reader on
        // the same line with the same words and no marker.
        let (_r, mut app) = make_app();
        select_group_of(&mut app, "src/main.txt");
        draft_one_note(&mut app);
        assert_eq!(app.session.unpublished().count(), 1);
        let fake = FakeForge::new(
            vec![my_unmarked_thread("M1", "on the change")],
            &app.session.doc().source.head,
        );
        app.link_forge(ForgeLink {
            forge: fake as Arc<dyn Forge>,
            request: github_request("7"),
        });
        app.start_fetch();
        settle(&mut app);
        assert!(
            app.status.contains("1 finding found already published"),
            "{}",
            app.status
        );
        let f = &app.session.findings()[0];
        assert_eq!(
            f.upstream
                .as_ref()
                .map(|u| (u.thread.as_str(), u.comment.as_str())),
            Some(("M1", "M1-root"))
        );
        assert_eq!(
            app.session.unpublished().count(),
            0,
            "P has nothing to send"
        );
        assert!(app.session.is_twinned(f), "listed once, as the thread");
        // Not healed: a different text on the same line stays a note.
        cursor_to_changed_line(&mut app);
        app.handle_key(key('c'));
        app.handle_paste("something else");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.handle_key(key('R'));
        settle(&mut app);
        assert_eq!(app.session.unpublished().count(), 1);
    }

    /// Publish two notes and land the cursor on the header row of the thread
    /// the first one became.
    fn published_and_parked(app: &mut App) -> String {
        draft_two(app);
        app.handle_key(key('P'));
        app.handle_key(key('y'));
        settle(app);
        let mine = app
            .session
            .findings()
            .iter()
            .find(|f| f.body == "three is a magic number")
            .unwrap()
            .id
            .clone();
        let tid = format!("T-{mine}");
        app.cursor = thread_rows(app, &tid)[0];
        mine
    }

    #[test]
    fn c_on_your_own_published_comment_edits_it_on_the_forge() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        let mine = published_and_parked(&mut app);
        app.handle_key(key('c'));
        let Mode::Editing { own, reply_to, .. } = &app.mode else {
            panic!("c opens the composer");
        };
        assert_eq!(
            own.as_ref().and_then(|o| o.finding.as_deref()),
            Some(mine.as_str()),
            "a rewrite of the comment, not a reply"
        );
        assert!(reply_to.is_none());
        app.handle_paste(", or is it");
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.syncing(), "the forge first");
        settle(&mut app);
        assert_eq!(app.status, "comment rewritten on the request");
        let f = app
            .session
            .findings()
            .iter()
            .find(|f| f.id == mine)
            .unwrap();
        assert_eq!(f.body, "three is a magic number, or is it");
        assert!(f.upstream.is_some(), "still published");
        {
            let on_forge = fake.threads.lock().unwrap();
            let c = on_forge
                .iter()
                .find(|t| t.id == format!("T-{mine}"))
                .unwrap()
                .root()
                .unwrap()
                .clone();
            assert_eq!(c.body, "three is a magic number, or is it");
            assert_eq!(
                c.finding.as_deref(),
                Some(mine.as_str()),
                "the marker travelled"
            );
        }
        // And the cached thread shows the new text without a refetch.
        assert_eq!(
            app.session
                .thread(&format!("T-{mine}"))
                .unwrap()
                .root()
                .unwrap()
                .body,
            "three is a magic number, or is it"
        );
    }

    #[test]
    fn dd_on_your_own_published_comment_asks_then_deletes_it_there_and_here() {
        let (_r, mut app, fake) = app_with_threads(vec![thread("T1", "C1")]);
        let mine = published_and_parked(&mut app);
        app.handle_key(key('d'));
        app.handle_key(key('d'));
        assert!(
            matches!(&app.mode, Mode::DeleteComment { own } if own.finding.as_deref() == Some(mine.as_str()))
        );
        app.handle_key(key('n'));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.status, "nothing deleted");
        assert_eq!(app.session.findings().len(), 2);

        app.handle_key(key('d'));
        app.handle_key(key('d'));
        app.handle_key(key('y'));
        settle(&mut app);
        assert_eq!(app.status, "comment deleted on the request");
        assert!(app.session.findings().iter().all(|f| f.id != mine));
        assert!(
            app.session.thread(&format!("T-{mine}")).is_none(),
            "its thread went with it"
        );
        assert!(
            fake.threads
                .lock()
                .unwrap()
                .iter()
                .all(|t| t.id != format!("T-{mine}"))
        );
        assert_eq!(app.session.findings().len(), 1, "the reply is untouched");
    }

    #[test]
    fn someone_elses_comment_takes_r_to_reply_not_c_to_edit() {
        let (_r, mut app, _fake) = app_with_threads(vec![thread("T1", "C1")]);
        // c on a comment that is not yours edits nothing and says so.
        app.cursor = thread_rows(&app, "T1")[0];
        app.handle_key(key('c'));
        assert!(matches!(app.mode, Mode::Normal), "c opens no composer");
        assert_eq!(app.status, "not your comment · r replies · x resolves");
        // r on it opens a reply to the thread.
        app.handle_key(key('r'));
        assert!(
            matches!(&app.mode, Mode::Editing { reply_to: Some(t), own: None, .. } if t == "T1")
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        // dd on it deletes nothing and says so.
        app.cursor = thread_rows(&app, "T1")[1];
        app.handle_key(key('d'));
        app.handle_key(key('d'));
        assert!(matches!(app.mode, Mode::Normal));
        assert_eq!(app.session.threads().len(), 1, "nothing deleted");
        assert_eq!(app.status, "not your comment · r replies · x resolves");
    }
}

// ------------------------------------------------------------------ the mouse

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use differential_tui::app::{
    DEFAULT_PLAN_COLS, Hint, MIN_HALF, MIN_PANE, centered_x, clamp_cols, composer_area,
    composer_footer, divider, file_list_modal_area, findings_modal_area, findings_question,
    footer_row, half_centre, half_widths, hints_width, layout, pane_inner, publish_area,
    publish_footer, search_modal_area, split_point,
};
use differential_tui::keymap::{Action, Keymap, KeysConfig};
use ratatui::layout::Rect;

/// The screen every mouse test is measured at.
const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 100,
    height: 40,
};

fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn wheel_down(x: u16, y: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollDown, x, y)
}

fn wheel_up(x: u16, y: u16) -> MouseEvent {
    mouse(MouseEventKind::ScrollUp, x, y)
}

fn click(x: u16, y: u16) -> MouseEvent {
    mouse(MouseEventKind::Down(MouseButton::Left), x, y)
}

fn drag(x: u16, y: u16) -> MouseEvent {
    mouse(MouseEventKind::Drag(MouseButton::Left), x, y)
}

fn sized(app: &mut App) {
    app.set_area(SCREEN);
}

/// `alt-=` and `alt--`, the keys that move the divider.
fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

/// A cell inside a pane's content, a little way in from its top-left corner.
fn inside(pane: Rect) -> (u16, u16) {
    let inner = pane_inner(pane);
    (inner.x + inner.width / 2, inner.y + inner.height / 3)
}

/// Content line `line` of a framed box, a couple of cells in.
fn on_line(area: Rect, line: u16) -> (u16, u16) {
    let inner = pane_inner(area);
    (inner.x + 2, inner.y + line)
}

/// The first cell of the `n`-th hint of a footer laid out from `x0` on `row`.
/// The widths come from the hints themselves, so a reworded footer moves the
/// click with it.
fn on_hint(hints: &[Hint], n: usize, x0: u16, row: Rect) -> (u16, u16) {
    (x0 + hints_width(&hints[..n]) as u16, row.y)
}

fn next_selectable_after(app: &App, from: usize) -> usize {
    (from + 1..app.rows.len())
        .find(|&i| app.rows[i].kind.selectable())
        .expect("a selectable row below the cursor")
}

#[test]
fn a_wheel_notch_in_the_detail_pane_moves_one_row_and_focuses_it() {
    let (_r, mut app) = app_with_many_files();
    sized(&mut app);
    let (x, y) = inside(layout(SCREEN, DEFAULT_PLAN_COLS).detail);
    assert_eq!(app.focus, Focus::Groups);
    let before = app.cursor;
    let want = next_selectable_after(&app, before);

    app.handle_mouse(wheel_down(x, y));

    assert_eq!(
        app.focus,
        Focus::Detail,
        "the pane under the pointer takes focus"
    );
    assert_eq!(app.cursor, want, "one notch is one selectable row");

    let want = next_selectable_after(&app, app.cursor);
    app.handle_mouse(wheel_down(x, y));
    assert_eq!(app.cursor, want, "and the next notch is the next row");

    let here = app.cursor;
    app.handle_mouse(wheel_up(x, y));
    assert!(app.cursor < here, "a notch up is one row back");
    assert!(app.rows[app.cursor].kind.selectable());
}

#[test]
fn a_wheel_notch_in_the_plan_pane_switches_entry_and_focuses_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let (x, y) = inside(layout(SCREEN, DEFAULT_PLAN_COLS).plan);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Detail);

    app.handle_mouse(wheel_down(x, y));

    assert_eq!(app.focus, Focus::Groups);
    assert_eq!(app.selected_group, 1);

    app.handle_mouse(wheel_up(x, y));
    assert_eq!(app.selected_group, 0);
}

#[test]
fn a_click_lands_on_the_row_under_the_pointer_counting_wrapped_lines() {
    let (_r, mut app) = app_with_a_long_line();
    sized(&mut app);
    let detail = layout(SCREEN, DEFAULT_PLAN_COLS).detail;
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('w'));
    assert!(app.wrap_on_for_test());
    assert_eq!(app.scroll(), 0);

    // The paragraph is one row and several screen lines.
    let tall = (0..app.rows.len())
        .find(|&i| app.rows[i].kind.selectable() && app.row_height(i) > 1)
        .expect("a wrapped selectable row");
    let top: usize = (0..tall).map(|i| app.row_height(i)).sum();
    let height = app.row_height(tall);

    let (x, y) = on_line(detail, (top + 1) as u16);
    app.handle_mouse(click(x, y));
    assert_eq!(
        app.cursor, tall,
        "the second screen line of a wrapped row is still that row"
    );

    if let Some(below) = (tall + 1..app.rows.len()).find(|&i| app.rows[i].kind.selectable()) {
        let below_top: usize = (0..below).map(|i| app.row_height(i)).sum();
        assert!(below_top >= top + height);
        let (x, y) = on_line(detail, below_top as u16);
        app.handle_mouse(click(x, y));
        assert_eq!(
            app.cursor, below,
            "the line after the wrapped row is the next row"
        );
    }
}

#[test]
fn a_click_on_a_row_that_cannot_be_selected_leaves_the_cursor() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('j'));
    let before = app.cursor;
    assert!(!app.rows[0].kind.selectable(), "row 0 is the group header");
    assert_eq!(app.scroll(), 0);

    // Content line 0 of the pane is row 0.
    let (x, y) = on_line(layout(SCREEN, DEFAULT_PLAN_COLS).detail, 0);
    app.handle_mouse(click(x, y));

    assert_eq!(app.cursor, before);
    assert_eq!(app.focus, Focus::Detail);
}

#[test]
fn a_click_selects_a_plan_entry_and_a_second_click_enters_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    assert_eq!(app.selected_group, 0);
    // `plan_block_height`: a group's block is a title row and a counts row,
    // plus an `after:` row when it follows another group.
    let first_block = 2 + usize::from(!app.groups()[0].depends_on.is_empty());
    let (x, y) = on_line(layout(SCREEN, DEFAULT_PLAN_COLS).plan, first_block as u16);

    app.handle_mouse(click(x, y));
    assert_eq!(app.selected_group, 1);
    assert_eq!(app.focus, Focus::Groups);

    app.handle_mouse(click(x, y));
    assert_eq!(app.selected_group, 1);
    assert_eq!(
        app.focus,
        Focus::Detail,
        "a click on the selected entry is enter"
    );
}

#[test]
fn the_file_list_modal_takes_a_click_and_closes_on_one_outside() {
    let (_r, mut app) = app_with_many_files();
    sized(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('f'));
    let (area, second_row) = match &app.mode {
        Mode::FileList { entries, .. } => (
            file_list_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body, entries),
            entries[1].row_idx,
        ),
        _ => panic!("f opens the file list"),
    };
    // Nothing has scrolled, so content line 1 is entry 1.
    let (x, y) = on_line(area, 1);

    app.handle_mouse(click(x, y));
    assert!(
        matches!(app.mode, Mode::FileList { selected: 1, .. }),
        "a click selects the entry under it"
    );

    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal), "a second click jumps");
    assert!(app.cursor >= second_row);
    assert!(app.rows[app.cursor].kind.selectable());

    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    // The screen's corner is the plan pane's frame: outside any box.
    app.handle_mouse(click(0, 0));
    assert!(
        matches!(app.mode, Mode::Normal),
        "a click outside the box closes it"
    );
}

#[test]
fn help_ignores_the_wheel_and_closes_on_a_click() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let (x, y) = inside(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    app.handle_key(key('?'));
    assert!(matches!(app.mode, Mode::Help(_)));

    app.handle_mouse(wheel_down(x, y));
    assert!(matches!(app.mode, Mode::Help(_)));

    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn the_viewport_records_the_screen_the_panes_are_laid_out_on() {
    let v = Viewport::measure(SCREEN, DEFAULT_PLAN_COLS);
    assert_eq!(v.area, SCREEN);
    let panes = layout(v.area, DEFAULT_PLAN_COLS);
    assert_eq!(panes.plan.x + panes.plan.width, panes.detail.x);
    assert_eq!(panes.status.y, SCREEN.height - 1);
}

#[test]
fn the_composer_footer_buttons_save_and_cancel() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('c'));
    for ch in "off by one".chars() {
        app.handle_key(key(ch));
    }
    let area = match &app.mode {
        Mode::Editing { editor, .. } => {
            composer_area(layout(SCREEN, DEFAULT_PLAN_COLS).body, editor)
        }
        _ => panic!("c opens the composer"),
    };
    let row = footer_row(area);
    let hints = composer_footer();
    let x0 = centered_x(row, hints_width(&hints));
    // `composer_footer`: save, a rule, the newline note, a rule, cancel.
    let save = on_hint(&hints, 0, x0, row);
    let cancel = on_hint(&hints, 4, x0, row);

    // A click in the text does nothing; the caret owns the box.
    let (x, y) = on_line(area, 1);
    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Editing { .. }));

    app.handle_mouse(click(save.0, save.1));
    assert!(matches!(app.mode, Mode::Normal), "save closes the composer");
    assert_eq!(app.session.findings().len(), 1);
    assert_eq!(app.session.findings()[0].body, "off by one");

    app.handle_key(key('c'));
    for ch in "second thought".chars() {
        app.handle_key(key(ch));
    }
    app.handle_mouse(click(cancel.0, cancel.1));
    assert!(
        matches!(app.mode, Mode::Normal),
        "cancel closes the composer"
    );
    assert_eq!(app.session.findings().len(), 1, "and keeps nothing");
}

#[test]
fn the_findings_footer_buttons_jump_and_close() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('c'));
    for ch in "off by one".chars() {
        app.handle_key(key(ch));
    }
    app.handle_key(ctrl('s'));
    app.handle_key(key('g'));
    let top_row = app.cursor;

    app.handle_key(key('F'));
    assert!(matches!(app.mode, Mode::Findings { .. }));
    // One local note, no section rules, nothing on a request.
    let row = footer_row(findings_modal_area(
        layout(SCREEN, DEFAULT_PLAN_COLS).body,
        1,
        0,
    ));
    // The table's rows for the findings list: enter jump, dd delete, D clear
    // local, P publish, esc close — with a separator hint between each.
    let keys = app.modal_footer();
    let (x, y) = on_hint(&keys, 9, row.x, row);
    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal), "esc close closes");

    app.handle_key(key('F'));
    let (x, y) = on_hint(&keys, 1, row.x, row);
    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal), "enter jump closes");
    assert!(
        matches!(app.rows[app.cursor].kind, RowKind::Finding(_, _)),
        "and lands on the note"
    );
    assert_ne!(app.cursor, top_row);

    // `D` asks. The question's footer is the question, `y`, a slash, `n`.
    let asks = findings_question(1, 0);
    let clear = on_hint(&keys, 5, row.x, row);
    let yes = on_hint(&asks, 1, row.x, row);
    let no = on_hint(&asks, 3, row.x, row);
    app.handle_key(key('F'));
    app.handle_mouse(click(clear.0, clear.1));
    assert!(matches!(
        app.mode,
        Mode::Findings {
            confirming: true,
            ..
        }
    ));
    app.handle_mouse(click(no.0, no.1));
    assert!(matches!(
        app.mode,
        Mode::Findings {
            confirming: false,
            ..
        }
    ));
    assert_eq!(app.session.findings().len(), 1);
    app.handle_mouse(click(clear.0, clear.1));
    app.handle_mouse(click(yes.0, yes.1));
    assert!(app.session.findings().is_empty());
}

#[test]
fn the_floating_overviews_swallow_a_click_and_the_wheel() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let panes = layout(SCREEN, DEFAULT_PLAN_COLS);

    // The plan has focus, so the group map floats at the foot of the diff.
    assert_eq!(app.focus, Focus::Groups);
    let map = app
        .group_map_area(panes.detail)
        .expect("the group map floats while the plan has focus");
    let (x, y) = inside(map);
    let (group, cursor) = (app.selected_group, app.cursor);
    app.handle_mouse(click(x, y));
    app.handle_mouse(wheel_down(x, y));
    assert_eq!(app.focus, Focus::Groups, "a map takes no focus");
    assert_eq!(
        (app.selected_group, app.cursor),
        (group, cursor),
        "and moves nothing"
    );

    // The diff has focus, so the file list floats at the foot of the plan.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    let list = app
        .file_list_area(panes.plan)
        .expect("the file list floats while the diff has focus");
    let (x, y) = inside(list);
    let (group, cursor) = (app.selected_group, app.cursor);
    app.handle_mouse(click(x, y));
    app.handle_mouse(wheel_down(x, y));
    assert_eq!(app.focus, Focus::Detail);
    assert_eq!((app.selected_group, app.cursor), (group, cursor));
}

#[test]
fn the_horizontal_wheel_shifts_the_diff_pane_as_h_and_l_do() {
    let (_r, mut app) = app_with_a_long_line();
    sized(&mut app);
    let (x, y) = inside(layout(SCREEN, DEFAULT_PLAN_COLS).detail);
    let head = "Soft wrap exists because";
    let before = wrapped_pane(&mut app);
    assert!(before.iter().any(|r| r.contains(head)));

    for _ in 0..6 {
        app.handle_mouse(mouse(MouseEventKind::ScrollRight, x, y));
    }
    let after = wrapped_pane(&mut app);
    assert!(
        !after.iter().any(|r| r.contains(head)),
        "six notches right move the head off the pane: {after:#?}"
    );

    for _ in 0..6 {
        app.handle_mouse(mouse(MouseEventKind::ScrollLeft, x, y));
    }
    assert_eq!(
        wrapped_pane(&mut app),
        before,
        "six notches left bring it back"
    );

    // A held key and the ordinary wheel is the same sideways move: most mice
    // have no sideways wheel, and a terminal reports the held key as a
    // modifier. Any of the three, because some terminals never pass shift on.
    for held in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
    ] {
        let with = |kind| MouseEvent {
            modifiers: held,
            ..mouse(kind, x, y)
        };
        for _ in 0..6 {
            app.handle_mouse(with(MouseEventKind::ScrollDown));
        }
        assert_eq!(
            wrapped_pane(&mut app),
            after,
            "{held:?} and six notches down is six notches right"
        );
        for _ in 0..6 {
            app.handle_mouse(with(MouseEventKind::ScrollUp));
        }
        assert_eq!(
            wrapped_pane(&mut app),
            before,
            "{held:?} and six notches up brings it back"
        );
    }
}

// ------------------------------------------------- the symbol float (`z`)

/// A change that declares something in one file and uses it twice on one line
/// in another.
///
/// Two uses on ONE line is the point: stepping is what `z` does after the
/// first press, and a line with a single symbol cannot show it. `helper_two`
/// sits to the right of `helper_one`, so column order and press order must
/// agree.
fn app_with_symbols() -> (TestRepo, App) {
    app_with_symbols_and(laid_out(false))
}

/// [`app_with_symbols`], opened with `opts`.
fn app_with_symbols_and(opts: ReviewOptions) -> (TestRepo, App) {
    let r = TestRepo::new();
    r.write("src/lib.rs", b"// lib\n");
    r.write("src/call.rs", b"// call\n");
    r.commit_all("base");
    r.write(
        "src/lib.rs",
        b"// lib\nfn helper_one() {\n    let inner = 1;\n    inner\n}\nfn helper_two() {\n    2\n}\n",
    );
    r.write(
        "src/call.rs",
        b"// call\nfn caller() {\n    let total = helper_one() + helper_two();\n}\n",
    );
    r.commit_all("head");
    // ONE group holding both files. Per-class groups would show the declaring
    // file and the calling file in different groups, and the reader has to be
    // standing on the call to press `z` at it.
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )
    });
    let app = open_app_with_opts(&r, &backend, ".dfr-symbol-store", opts);
    (r, app)
}

/// Put the cursor on the row whose drawn text contains `needle`.
fn cursor_on_text(app: &mut App, needle: &str) -> usize {
    let pos = app
        .rows
        .iter()
        .position(|r| r.line.as_ref().is_some_and(|l| l.text.contains(needle)))
        .unwrap_or_else(|| panic!("no row containing {needle:?}"));
    app.cursor = pos;
    app.focus = Focus::Detail;
    pos
}

/// Standing on a line marks what it could show, before any key is pressed.
///
/// Without this a reader has to press `z` on every line to find out which ones
/// have anything to say. The mark is what makes the key findable at all.
#[test]
fn a_line_marks_its_symbols_before_z_is_pressed() {
    let (_r, mut app) = app_with_symbols();
    let row = cursor_on_text(&mut app, "helper_one() + helper_two()");

    assert!(app.peek.is_none(), "nothing is open yet");
    let marks = app.symbols_on(row);
    assert_eq!(marks.len(), 2, "both calls are marked: {marks:?}");
    // In column order, and on the names rather than anywhere on the line.
    let text = app.rows[row].line.as_ref().unwrap().text.clone();
    assert_eq!(&text[marks[0].0..marks[0].1], "helper_one");
    assert_eq!(&text[marks[1].0..marks[1].1], "helper_two");

    // A row with nothing to resolve marks nothing.
    let header = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .expect("a hunk header");
    assert!(app.symbols_on(header).is_empty());
}

/// The mark reaches the screen, not just the model.
///
/// `symbols_on` returning the right columns proves nothing about what is drawn;
/// this reads the cells back. An underline on every resolvable name, and the
/// HIGHLIGHT on the one the float is answering — which this used to claim in
/// prose and never check, so the ink could have been anything at all.
#[test]
fn the_marked_symbol_is_underlined_on_screen() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");

    // Every cell carrying a modifier, with the ink it carries — so one read
    // answers both halves: which names are marked, and how the lit one is
    // told from the quiet ones.
    let marked = |app: &mut App, m: ratatui::style::Modifier| -> Vec<(String, Option<Color>)> {
        let backend = ratatui::backend::TestBackend::new(110, 20);
        let mut t = ratatui::Terminal::new(backend).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let buf = t.backend().buffer().clone();
        let mut out = Vec::new();
        for y in 0..20 {
            for x in 0..110 {
                let cell = &buf[(x, y)];
                if cell.modifier.contains(m) {
                    out.push((cell.symbol().to_string(), cell.style().fg));
                }
            }
        }
        out
    };
    let text =
        |v: &[(String, Option<Color>)]| -> String { v.iter().map(|(c, _)| c.as_str()).collect() };
    let underlined = ratatui::style::Modifier::UNDERLINED;
    // Both, not bold alone: every pane title and pill on this screen is bold,
    // and only a lit symbol is bold AND underlined.
    let lit_marks = underlined | ratatui::style::Modifier::BOLD;

    // Nothing open: both names are marked, and nothing else is.
    assert_eq!(
        text(&marked(&mut app, underlined)),
        "helper_onehelper_two",
        "standing on the line marks what it could show"
    );
    // And the quiet mark is a SHAPE: no name has been re-inked yet.
    let t = theme();
    assert!(
        marked(&mut app, underlined)
            .iter()
            .all(|(_, fg)| *fg != Some(t.highlight_ink)),
        "a quiet mark keeps the syntax ink it had"
    );

    // Open on the first: still both underlined, and the lit one alone is bold
    // and wears the highlight.
    app.handle_key(key('z'));
    assert_eq!(text(&marked(&mut app, underlined)), "helper_onehelper_two");
    let lit = marked(&mut app, lit_marks);
    assert_eq!(
        text(&lit),
        "helper_one",
        "one name is lit, and it is the one the float is answering"
    );
    assert!(
        lit.iter().all(|(_, fg)| *fg == Some(t.highlight_ink)),
        "the lit name wears the highlight, not the accent a syntax theme also \
         spends (issue 126): {lit:?}"
    );
    assert_ne!(
        t.highlight_ink, t.header_fg,
        "and those two are different colours, or the fix is no fix"
    );
}

#[test]
fn z_steps_through_the_symbols_on_a_line_then_closes() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");

    app.handle_key(key('z'));
    let first = app.peek.as_ref().expect("the first symbol opens the float");
    assert_eq!(first.nth, 0);
    assert!(
        first.title.starts_with("helper_one ·"),
        "leftmost first: {}",
        first.title
    );

    app.handle_key(key('z'));
    let second = app.peek.as_ref().expect("the second symbol");
    assert_eq!(second.nth, 1);
    assert!(
        second.title.starts_with("helper_two ·"),
        "then rightward: {}",
        second.title
    );

    // Past the last one it closes rather than wrapping: stepping is how the
    // reader asks what else is here, and a wrap gives them no way out through
    // the key they are already pressing.
    app.handle_key(key('z'));
    assert!(app.peek.is_none(), "after the last symbol the float closes");
}

#[test]
fn the_float_shows_the_declaration_body_with_its_own_line_numbers() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));

    let peek = app.peek.as_ref().expect("open");
    let numbers: Vec<u32> = peek.body.iter().map(|l| l.number).collect();
    // `fn helper_one` is lines 2..=5 of the head file, and the float shows all
    // four — the whole declaration, not just the line it starts on.
    assert_eq!(numbers, vec![2, 3, 4, 5], "the whole body");
    let text: String = peek
        .body
        .iter()
        .flat_map(|l| l.pairs.iter().map(|(_, t)| t.as_str()))
        .collect();
    assert!(text.contains("fn helper_one"), "got: {text:?}");
    assert!(
        text.contains("let inner"),
        "the body, not only the signature"
    );
}

/// The float says which of the declaration's lines the change wrote.
///
/// Head-side code with no origin leaves the reader unable to tell a declaration
/// the change INTRODUCED from one it merely touched. It says so the way the
/// pane behind it does — colour, no `+` column — so `origin` is what this
/// asserts.
#[test]
fn the_float_marks_which_lines_the_change_added() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));

    let peek = app.peek.as_ref().expect("open");
    let origins: Vec<LineOrigin> = peek.body.iter().map(|l| l.origin).collect();
    // `src/lib.rs` gains the whole declaration here, so every line is an
    // addition.
    assert!(
        origins.iter().all(|o| *o == LineOrigin::Addition),
        "a wholly new declaration is wholly added: {origins:?}"
    );
}

/// A declaration the change only PARTLY wrote shows both origins.
///
/// This is the case that matters, and the common one: a signature edited while
/// the body stays put. Painting the whole float green there would tell the
/// reader the change introduced a function it merely touched.
///
/// Note what it takes to reach: a definition reaches the index from an ADDED
/// line, so a declaration the change never touched at all resolves to nothing
/// and `z` does what it always did. The mixed case is the reachable one.
#[test]
fn a_declaration_the_change_only_touched_shows_context_too() {
    let r = TestRepo::new();
    r.write(
        "src/lib.rs",
        b"// lib\nfn helper_one(a: u8) {\n    let inner = 1;\n    inner\n}\n",
    );
    r.write("src/call.rs", b"// call\nfn caller() {\n}\n");
    r.commit_all("base");
    // ONLY the signature moves. Lines 3, 4 and 5 are untouched.
    r.write(
        "src/lib.rs",
        b"// lib\nfn helper_one(a: u8, b: u8) {\n    let inner = 1;\n    inner\n}\n",
    );
    r.write(
        "src/call.rs",
        b"// call\nfn caller() {\n    let total = helper_one(1, 2);\n}\n",
    );
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )
    });
    let mut app = open_app_with_opts(&r, &backend, ".dfr-context-store", laid_out(false));
    cursor_on_text(&mut app, "helper_one(1, 2)");
    app.handle_key(key('z'));

    let peek = app.peek.as_ref().expect("the call resolves");
    let origins: Vec<(u32, LineOrigin)> = peek.body.iter().map(|l| (l.number, l.origin)).collect();
    assert_eq!(
        origins,
        vec![
            (2, LineOrigin::Addition),
            (3, LineOrigin::Context),
            (4, LineOrigin::Context),
            (5, LineOrigin::Context),
        ],
        "the signature changed; the body did not"
    );
}

#[test]
fn esc_closes_the_float_and_leaves_a_selection_alone() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('v'));
    app.handle_key(key('z'));
    assert!(app.peek.is_some() && app.visual.is_some());

    // One press, one thing. The float came last, so it goes first.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.peek.is_none(), "the float closed");
    assert!(app.visual.is_some(), "the selection is still open");

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.visual.is_none(), "a second esc drops the selection");
}

#[test]
fn moving_the_cursor_closes_the_float() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    assert!(app.peek.is_some());

    app.handle_key(key('j'));
    assert!(
        app.peek.is_none(),
        "a highlight pointing at a row the cursor left is worse than none"
    );
}

/// EVERY way of leaving the row closes it, not only `j` and `k`.
///
/// Seven things move the cursor and one of them cleared the float, so `G`,
/// `ctrl+d`, a jump and a click each left an answer pointing at a row the
/// reader had gone from. The rule is now one invariant, settled once per
/// event, and this is what holds it there.
#[test]
fn every_way_of_leaving_the_row_closes_the_float() {
    let leavers: &[(&str, KeyEvent)] = &[
        ("G", key('G')),
        ("g", key('g')),
        (
            "ctrl+d",
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        ),
        (
            "ctrl+u",
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        ),
        ("n", key('n')),
    ];
    for (named, press) in leavers {
        let (_r, mut app) = app_with_symbols();
        let row = cursor_on_text(&mut app, "helper_one() + helper_two()");
        app.handle_key(key('z'));
        assert!(app.peek.is_some(), "{named}: the float opened");

        app.handle_key(*press);
        if app.cursor == row {
            continue; // That key had nowhere to go on this document.
        }
        assert!(
            app.peek.is_none(),
            "{named} left the row and the float stayed"
        );
    }
}

/// A click on another row closes it too — the click sets the cursor directly.
#[test]
fn a_click_on_another_row_closes_the_float() {
    let (_r, mut app) = app_with_symbols();
    sized(&mut app);
    let row = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    assert!(app.peek.is_some());

    // A selectable row that is not the cursor's, and whose screen line the
    // float does not cover — the float swallows a click on itself by design.
    let panes = app.panes();
    let float = app.peek_area(panes.detail).expect("the float has a place");
    let mut top = 0usize;
    let mut hit = None;
    for i in 0..app.rows.len() {
        let (_, y) = on_line(panes.detail, top as u16);
        if i != row && app.rows[i].kind.selectable() && !(float.y..float.bottom()).contains(&y) {
            hit = Some((i, top));
            break;
        }
        top += app.row_height(i);
    }
    let (target, line) = hit.expect("some other row is clickable");

    let (x, y) = on_line(panes.detail, line as u16);
    app.handle_mouse(click(x, y));
    assert_eq!(app.cursor, target, "the click moved the cursor");
    assert!(app.peek.is_none(), "and the float went with it");
}

/// Focus leaving the diff pane closes it, because the marks leave with it.
///
/// The underlines are drawn only while the diff pane has focus, so a float
/// open under the plan pane is an answer with no visible question.
#[test]
fn focus_leaving_the_diff_pane_closes_the_float() {
    let (_r, mut app) = app_with_symbols();
    sized(&mut app);
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    assert!(app.peek.is_some());

    let panes = app.panes();
    let (x, y) = inside(panes.plan);
    app.handle_mouse(click(x, y));
    assert_eq!(app.focus, Focus::Groups, "the click took focus");
    assert!(app.peek.is_none(), "the float closed with the marks");
}

#[test]
fn z_on_a_row_with_no_symbol_still_folds_the_group() {
    let (_r, mut app) = app_with_symbols();
    // A row the index cannot resolve: the file header is not a diff line.
    let before = app.folds_open.clone();
    app.focus = Focus::Detail;
    put_cursor_on(&mut app, |k| matches!(k, RowKind::HunkHeader { .. }));
    app.handle_key(key('z'));
    assert!(app.peek.is_none(), "nothing to peek at on a hunk header");
    assert_ne!(
        app.folds_open, before,
        "z kept the meaning it already had on this row"
    );
}

#[test]
fn the_float_never_covers_the_cursors_row() {
    let (_r, mut app) = app_with_symbols();
    let row = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));

    let panes = app.panes();
    let area = app.peek_area(panes.detail).expect("the float has a home");

    // Where the cursor's row is drawn, in the same arithmetic the float used.
    let mut top = panes.detail.y + 1;
    for i in app.scroll()..row {
        top += app.row_height(i) as u16;
    }
    let bottom = top + app.row_height(row) as u16;
    let covers = area.y < bottom && top < area.y + area.height;
    assert!(
        !covers,
        "float at {}..{} overlaps the row at {top}..{bottom}",
        area.y,
        area.y + area.height
    );
}

#[test]
fn a_pane_too_short_for_the_float_shows_none_of_it() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    // Five rows of terminal: a bordered pane with almost nothing in it. The
    // float yields entirely rather than land on the row it is about.
    app.set_area(Rect::new(0, 0, 120, 5));
    let panes = app.panes();
    assert!(app.peek_area(panes.detail).is_none());
}

/// A NEW call to a helper that was already there resolves.
///
/// The shape the author asked for: `sum_xy` exists, this change adds a call to
/// it, and the call site has to light up. Nothing about `sum_xy` itself moves.
#[test]
fn a_new_call_to_an_existing_helper_lights_up() {
    let r = TestRepo::new();
    let lib_base = b"// lib\nfn sum_xy(a: u8, b: u8) -> u8 {\n    a + b\n}\nfn other() {}\n";
    r.write("src/lib.rs", lib_base);
    r.write("src/call.rs", b"// call\nfn caller() {\n}\n");
    r.commit_all("base");
    // `sum_xy` is untouched. Only `other` changes, which is what keeps the
    // file in the diff at all.
    r.write(
        "src/lib.rs",
        b"// lib\nfn sum_xy(a: u8, b: u8) -> u8 {\n    a + b\n}\nfn other() { changed() }\n",
    );
    r.write(
        "src/call.rs",
        b"// call\nfn caller() {\n    let n = sum_xy(1, 2);\n}\n",
    );
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )
    });
    let mut app = open_app_with_opts(&r, &backend, ".dfr-sumxy-store", laid_out(false));
    let row = cursor_on_text(&mut app, "sum_xy(1, 2)");

    // Marked before any key is pressed.
    let marks = app.symbols_on(row);
    let text = app.rows[row].line.as_ref().unwrap().text.clone();
    assert!(
        marks.iter().any(|(s, e)| &text[*s..*e] == "sum_xy"),
        "the new call site marks the helper: {marks:?} in {text:?}"
    );

    app.handle_key(key('z'));
    let peek = app.peek.as_ref().expect("the call resolves");
    assert!(
        peek.title.starts_with("sum_xy · src/lib.rs:2"),
        "got: {}",
        peek.title
    );
    // No class and no group: the change did not write this declaration, and
    // the title says so by leaving them off.
    assert_eq!(
        peek.title, "sum_xy · src/lib.rs:2",
        "a declaration outside the change names no group to read first"
    );
    // And the body is wholly unchanged, which is the other half of saying it.
    assert!(
        peek.body.iter().all(|l| l.origin == LineOrigin::Context),
        "the change never wrote this declaration"
    );
}

/// `enter` on an open float goes to what it shows (issue 161).
///
/// The float answers "what is this", and the reader's next question is often
/// "then let me read it where it lives" — with its neighbours, its findings,
/// its `space`. The cursor lands on the declaration's first line.
#[test]
fn enter_on_the_float_goes_to_the_declaration() {
    let (_r, mut app) = app_with_symbols();
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    // The SECOND symbol, so the jump follows the one the float is answering
    // and not merely the first name on the line.
    app.handle_key(key('z'));
    app.handle_key(key('z'));
    assert!(app.peek.as_ref().is_some_and(|p| p.nth == 1));

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_path(&app).as_deref(), Some("src/lib.rs"));
    assert_eq!(cursor_line(&app), Some(6), "`fn helper_two` is line 6");
    assert_eq!(app.focus, Focus::Detail);
    assert!(app.peek.is_none(), "the float closes behind the jump");
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_jump_to_declaration`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_jump_to_declaration() {
    let (_r, mut app) = app_with_symbols();
    sized(&mut app);
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    app.handle_key(key('z'));
    println!("\n=== z z — helper_two floated; the footer offers enter ===");
    for row in screen(&app, 100, 30) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    println!("\n=== enter — the cursor on `fn helper_two`, src/lib.rs:6 ===");
    for row in screen(&app, 100, 30) {
        println!("{row}");
    }
    app.handle_key(ctrl('o'));
    println!("\n=== ctrl-o — back on the call, no float ===");
    for row in screen(&app, 100, 30) {
        println!("{row}");
    }
    app.handle_key(key('?'));
    println!("\n=== ? — ctrl-o is among the keys that work anywhere ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
}

/// Without a float, `enter` in the diff pane still does nothing.
#[test]
fn enter_without_a_float_moves_nothing() {
    let (_r, mut app) = app_with_symbols();
    let row = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.cursor, row);
}

/// A declaration the change did not write is outside every hunk, and the
/// jump opens the window to reach it — the same way a search hit does.
#[test]
fn enter_opens_the_window_to_reach_an_untouched_declaration() {
    let r = TestRepo::new();
    r.write(
        "src/lib.rs",
        b"// lib\nfn sum_xy(a: u8, b: u8) -> u8 {\n    a + b\n}\n\n\n\n\n\n\n\nfn other() {}\n",
    );
    r.write("src/call.rs", b"// call\nfn caller() {\n}\n");
    r.commit_all("base");
    // `sum_xy` is untouched and eight lines above the only hunk in its file,
    // past the three lines of context the pane opens with.
    r.write(
        "src/lib.rs",
        b"// lib\nfn sum_xy(a: u8, b: u8) -> u8 {\n    a + b\n}\n\n\n\n\n\n\n\nfn other() { changed() }\n",
    );
    r.write(
        "src/call.rs",
        b"// call\nfn caller() {\n    let n = sum_xy(1, 2);\n}\n",
    );
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )
    });
    let mut app = open_app_with_opts(&r, &backend, ".dfr-untouched-store", laid_out(false));
    assert!(
        !app.rows.iter().any(|r| r
            .line
            .as_ref()
            .is_some_and(|l| l.text.contains("fn sum_xy"))),
        "the fixture needs the declaration out of view"
    );
    cursor_on_text(&mut app, "sum_xy(1, 2)");
    app.handle_key(key('z'));
    assert!(app.peek.is_some());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_path(&app).as_deref(), Some("src/lib.rs"));
    assert_eq!(cursor_line(&app), Some(2), "the gap was opened to reach it");
}

/// A declaration in one group and its call in another, with the calling
/// group selected.
fn app_with_a_declaration_in_another_group() -> (TestRepo, App) {
    let r = TestRepo::new();
    // Two shapes, so two classes and so two groups: the declaring file edits a
    // line in place, and the calling file only adds.
    r.write("src/lib.rs", b"// lib\nfn helper_one() {\n    0\n}\n");
    r.write("src/call.rs", b"// call\n");
    r.commit_all("base");
    r.write("src/lib.rs", b"// lib\nfn helper_one() {\n    1\n}\n");
    r.write(
        "src/call.rs",
        b"// call\nfn caller() {\n    let total = helper_one();\n}\n",
    );
    r.commit_all("head");
    let mut app = open_app_with_opts(
        &r,
        &one_group_per_class(),
        ".dfr-cross-group-store",
        laid_out(false),
    );
    // Stand in whichever group holds the call.
    let _calling = (0..app.groups().len())
        .find(|&g| {
            app.selected_group = g;
            app.rebuild_rows();
            app.rows.iter().any(|r| {
                r.line
                    .as_ref()
                    .is_some_and(|l| l.text.contains("let total = helper_one()"))
            })
        })
        .expect("a group holds the call");
    (r, app)
}

/// A declaration in another group is reached by selecting that group, whose
/// rows do not exist until it is — the navigation a search hit makes.
#[test]
fn enter_reaches_a_declaration_in_another_group() {
    let (_r, mut app) = app_with_a_declaration_in_another_group();
    let calling = app.selected_group;
    cursor_on_text(&mut app, "let total = helper_one()");
    assert!(
        !app.rows.iter().any(|r| r
            .line
            .as_ref()
            .is_some_and(|l| l.text.contains("fn helper_one"))),
        "the fixture needs the declaration in another group"
    );
    app.handle_key(key('z'));
    assert!(app.peek.is_some());

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(
        app.selected_group, calling,
        "it went to the declaring group"
    );
    assert_eq!(cursor_path(&app).as_deref(), Some("src/lib.rs"));
    assert_eq!(cursor_line(&app), Some(2));
}

/// The float names its own keys on its bottom edge, and each is a button.
///
/// The window footer said `enter go to` already, but the reader's eyes are
/// on the float; a key the float does not name is one they have to go and
/// find.
#[test]
fn the_float_names_enter_on_its_own_edge_and_a_click_presses_it() {
    let (_r, mut app) = app_with_symbols();
    sized(&mut app);
    cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    let area = app
        .peek_area(app.panes().detail)
        .expect("the float has a home");
    let edge = area.bottom() - 1;
    let rows = screen(&app, SCREEN.width, SCREEN.height);
    let drawn = &rows[edge as usize];
    assert!(
        drawn.trim_end().ends_with("enter go to  ·  esc close ┘"),
        "the bottom edge names the keys, against its corner: {drawn:?}"
    );

    // A click on `enter` presses it.
    let col = drawn
        .char_indices()
        .map(|(b, _)| b)
        .position(|b| drawn[b..].starts_with("enter go to"))
        .expect("the hint is on the edge") as u16;
    app.handle_mouse(click(col, edge));
    assert_eq!(cursor_path(&app).as_deref(), Some("src/lib.rs"));
    assert_eq!(
        cursor_line(&app),
        Some(2),
        "the click went to the declaration"
    );
}

/// `ctrl-o` goes back to where the jump left from.
#[test]
fn ctrl_o_goes_back_from_the_declaration_to_the_call() {
    let (_r, mut app) = app_with_symbols();
    let call = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_path(&app).as_deref(), Some("src/lib.rs"));

    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, call, "back on the call");
    assert_eq!(app.focus, Focus::Detail);
    assert!(
        app.peek.is_none(),
        "only the cursor comes back, not the float"
    );

    // And the stack is spent.
    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, call);
    assert_eq!(app.status, "nowhere to go back to");
}

/// A chain of declarations walks back up the way it came.
#[test]
fn ctrl_o_walks_back_up_a_chain_of_declarations() {
    let r = TestRepo::new();
    r.write("src/lib.rs", b"// lib\n");
    r.commit_all("base");
    r.write(
        "src/lib.rs",
        b"// lib\nfn leaf() {}\nfn middle() {\n    leaf();\n}\nfn top() {\n    middle();\n}\n",
    );
    r.commit_all("head");
    let mut app = open_app_with_opts(
        &r,
        &one_group_per_class(),
        ".dfr-chain-store",
        laid_out(false),
    );
    let start = cursor_on_text(&mut app, "    middle();");
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_line(&app), Some(3), "at `fn middle`");
    let middle = cursor_on_text(&mut app, "    leaf();");
    // Standing somewhere else first is a move, not a jump; the stack holds
    // where the jumps LEFT from.
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_line(&app), Some(2), "at `fn leaf`");

    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, middle, "first back to the call of leaf");
    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, start, "then back to the call of middle");
}

/// Back from another group selects the group the reader came from.
#[test]
fn ctrl_o_comes_back_across_groups() {
    let (_r, mut app) = app_with_a_declaration_in_another_group();
    let calling = app.selected_group;
    let call = cursor_on_text(&mut app, "let total = helper_one()");
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(app.selected_group, calling);

    app.handle_key(ctrl('o'));
    assert_eq!(app.selected_group, calling, "the calling group again");
    assert_eq!(app.cursor, call);
}

/// Back lands on the LINE, not a stale row index: the jump opened a gap in
/// a file drawn above the call, which moved every row below it.
#[test]
fn ctrl_o_lands_on_the_line_after_the_jump_moved_the_rows() {
    let r = TestRepo::new();
    let lib = |tail: &str| {
        format!("// lib\nfn sum_xy(a: u8, b: u8) -> u8 {{\n    a + b\n}}\n\n\n\n\n\n\n\n{tail}\n")
    };
    // `a_lib.rs` sorts above `z_call.rs`, so what the jump opens is drawn
    // ABOVE the call.
    r.write("src/a_lib.rs", lib("fn other() {}").as_bytes());
    r.write("src/z_call.rs", b"// call\nfn caller() {\n}\n");
    r.commit_all("base");
    r.write("src/a_lib.rs", lib("fn other() { changed() }").as_bytes());
    r.write(
        "src/z_call.rs",
        b"// call\nfn caller() {\n    let n = sum_xy(1, 2);\n}\n",
    );
    r.commit_all("head");
    let backend = FakeBackend::new("fake", |ids| {
        let all: Vec<String> = ids
            .iter()
            .map(|i| format!("{i:?}").trim_matches('"').to_string())
            .collect();
        format!(
            r#"{{"groups": [{}]}}"#,
            json_group(
                "Everything",
                "focus",
                &all.iter().map(String::as_str).collect::<Vec<_>>()
            )
        )
    });
    let mut app = open_app_with_opts(&r, &backend, ".dfr-shifted-store", laid_out(false));
    let call = cursor_on_text(&mut app, "sum_xy(1, 2)");
    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_line(&app), Some(2));

    app.handle_key(ctrl('o'));
    assert_ne!(app.cursor, call, "the fixture needs the rows to have moved");
    assert_eq!(cursor_path(&app).as_deref(), Some("src/z_call.rs"));
    assert_eq!(cursor_line(&app), Some(3), "back on the call's line");
}

/// Every jump pushes, not only the float's: a search hit and a finding too.
#[test]
fn ctrl_o_comes_back_from_a_search_and_from_a_finding() {
    let (_r, mut app) = app_with_symbols();
    let call = cursor_on_text(&mut app, "helper_one() + helper_two()");
    note_on(
        &mut app,
        |r| {
            r.line
                .as_ref()
                .is_some_and(|l| l.text.contains("fn helper_two"))
        },
        "a note",
    );

    // From the call, to the note, through the findings list.
    app.cursor = call;
    app.handle_key(key('F'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(app.cursor, call, "the list jumped");
    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, call, "back from the finding");

    // And through a search.
    search_for(&mut app, "let inner");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_line(&app), Some(3));
    app.handle_key(ctrl('o'));
    assert_eq!(app.cursor, call, "back from the search hit");
}

/// A jump that goes nowhere records nothing, and `f` forgets every place:
/// a place names a group in one view and a tree row in the other.
#[test]
fn a_jump_that_stays_put_records_nothing_and_f_clears_the_stack() {
    let (_r, mut app) = app_with_symbols();
    let call = cursor_on_text(&mut app, "helper_one() + helper_two()");
    // The top hit is the line the reader is on.
    search_for(&mut app, "helper_two()");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.cursor, call);
    app.handle_key(ctrl('o'));
    assert_eq!(app.status, "nowhere to go back to");

    app.handle_key(key('z'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.focus = Focus::Groups;
    app.handle_key(key('f'));
    app.handle_key(ctrl('o'));
    assert_eq!(app.status, "nowhere to go back to");
}

// ------------------------------------------------------------------ search

/// `/` and a word typed into the box.
fn search_for(app: &mut App, word: &str) {
    app.handle_key(key('/'));
    for c in word.chars() {
        app.handle_key(key(c));
    }
}

/// The occurrence list, as `path:line g<N> <tier>`.
fn occurrences(app: &App) -> Vec<String> {
    searching(app)
        .entries
        .iter()
        .map(|e| format!("{}:{} {}", e.path, e.line, e.badge))
        .collect()
}

/// The open search box. Every assertion about it goes through here, so the
/// tests know one thing about its shape and not seven.
fn searching(app: &App) -> &Search {
    app.search().expect("the search box is not open")
}

fn search_selected(app: &App) -> usize {
    searching(app).selected
}

fn search_query(app: &App) -> &str {
    searching(app).query()
}

/// Move the list onto the first occurrence in `path`.
fn select_occurrence_in(app: &mut App, path: &str) {
    let want = occurrences(app)
        .iter()
        .position(|o| o.starts_with(&format!("{path}:")))
        .unwrap_or_else(|| panic!("no occurrence in {path}: {:?}", occurrences(app)));
    while search_selected(app) < want {
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    }
    while search_selected(app) > want {
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    }
}

/// The head-side line the cursor is standing on.
fn cursor_line(app: &App) -> Option<u32> {
    app.rows[app.cursor].line.as_ref()?.line_on("new")
}

fn cursor_path(app: &App) -> Option<String> {
    app.file_at_cursor().map(|i| app.files()[i].path.clone())
}

#[test]
fn slash_finds_a_word_in_every_changed_file() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    assert_eq!(
        occurrences(&app),
        vec![
            "src/a.txt:1 g1 skim C0",
            "src/b.txt:1 g1 skim C0",
            "src/c.txt:1 g1 skim C0",
        ],
        "one row per matching LINE, labelled with its group, tier and shape class"
    );
}

#[test]
fn the_search_reads_the_file_as_it_is_now_so_a_removed_word_is_gone() {
    let (_r, mut app) = app_with_a_long_file();
    search_for(&mut app, "after");
    assert_eq!(
        occurrences(&app),
        vec!["src/long.rs:20 g0 focus C0", "src/long.rs:40 g0 focus C0"],
        "the head side is what is searched"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    search_for(&mut app, "before");
    assert!(
        occurrences(&app).is_empty(),
        "the change removed both of those lines, so they are not in the file"
    );
}

#[test]
fn a_hit_inside_a_hunk_ranks_above_one_outside() {
    let (_r, mut app) = app_with_a_long_file();
    // `9` is on two changed lines (20 and 40) and on six unchanged ones.
    search_for(&mut app, "9");
    let found = occurrences(&app);
    assert_eq!(
        &found[..2],
        &["src/long.rs:20 g0 focus C0", "src/long.rs:40 g0 focus C0"],
        "the changed lines come first: {found:?}"
    );
    assert!(
        found.len() > 2,
        "the unchanged lines are found too: {found:?}"
    );
    assert!(
        found[2..].iter().all(|o| !o.contains(" C")),
        "a line inside no hunk has no shape class, and the gap says so: {found:?}"
    );
}

#[test]
fn the_group_the_reader_has_open_ranks_first() {
    let (_r, mut app) = make_app();
    // g1 is the skim sweep over a, b and c; g0 is the one file main.txt.
    app.handle_key(key('J'));
    search_for(&mut app, "content");
    assert!(
        occurrences(&app).first().is_some_and(|o| o.contains("g1")),
        "the open group's hits lead: {:?}",
        occurrences(&app)
    );
}

#[test]
fn enter_lands_the_cursor_on_the_matched_line() {
    let (_r, mut app) = app_with_a_long_file();
    search_for(&mut app, "also_after");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal), "the box closes");
    assert_eq!(app.focus, Focus::Detail);
    assert_eq!(cursor_path(&app).as_deref(), Some("src/long.rs"));
    assert_eq!(cursor_line(&app), Some(40));
}

#[test]
fn enter_opens_a_folded_skim_remainder_to_reach_its_line() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    // a, b and c share one shape class, so the skim group lists one of them
    // and defers the rest. Reaching a deferred one has to open the fold.
    select_occurrence_in(&mut app, "src/c.txt");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_path(&app).as_deref(), Some("src/c.txt"));
    assert_eq!(cursor_line(&app), Some(1));
}

#[test]
fn enter_opens_the_window_to_reach_a_line_outside_every_hunk() {
    let (_r, mut app) = app_with_a_long_file();
    // Line 30 sits in the gap between the two hunks, which the pane does not
    // show until something opens it.
    search_for(&mut app, "filler30");
    assert_eq!(occurrences(&app), vec!["src/long.rs:30 g0 focus"]);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(cursor_line(&app), Some(30), "the gap was opened");
}

#[test]
fn a_line_too_far_from_any_hunk_says_so_rather_than_opening_the_file() {
    // The hunk is at the top and the word is 4,500 lines below it, which is
    // past the cap on how far `enter` will pull a file open.
    let r = TestRepo::new();
    let body = |first: &str| {
        let mut out = String::from(first);
        for i in 2..=4500 {
            out.push_str(&format!("let filler{i} = {i};\n"));
        }
        out.push_str("let needle = 0;\n");
        out.into_bytes()
    };
    r.write("src/vast.rs", &body("let before = 1;\n"));
    r.commit_all("base");
    r.write("src/vast.rs", &body("let after = 99;\n"));
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-vast-store");
    search_for(&mut app, "needle");
    assert_eq!(occurrences(&app).len(), 1);
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.status.contains("lines from the nearest hunk"),
        "the footer says how far it still is: {:?}",
        app.status
    );
}

#[test]
fn the_query_and_the_hit_survive_a_close() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('/'));
    assert_eq!(occurrences(&app).len(), 3, "the query came back");
    assert_eq!(search_selected(&app), 1, "and so did the hit it was on");
}

#[test]
fn every_printable_key_types_and_the_arrows_move() {
    let (_r, mut app) = make_app();
    // `j`, `k`, `q` and `?` are keys of the review, and characters here.
    search_for(&mut app, "j");
    app.handle_key(key('k'));
    app.handle_key(key('?'));
    assert_eq!(search_query(&app), "jk?");
    app.handle_key(ctrl('u'));
    search_for_more(&mut app, "helper");
    assert_eq!(search_selected(&app), 0);
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(search_selected(&app), 1, "the arrows move the list");
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(search_query(&app), "helpe", "backspace edits the query");
}

/// Type into a box that is already open.
fn search_for_more(app: &mut App, word: &str) {
    for c in word.chars() {
        app.handle_key(key(c));
    }
}

#[test]
fn an_uppercase_letter_makes_the_query_case_sensitive() {
    let r = TestRepo::new();
    r.write(
        "src/case.txt",
        b"let value = 1;
let VALUE = 2;
",
    );
    r.commit_all("base");
    r.write(
        "src/case.txt",
        b"let value = 3;
let VALUE = 4;
",
    );
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-case-store");
    search_for(&mut app, "value");
    assert_eq!(occurrences(&app).len(), 2, "all lowercase folds case");
    app.handle_key(ctrl('u'));
    search_for_more(&mut app, "VALUE");
    assert_eq!(
        occurrences(&app),
        vec!["src/case.txt:2 g0 focus C0"],
        "an uppercase letter is the case the reader meant"
    );
}

#[test]
fn the_query_is_a_literal_not_a_regexp() {
    let r = TestRepo::new();
    r.write(
        "src/dots.txt",
        b"a.c
abc
",
    );
    r.commit_all("base");
    r.write(
        "src/dots.txt",
        b"a.c
abc
more
",
    );
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-dots-store");
    search_for(&mut app, "a.c");
    // Line 1 is unchanged, so the hit is outside every hunk and carries no
    // shape class — only the group that reads the file.
    assert_eq!(occurrences(&app), vec!["src/dots.txt:1 g0 focus"]);
}

#[test]
fn the_search_opens_from_either_pane_and_from_either_list() {
    let (_r, mut app) = make_app();
    for focus in [Focus::Groups, Focus::Detail] {
        app.focus = focus;
        app.handle_key(key('/'));
        assert!(matches!(app.mode, Mode::Search { .. }), "from {focus:?}");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    }
    // And from the two list modals, which close as it opens.
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    app.handle_key(key('/'));
    assert!(
        matches!(app.mode, Mode::Search { .. }),
        "from the file list"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    // The findings list needs a finding in it before it opens.
    app.handle_key(key('c'));
    app.handle_paste("a note");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(key('F'));
    assert!(matches!(app.mode, Mode::Findings { .. }));
    app.handle_key(key('/'));
    assert!(
        matches!(app.mode, Mode::Search { .. }),
        "from the findings list"
    );
}

#[test]
fn the_box_spans_both_panes() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let body = layout(SCREEN, DEFAULT_PLAN_COLS).body;
    let area = search_modal_area(body);
    let plan = layout(SCREEN, DEFAULT_PLAN_COLS).plan;
    assert!(area.x < plan.right(), "it starts over the plan pane");
    assert!(area.right() > plan.right(), "and ends over the diff pane");
}

#[test]
fn a_click_selects_an_occurrence_and_a_second_opens_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    // The list starts one row below the query row.
    let (x, y) = on_line(area, 2);
    app.handle_mouse(click(x, y));
    assert_eq!(search_selected(&app), 1, "a click selects");
    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal), "a second click opens it");
    assert_eq!(cursor_path(&app).as_deref(), Some("src/b.txt"));
}

#[test]
fn a_click_outside_the_box_closes_the_search() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    app.handle_mouse(click(0, 0));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn the_wheel_steps_the_occurrence_list() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    let (x, y) = on_line(area, 2);
    app.handle_mouse(wheel_down(x, y));
    assert_eq!(search_selected(&app), 1);
    app.handle_mouse(wheel_up(x, y));
    assert_eq!(search_selected(&app), 0);
}

#[test]
fn the_footer_keys_are_buttons_here_too() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    let row = footer_row(area);
    let hints = app.modal_footer();
    // By its words, not by an index: this footer has grown a button once.
    let (x, y) = on_hint(&hints, hint_named(&hints, "close"), row.x, row);
    app.handle_mouse(click(x, y));
    assert!(matches!(app.mode, Mode::Normal), "esc was pressed");
}

/// Which hint of a footer says `word`. Aimed at by name so a reworded or
/// reordered footer moves the click with it rather than breaking the test.
fn hint_named(hints: &[Hint], word: &str) -> usize {
    hints
        .iter()
        .position(|h| h.pieces.iter().any(|(t, _)| t.contains(word)))
        .unwrap_or_else(|| panic!("no hint saying {word:?}"))
}

#[test]
fn the_footer_names_ctrl_r_and_a_click_on_it_presses_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    let row = footer_row(area);
    let hints = app.modal_footer();
    // The label says what the key WILL do, which is the footer's own rule.
    let at = hint_named(&hints, "regexp");
    let (x, y) = on_hint(&hints, at, row.x, row);
    app.handle_mouse(click(x, y));
    assert_eq!(app.search_reading(), Reading::Regexp);
    // And now it offers the way back.
    let hints = app.modal_footer();
    assert!(
        hints
            .iter()
            .any(|h| h.pieces.iter().any(|(t, _)| t.contains("literal"))),
        "the label follows the reading"
    );
}

#[test]
fn a_regexp_reading_wears_a_pill_and_a_literal_one_does_not() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    // The QUERY row, not the whole screen: the footer says `ctrl-r regexp`
    // in either reading, because a label says what the key WILL do.
    let query_row = |app: &App| -> String {
        let y = pane_inner(search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body)).y;
        screen(app, 100, 40)[y as usize].clone()
    };
    assert!(
        !query_row(&app).contains("regexp"),
        "a literal reading is the default and wears no badge: {}",
        query_row(&app)
    );
    app.handle_key(ctrl('r'));
    assert!(
        query_row(&app).contains("regexp"),
        "the reading is a pill on the query row: {}",
        query_row(&app)
    );
}

#[test]
fn the_preview_fills_the_hit_and_reverses_the_ink_out_of_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let t = theme();
    let buf = buffer_of(&app);
    let filled: Vec<(u16, u16)> = (0..40u16)
        .flat_map(|y| (0..100u16).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].style().bg == Some(t.highlight_bg))
        .collect();
    let word: String = filled
        .iter()
        .map(|&(x, y)| buf[(x, y)].symbol().to_string())
        .collect();
    assert_eq!(word, "helper", "the whole word wears the fill, and only it");
    assert!(
        filled
            .iter()
            .all(|&(x, y)| buf[(x, y)].style().fg == Some(t.highlight_fg)),
        "and the ink on it is the one reversed out of the fill"
    );
}

#[test]
fn ctrl_r_reads_the_query_as_a_regexp() {
    let r = TestRepo::new();
    r.write("src/dots.txt", b"a.c\nabc\n");
    r.commit_all("base");
    r.write("src/dots.txt", b"a.c\nabc\nmore\n");
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-re-store");
    search_for(&mut app, "a.c");
    assert_eq!(occurrences(&app), vec!["src/dots.txt:1 g0 focus"]);
    app.handle_key(ctrl('r'));
    assert_eq!(
        occurrences(&app),
        vec!["src/dots.txt:1 g0 focus", "src/dots.txt:2 g0 focus"],
        "the dot is any character now"
    );
    // And back, on the same query.
    app.handle_key(ctrl('r'));
    assert_eq!(occurrences(&app), vec!["src/dots.txt:1 g0 focus"]);
}

#[test]
fn the_reading_survives_a_close() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    assert_eq!(
        app.search_reading(),
        Reading::Literal,
        "a box opens literal"
    );
    app.handle_key(ctrl('r'));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('/'));
    assert_eq!(
        app.search_reading(),
        Reading::Regexp,
        "and comes back as the reader left it"
    );
}

/// The caret is a CHARACTER index and the window is a COLUMN count, and the
/// two are the same number only until the query holds a wide character.
#[test]
fn the_caret_lands_on_its_character_in_a_wide_query() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(key('/'));
    for c in "\u{3042}\u{3044}x".chars() {
        app.handle_key(key(c));
    }
    // Two characters back from the end: the caret sits on the second kana,
    // which is at column 2 and char 1.
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    let inner = pane_inner(area);
    let buf = buffer_of(&app);
    let reversed: String = (inner.x..inner.right())
        .filter(|x| {
            buf[(*x, inner.y)]
                .style()
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED)
        })
        .map(|x| buf[(x, inner.y)].symbol().to_string())
        .collect();
    assert_eq!(
        reversed.trim(),
        "\u{3044}",
        "the caret is on the character it indexes, not on the one at that column"
    );
}

#[test]
fn the_query_row_carries_no_lead_character() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    let y = pane_inner(search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body)).y;
    let row = screen(&app, 100, 40)[y as usize].clone();
    assert!(row.contains("helper"), "the query is on its row: {row}");
    assert!(
        !row.contains("/helper") && !row.contains("/~helper"),
        "nothing leads it — the box's title says it is a search: {row}"
    );
}

#[test]
fn the_arrows_move_the_caret_and_typing_lands_on_it() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    for _ in 0..3 {
        app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    }
    app.handle_key(key('X'));
    assert_eq!(search_query(&app), "helXper", "typed where the caret was");
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(search_query(&app), "helper", "and deleted before it");
    app.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    app.handle_key(key('s'));
    assert_eq!(search_query(&app), "helpers");
}

#[test]
fn an_arrow_keeps_a_query_that_came_back_and_typing_replaces_it() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('/'));
    // Moving the caret is not writing, so the word the reader came back to
    // stays — and the next character still replaces the whole of it.
    app.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(search_query(&app), "helper", "an arrow keeps it");
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(search_query(&app), "helper", "and so does the list moving");
    app.handle_key(key('o'));
    assert_eq!(search_query(&app), "o", "typing replaces it");
}

#[test]
fn a_regexp_that_does_not_compile_says_so() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    app.handle_key(ctrl('r'));
    app.handle_key(key('('));
    assert!(occurrences(&app).is_empty());
    let on_screen = screen(&app, 100, 40).join("\n");
    assert!(
        on_screen.contains("bad regexp"),
        "a typo and an honest answer have to look different:\n{on_screen}"
    );
}

#[test]
fn backspace_clears_a_query_that_came_back_selected() {
    let (_r, mut app) = make_app();
    search_for(&mut app, "helper");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('/'));
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(
        search_query(&app),
        "",
        "backspace takes the whole selection"
    );
}

#[test]
fn the_preview_shifts_so_a_hit_far_along_a_line_is_on_it() {
    let r = TestRepo::new();
    let long = format!("let x = 1; // {}NEEDLE\n", "pad ".repeat(60));
    r.write("src/wide.txt", b"let x = 0;\n");
    r.commit_all("base");
    r.write("src/wide.txt", long.as_bytes());
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-wide-store");
    sized(&mut app);
    search_for(&mut app, "NEEDLE");
    assert_eq!(occurrences(&app), vec!["src/wide.txt:1 g0 focus C0"]);
    let on_screen = screen(&app, 100, 40).join("\n");
    assert!(
        on_screen.contains("NEEDLE"),
        "the hit is 250 columns in and has to be shown:\n{on_screen}"
    );
}

#[test]
fn the_search_box_keeps_the_themes_ground() {
    use differential_engine::config::ThemeName;
    let (_r, mut app) = make_app();
    app.set_theme(Theme::named(ThemeName::SolarizedLight));
    sized(&mut app);
    search_for(&mut app, "helper");
    let ground = Some(Theme::named(ThemeName::SolarizedLight).bg);
    let buf = buffer_of(&app);
    let area = search_modal_area(layout(SCREEN, DEFAULT_PLAN_COLS).body);
    let stray: Vec<String> = (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].style().bg == Some(Color::Reset))
        .map(|(x, y)| format!("({x},{y})"))
        .take(5)
        .collect();
    assert!(stray.is_empty(), "cells left to the terminal: {stray:?}");
    assert!(
        (area.x..area.right()).any(|x| buf[(x, area.y + 1)].style().bg == ground),
        "the box carries none of the theme's ground"
    );
}

#[test]
fn a_binary_file_is_searched_for_nothing() {
    let r = TestRepo::new();
    r.write(
        "src/keep.txt",
        b"searchable word here
",
    );
    r.write("blob.bin", &[0u8, 159, 146, 150, b'w', b'o', b'r', b'd', 0]);
    r.commit_all("base");
    r.write(
        "src/keep.txt",
        b"searchable word there
",
    );
    r.write("blob.bin", &[0u8, 1, 2, b'w', b'o', b'r', b'd', 0, 3]);
    r.commit_all("head");
    let mut app = open_app_with(&r, &one_group_per_class(), ".dfr-bin-store");
    search_for(&mut app, "word");
    assert_eq!(
        occurrences(&app),
        vec!["src/keep.txt:1 g0 focus C0"],
        "the binary file is enumerated and counted, and holds no text to find"
    );
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_search`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_search() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    search_for(&mut app, "helper");
    println!("\n=== / helper — the list, and the preview under it ===");
    println!("(the hit is a filled yellow block; a dump shows no colour)");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(ctrl('r'));
    println!("\n=== ctrl-r — the same query, read as a regexp ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(ctrl('r'));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('/'));
    println!("\n=== / again — the query comes back SELECTED ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    println!("\n=== enter — landed on src/a.txt:1 ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }

    // A file long enough to fill the preview, and a hit outside every hunk.
    let (_r, mut app) = app_with_a_long_file();
    sized(&mut app);
    search_for(&mut app, "filler3");
    println!("\n=== / filler3 — hits inside no hunk, so no shape class ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    println!("\n=== enter — the context gap opened to reach line 3 ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
}

// ---------------------------------------------------------------------------
// The divider between the left pane and the diff pane (#125).
// ---------------------------------------------------------------------------

#[test]
fn the_clamp_keeps_both_panes_above_the_floor() {
    // A width the reader could ask for, on a screen with room for it.
    assert_eq!(clamp_cols(60, 100), 60);
    // Neither end may squeeze a pane below the floor.
    assert_eq!(clamp_cols(0, 100), MIN_PANE);
    assert_eq!(clamp_cols(200, 100), 100 - MIN_PANE);
    // A screen too narrow for two floors gets half each. Handing the floor to
    // the left pane would starve the diff — at twenty columns it would leave
    // it nothing at all. Half is also what keeps this total: `u16::clamp`
    // panics when its low bound passes its high one, and this is the case
    // that would.
    assert_eq!(clamp_cols(40, 30), 15);
    assert_eq!(clamp_cols(1, 30), 15);
    assert_eq!(clamp_cols(40, 20), 10);
    assert_eq!(clamp_cols(40, 10), 5);
}

#[test]
fn the_divider_names_the_two_border_columns() {
    let panes = layout(SCREEN, DEFAULT_PLAN_COLS);
    let (left, right) = divider(&panes);
    // The left pane's last column and the diff pane's first: adjacent, and
    // the two the frames actually paint.
    assert_eq!(left, panes.plan.x + panes.plan.width - 1);
    assert_eq!(right, panes.detail.x);
    assert_eq!(left + 1, right);
}

#[test]
fn alt_equals_widens_the_diff_pane_and_alt_minus_narrows_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let before = app.panes().detail.width;

    app.handle_key(alt('='));
    let wider = app.panes().detail.width;
    assert!(
        wider > before,
        "alt-= should widen the diff pane: {before} -> {wider}"
    );

    app.handle_key(alt('-'));
    assert_eq!(
        app.panes().detail.width,
        before,
        "alt-- should undo what alt-= did"
    );

    // `+` is what most keyboards send for shifted `=`, and it is the same key.
    app.handle_key(alt('+'));
    assert_eq!(app.panes().detail.width, wider);
}

#[test]
fn the_divider_moves_from_either_pane() {
    // The one exception this key makes to "keys act on the pane you are in":
    // it names the diff pane from either side.
    let (_r, mut app) = make_app();
    sized(&mut app);
    let before = app.panes().detail.width;

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Detail);
    app.handle_key(alt('='));
    let from_diff = app.panes().detail.width;
    assert!(from_diff > before);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Groups);
    app.handle_key(alt('='));
    assert!(app.panes().detail.width > from_diff);
}

#[test]
fn the_panes_stay_adjacent_after_a_resize() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    for _ in 0..4 {
        app.handle_key(alt('='));
    }
    let panes = app.panes();
    assert_eq!(panes.plan.x + panes.plan.width, panes.detail.x);
    assert_eq!(panes.plan.width + panes.detail.width, SCREEN.width);
}

#[test]
fn a_resize_remeasures_the_diff_pane() {
    // The scroll budget counts screen lines at the pane's CONTENT width, so a
    // divider that moved without re-measuring would leave every wrapped row's
    // height stale.
    let (_r, mut app) = make_app();
    sized(&mut app);
    let before = app.viewport().detail_cols;
    app.handle_key(alt('='));
    let after = app.viewport().detail_cols;
    assert!(after > before, "{before} -> {after}");
    assert_eq!(after, app.panes().detail.width as usize - 2);
}

#[test]
fn the_divider_stops_at_the_edge_and_says_so() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    // Walk it out to the far edge. A press that changes nothing reads as a key
    // that does not work, so the footer has to answer for the one that cannot.
    for _ in 0..40 {
        app.handle_key(alt('='));
    }
    let pinned = app.panes().plan.width;
    assert_eq!(pinned, MIN_PANE);
    app.handle_key(alt('='));
    assert_eq!(app.panes().plan.width, pinned);
    assert!(
        app.status.contains("as wide as it goes"),
        "status was {:?}",
        app.status
    );

    for _ in 0..40 {
        app.handle_key(alt('-'));
    }
    assert_eq!(app.panes().detail.width, MIN_PANE);
    app.handle_key(alt('-'));
    assert!(
        app.status.contains("as narrow as it goes"),
        "status was {:?}",
        app.status
    );
}

#[test]
fn the_plan_and_the_tree_each_keep_their_own_divider() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let plan_default = app.panes().plan.width;

    // Widen the plan's own divider.
    app.handle_key(alt('-'));
    let plan_wide = app.panes().plan.width;
    assert!(plan_wide > plan_default);

    // `f` swaps the list in, and the divider with it — the tree has not been
    // moved, so it is still where it started.
    app.handle_key(key('f'));
    assert_eq!(app.view_mode, ViewMode::Files);
    assert_eq!(app.panes().plan.width, plan_default);

    // Move the tree's, independently.
    app.handle_key(alt('-'));
    app.handle_key(alt('-'));
    let tree_wide = app.panes().plan.width;
    assert!(tree_wide > plan_wide);

    // Back to the plan, and it kept its own.
    app.handle_key(key('f'));
    assert_eq!(app.panes().plan.width, plan_wide);
    app.handle_key(key('f'));
    assert_eq!(app.panes().plan.width, tree_wide);
}

#[test]
fn a_press_on_the_divider_grabs_it_and_a_drag_moves_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = app.panes().plan.y + 5;

    // Both columns the divider paints on grab it, and THE COLUMN GRABBED is
    // what ends up under the pointer. The two are one apart, so a drag that
    // assumed the left one ran a column ahead of every grab on the right.
    // Read afresh each time round, because the pass before moves them.
    for right_hand in [false, true] {
        for drop_at in [70, 30] {
            let (left, right) = divider(&app.panes());
            let grab = if right_hand { right } else { left };
            app.handle_mouse(click(grab, row));
            app.handle_mouse(drag(drop_at, row));

            let (left, right) = divider(&app.panes());
            let landed = if right_hand { right } else { left };
            assert_eq!(
                landed, drop_at,
                "the column grabbed stays under the pointer (right_hand={right_hand})"
            );
        }
    }
}

#[test]
fn a_press_on_the_divider_selects_no_row_and_takes_no_focus() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Detail);
    let cursor = app.cursor;
    let (left, _) = divider(&app.panes());

    app.handle_mouse(click(left, app.panes().plan.y + 5));
    assert_eq!(
        app.focus,
        Focus::Detail,
        "the grab is not a click on a pane"
    );
    assert_eq!(app.cursor, cursor);
}

#[test]
fn a_drag_that_did_not_start_on_the_divider_moves_nothing() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let before = app.panes().plan.width;
    let (x, y) = inside(app.panes().detail);

    // A press in a pane, then a drag across the divider's column: the reader
    // is selecting, or their hand slipped. Neither is a resize.
    app.handle_mouse(click(x, y));
    app.handle_mouse(drag(70, y));
    assert_eq!(app.panes().plan.width, before);

    // A bare drag, with no press at all, likewise.
    app.handle_mouse(drag(70, y));
    assert_eq!(app.panes().plan.width, before);
}

#[test]
fn the_drag_lets_go_on_the_next_press_elsewhere() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let (left, _) = divider(&app.panes());
    let row = app.panes().plan.y + 5;

    app.handle_mouse(click(left, row));
    app.handle_mouse(drag(60, row));
    let held = app.panes().plan.width;
    assert_eq!(held, 61);

    // A press somewhere else ends the gesture, so the drags after it are the
    // pane's business and not the divider's.
    let (x, y) = inside(app.panes().detail);
    app.handle_mouse(click(x, y));
    app.handle_mouse(drag(30, y));
    assert_eq!(app.panes().plan.width, held);
}

#[test]
fn a_drag_off_the_screen_stops_at_the_last_width_that_fits() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let (left, _) = divider(&app.panes());
    let row = app.panes().plan.y + 5;
    app.handle_mouse(click(left, row));

    app.handle_mouse(drag(SCREEN.width + 50, row));
    assert_eq!(app.panes().plan.width, SCREEN.width - MIN_PANE);
    app.handle_mouse(drag(0, row));
    assert_eq!(app.panes().plan.width, MIN_PANE);
}

#[test]
fn a_terminal_that_shrinks_pulls_the_divider_back_in() {
    // The width the reader asked for is kept; what the screen can show is
    // clamped. So a narrow window does not lose the divider off its edge, and
    // a window opened wide again puts it back where they left it.
    let (_r, mut app) = make_app();
    sized(&mut app);
    for _ in 0..6 {
        app.handle_key(alt('-'));
    }
    let wide = app.panes().plan.width;
    assert_eq!(wide, DEFAULT_PLAN_COLS + 24);

    app.set_area(Rect::new(0, 0, 50, 40));
    assert_eq!(app.panes().plan.width, 50 - MIN_PANE);

    app.set_area(SCREEN);
    assert_eq!(app.panes().plan.width, wide);
}

#[test]
fn the_divider_opens_at_the_default_every_sitting() {
    // Nothing about the divider reaches the sidecar: where it sits is a
    // reading position for this sitting, as the sideways shift is. So a fresh
    // reviewer opens at the default, and both lists do.
    let (_r, mut app) = make_app();
    sized(&mut app);
    assert_eq!(app.plan_cols(), DEFAULT_PLAN_COLS);
    app.handle_key(key('f'));
    assert_eq!(app.plan_cols(), DEFAULT_PLAN_COLS);
}

#[test]
fn the_default_viewport_is_what_a_real_measurement_gives() {
    // Forty tests set a synthetic `Viewport` and never measure. They are only
    // honest while the two agree, and the day `DEFAULT_PLAN_COLS` moves is the
    // day they quietly stop.
    let d = Viewport::default();
    assert_eq!(d, Viewport::measure(d.area, DEFAULT_PLAN_COLS));
}

#[test]
fn a_terminal_too_narrow_for_two_floors_splits_it_in_half() {
    let (_r, mut app) = make_app();
    app.set_area(Rect::new(0, 0, 30, 20));
    let panes = app.panes();
    assert_eq!(panes.plan.width, 15);
    assert_eq!(panes.detail.width, 15);
    // And the diff pane still has something inside its frame. Giving the floor
    // to the left pane instead would have left it three columns here, and
    // nothing at all on a twenty-column screen.
    assert!(app.viewport().detail_cols > 0);
    assert!(!screen(&app, 30, 20).is_empty());
}

#[test]
fn a_screen_too_narrow_to_move_the_divider_says_so() {
    let (_r, mut app) = make_app();
    app.set_area(Rect::new(0, 0, 30, 20));
    app.handle_key(alt('='));
    assert_eq!(app.panes().plan.width, 15);
    assert!(
        app.status.contains("too narrow"),
        "status was {:?}",
        app.status
    );
}

#[test]
fn a_shrunk_terminal_does_not_report_a_move_it_did_not_make() {
    // The width is clamped on the way out of the model, so a number the reader
    // chose on a wide screen cannot be read back as a divider the frame does
    // not show. Without that, this press wrote the clamped width back over the
    // stored one and reported a move that never happened.
    let (_r, mut app) = make_app();
    sized(&mut app);
    for _ in 0..8 {
        app.handle_key(alt('-'));
    }
    app.set_area(Rect::new(0, 0, 60, 40));
    let pinned = app.panes().plan.width;
    assert_eq!(pinned, 60 - MIN_PANE);

    app.handle_key(alt('-'));
    assert_eq!(app.panes().plan.width, pinned);
    assert!(
        app.status.contains("as narrow as it goes"),
        "status was {:?}",
        app.status
    );
}

#[test]
fn the_divider_is_where_the_draw_paints_it() {
    // The rule the mouse work established: every click number comes from the
    // component that draws it. So the columns `divider` names are the columns
    // the frames actually carry.
    let (_r, mut app) = make_app();
    sized(&mut app);
    for _ in 0..3 {
        app.handle_key(alt('='));
    }
    let (left, right) = divider(&app.panes());
    let rows = screen(&app, SCREEN.width, SCREEN.height);
    // A row inside both frames, clear of their titles and their feet.
    let row: Vec<char> = rows[5].chars().collect();
    assert_eq!(row[left as usize], '│', "the left pane's right border");
    assert_eq!(row[right as usize], '│', "the diff pane's left border");
}

#[test]
fn f_in_the_diff_pane_leaves_the_divider_alone() {
    // `f` acts on the pane it is pressed in: in the left pane it swaps the
    // list, and the divider with it; in the diff pane it opens the file list,
    // which is not a list swap and must move nothing.
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(alt('-'));
    let moved = app.panes().plan.width;

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    assert_eq!(app.panes().plan.width, moved);
    assert_eq!(app.view_mode, ViewMode::Groups);
}

#[test]
fn a_wrapped_row_re_wraps_when_the_divider_moves() {
    // With `w` on, a row's height is a function of the diff pane's content
    // width, and the scroll budget counts screen lines. A divider that moved
    // without re-measuring would leave every one of those heights stale.
    let (_r, mut app) = app_with_a_long_line();
    sized(&mut app);
    app.handle_key(key('w'));
    let wide = app.viewport().detail_cols;

    app.handle_key(alt('-'));
    let narrow = app.viewport().detail_cols;
    assert_eq!(narrow, wide - 4);
    assert_eq!(narrow, app.panes().detail.width as usize - 2);
}

#[test]
fn the_help_modal_names_the_divider_keys() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(key('?'));
    let text = screen(&app, SCREEN.width, SCREEN.height).join("\n");
    assert!(text.contains("alt-=/alt-- · alt-+"), "help was:\n{text}");
    // The key column is as wide as the widest key, so no key runs straight
    // into its own words.
    assert!(
        text.contains("alt-=/alt-- · alt-+  widen"),
        "the key and its words must not run together:\n{text}"
    );
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_split`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_split() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;
    for drop_at in [0u16, 45, 85] {
        if drop_at > 0 {
            let grab = app.split_column().expect("split view");
            app.handle_mouse(click(grab, row));
            app.handle_mouse(drag(drop_at, row));
        }
        println!(
            "\n== middle at column {:?} (offset {}) ==",
            app.split_column(),
            app.split_offset()
        );
        for line in screen(&app, SCREEN.width, 14) {
            println!("{line}");
        }
    }
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_divider`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_divider() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let steps: [(&str, i32); 3] = [
        ("opens at 40", 0),
        ("after three alt-=", 3),
        ("after six alt--", -6),
    ];
    for (label, presses) in steps {
        for _ in 0..presses.abs() {
            app.handle_key(alt(if presses > 0 { '=' } else { '-' }));
        }
        println!(
            "\n== {label} — left pane {} columns ==",
            app.panes().plan.width
        );
        for line in screen(&app, SCREEN.width, SCREEN.height) {
            println!("{line}");
        }
    }
}

// ---------------------------------------------------------------------------
// The split view's middle.
// ---------------------------------------------------------------------------

#[test]
fn the_middle_sits_at_the_centre_until_it_is_dragged() {
    // Offset zero must give exactly the arithmetic the split view had before
    // there was a drag, or every review opens looking different.
    for width in [20usize, 41, 58, 99] {
        let (lw, rw) = half_widths(width, 0);
        assert_eq!(lw, (width - 1) / 2);
        assert_eq!(lw + rw, width - 1, "the `│` takes the odd column");
    }
}

#[test]
fn the_middle_moves_by_its_offset_and_stops_at_each_floor() {
    let (lw, _) = half_widths(58, 10);
    assert_eq!(lw, 28 + 10);
    let (lw, _) = half_widths(58, -10);
    assert_eq!(lw, 28 - 10);

    // Neither half may be dragged below its floor.
    let (lw, rw) = half_widths(58, -100);
    assert_eq!(lw, MIN_HALF);
    assert_eq!(lw + rw, 57);
    let (lw, rw) = half_widths(58, 100);
    assert_eq!(rw, MIN_HALF);
    assert_eq!(lw + rw, 57);
}

#[test]
fn a_pane_too_narrow_for_two_halves_ignores_the_drag() {
    // The same answer it gave before there was a drag, rather than a clamp
    // that would panic with its low bound past its high one.
    let narrow = 2 * MIN_HALF;
    for offset in [-50, 0, 50] {
        let (lw, rw) = half_widths(narrow, offset);
        assert_eq!(lw, (narrow - 1) / 2);
        assert_eq!(lw + rw, narrow - 1);
    }
}

#[test]
fn a_unified_diff_has_no_middle_to_grab() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    assert!(app.split_column().is_some());
    app.handle_key(key('s'));
    assert!(!app.wrap_on_for_test());
    assert_eq!(app.split_column(), None, "unified is one column");
    app.handle_key(key('s'));
    assert!(app.split_column().is_some());
}

#[test]
fn the_middle_is_where_the_draw_paints_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;
    let grab = app.split_column().expect("split view");
    app.handle_mouse(click(grab, row));
    app.handle_mouse(drag(85, row));

    let at = app.split_column().expect("split view");
    let rows = screen(&app, SCREEN.width, SCREEN.height);
    // The `old`/`new` header is the row that names both halves, so it is the
    // one that must carry the middle between them.
    let header = rows
        .iter()
        .find(|r| r.contains(" old ") && r.contains(" new "))
        .expect("the split header row");
    let chars: Vec<char> = header.chars().collect();
    assert_eq!(chars[at as usize], '│', "header was {header:?}");
}

#[test]
fn a_press_on_the_middle_grabs_it_and_a_drag_moves_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;

    for drop_at in [85u16, 60, 70] {
        let grab = app.split_column().expect("split view");
        app.handle_mouse(click(grab, row));
        app.handle_mouse(drag(drop_at, row));
        assert_eq!(
            app.split_column(),
            Some(drop_at),
            "the middle follows the pointer"
        );
    }
}

#[test]
fn a_press_that_is_not_on_the_middle_does_not_grab_it() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;
    let before = app.split_column();

    // One column either side of it, and a press on a pane's own content.
    for miss in [before.unwrap() - 1, before.unwrap() + 1] {
        app.handle_mouse(click(miss, row));
        app.handle_mouse(drag(85, row));
        assert_eq!(app.split_column(), before);
    }
}

#[test]
fn the_middle_is_not_grabbable_off_the_end_of_its_line() {
    // The status row is under the diff pane's frame, and the middle is not
    // drawn there. A press on nothing grabs nothing, whatever column it is in.
    let (_r, mut app) = make_app();
    sized(&mut app);
    let before = app.split_column();
    let at = before.unwrap();
    app.handle_mouse(click(at, app.panes().status.y));
    app.handle_mouse(drag(85, app.panes().status.y));
    assert_eq!(app.split_column(), before);
}

#[test]
fn the_middle_keeps_its_skew_when_the_pane_divider_moves() {
    // Stored as a distance from the centre, so widening the diff pane gives
    // both halves the new room and leaves the skew the reader chose alone.
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;
    let grab = app.split_column().expect("split view");
    app.handle_mouse(click(grab, row));
    app.handle_mouse(drag(80, row));
    let offset = app.split_offset();
    assert!(offset > 0);

    app.handle_key(alt('='));
    assert_eq!(app.split_offset(), offset, "the skew is kept");
    let inner = pane_inner(app.panes().detail);
    let (lw, _) = half_widths(inner.width as usize, offset);
    assert_eq!(app.split_column(), Some(inner.x + lw as u16));
}

#[test]
fn s_to_unified_and_back_keeps_the_middle() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let row = pane_inner(app.panes().detail).y + 4;
    let grab = app.split_column().expect("split view");
    app.handle_mouse(click(grab, row));
    app.handle_mouse(drag(80, row));
    let at = app.split_column();

    app.handle_key(key('s'));
    app.handle_key(key('s'));
    assert_eq!(app.split_column(), at);
}

#[test]
fn the_middle_opens_at_the_centre_every_sitting() {
    // Nothing about it reaches the sidecar, as nothing about the pane divider
    // does: where the middle sits is a reading position for this sitting.
    let (_r, mut app) = make_app();
    sized(&mut app);
    assert_eq!(app.split_offset(), 0);
}

#[test]
fn moving_the_middle_re_wraps_the_rows_it_narrows() {
    // A row's height is a function of the columns its halves draw at, and the
    // scroll budget counts screen lines. A middle that moved without
    // re-measuring would leave every one of those heights stale.
    let (_r, mut app) = app_with_a_long_line();
    sized(&mut app);
    // The fixture opens unified; the middle only exists in the split view.
    app.handle_key(key('s'));
    app.handle_key(key('w'));
    // A DIFF row, because a banner is built to the pane and has no halves.
    let row = (0..app.rows.len())
        .filter(|&i| matches!(app.rows[i].kind, RowKind::Diff(_)))
        .max_by_key(|&i| app.row_height(i))
        .expect("a diff row");
    let tall = app.row_height(row);
    assert!(tall > 1, "the fixture must have a diff row that wraps");

    let y = pane_inner(app.panes().detail).y + 4;
    let move_middle_to = |app: &mut App, x: u16| {
        let grab = app.split_column().expect("split view");
        app.handle_mouse(click(grab, y));
        app.handle_mouse(drag(x, y));
    };

    // The long line is on the NEW side. Dragging the middle right narrows that
    // half, so the row takes more screen lines.
    move_middle_to(&mut app, 85);
    let narrowed = app.row_height(row);
    assert!(
        narrowed > tall,
        "a narrower half wraps into more lines: {tall} -> {narrowed}"
    );

    // And left again gives the room back.
    move_middle_to(&mut app, 50);
    let widened = app.row_height(row);
    assert!(
        widened < tall,
        "a wider half wraps into fewer: {tall} -> {widened}"
    );
}

#[test]
fn both_dividers_are_held_to_one_rule() {
    // `clamp_cols` and `half_widths` are the same question with different
    // spans and floors. The fallback is why it is one function: `clamp` panics
    // when its low bound passes its high one, and a second copy is a second
    // chance to leave that guard out.
    assert_eq!(split_point(50, 100, 20), 50);
    assert_eq!(split_point(5, 100, 20), 20);
    assert_eq!(split_point(95, 100, 20), 80);
    // Too narrow for two floors: half each, and no panic.
    assert_eq!(split_point(5, 30, 20), 15);
    assert_eq!(split_point(99, 30, 20), 15);
    assert_eq!(split_point(0, 0, 20), 0);

    // And the two callers are it, in their own units.
    for width in [10u16, 30, 100, 200] {
        for cols in [0u16, 20, 40, 300] {
            assert_eq!(
                clamp_cols(cols, width),
                split_point(cols.into(), width.into(), MIN_PANE.into()) as u16
            );
        }
    }
    for width in [10usize, 17, 58, 200] {
        for split in [-99i16, -4, 0, 4, 99] {
            let (lw, rw) = half_widths(width, split);
            let want = half_centre(width).saturating_add_signed(split as isize);
            assert_eq!(lw, split_point(want, width - 1, MIN_HALF));
            assert_eq!(lw + rw, width - 1, "the `│` keeps its own column");
        }
    }
}

/// Options whose keymap is the defaults with `[keys]` laid over them.
fn with_keys(pairs: &[(Action, &[&str])]) -> ReviewOptions {
    let config = KeysConfig(
        pairs
            .iter()
            .map(|(a, ks)| (*a, ks.iter().map(|k| k.to_string()).collect()))
            .collect(),
    );
    ReviewOptions {
        keymap: Keymap::new(&config).expect("a good [keys] table"),
        ..ReviewOptions::default()
    }
}

/// `back` rebinds like any action, and the float's edge names the keys the
/// reader bound rather than the defaults.
#[test]
fn back_and_the_floats_keys_follow_the_keys_table() {
    let opts = ReviewOptions {
        // `m`, not `e`: `e` is `external-editor`'s default, so binding
        // `close` to it would be a clash rather than a rebinding.
        keymap: with_keys(&[(Action::Back, &["backspace"]), (Action::Close, &["m"])]).keymap,
        ..laid_out(false)
    };
    let (_r, mut app) = app_with_symbols_and(opts);
    sized(&mut app);
    let call = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    let area = app
        .peek_area(app.panes().detail)
        .expect("the float has a home");
    let edge = &screen(&app, SCREEN.width, SCREEN.height)[(area.bottom() - 1) as usize];
    assert!(
        edge.trim_end().ends_with("enter go to  ·  m close ┘"),
        "the edge names the bound close key: {edge:?}"
    );

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(app.cursor, call);
    app.handle_key(ctrl('o'));
    assert_ne!(app.cursor, call, "ctrl-o is not back any more");
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(app.cursor, call, "backspace is");
}

/// An `e` that lands somewhere, from every kind of row.
///
/// The key's whole promise is that it always has an answer in the diff pane,
/// so each narrowing step of `editor_target` gets a row that reaches it.
fn with_editor() -> ReviewOptions {
    ReviewOptions {
        editor: Some(
            differential_engine::config::EditorCommand::parse("nvim +{line} {file}", "test")
                .unwrap(),
        ),
        ..ReviewOptions::default()
    }
}

#[test]
fn a_rebound_key_does_the_action_and_the_old_one_does_nothing() {
    let (_r, mut app) = make_app_with(with_keys(&[(Action::Down, &["m"])]));
    assert_eq!(app.selected_group, 0);
    app.handle_key(key('j'));
    assert_eq!(app.selected_group, 0, "j is not down any more");
    app.handle_key(key('m'));
    assert_eq!(app.selected_group, 1, "m is");

    // The same action in a list: the rebinding holds wherever `down` works.
    // Unfolded first, so the list has more than the one exemplar's file.
    app.handle_key(key('z'));
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    let selected = |app: &App| match app.mode {
        Mode::FileList { selected, .. } => selected,
        _ => panic!("the file list is open"),
    };
    assert_eq!(selected(&app), 0);
    app.handle_key(key('j'));
    assert_eq!(selected(&app), 0, "j does nothing in the list either");
    app.handle_key(key('m'));
    assert_eq!(selected(&app), 1);
}

#[test]
fn the_footer_and_the_help_name_the_rebound_key() {
    let (_r, mut app) = make_app_with(with_keys(&[(Action::Open, &["o"])]));
    sized(&mut app);
    let footer = screen(&app, SCREEN.width, SCREEN.height)
        .last()
        .cloned()
        .unwrap();
    assert!(
        footer.contains("o open"),
        "the plan pane's footer: {footer:?}"
    );
    assert!(!footer.contains("enter open"), "{footer:?}");
    assert!(footer.trim_end().ends_with("? help"), "{footer:?}");

    app.handle_key(key('?'));
    assert!(matches!(app.mode, Mode::Help(_)));
    let text = screen(&app, SCREEN.width, SCREEN.height);
    let key_of = |words: &str| {
        let row = text.iter().find(|r| r.contains(words)).expect(words);
        let inside = row.split('│').find(|c| c.contains(words)).unwrap();
        inside.split_whitespace().next().unwrap().to_string()
    };
    assert_eq!(key_of("open the group or file"), "o", "{text:#?}");
    assert_eq!(key_of("these keys"), "?", "help is fixed");
}

#[test]
fn a_click_on_a_footer_button_presses_the_rebound_key() {
    let (_r, mut app) = make_app_with(with_keys(&[(Action::Open, &["o"])]));
    sized(&mut app);
    let panes = layout(SCREEN, DEFAULT_PLAN_COLS);
    let (hints, x0) = app.status_hints(panes.status);
    let open = hints.first().expect("`o open` leads the plan pane's keys");
    assert_eq!(open.presses, vec![key('o')]);
    assert_eq!(app.focus, Focus::Groups);
    app.handle_mouse(click(x0, panes.status.y));
    assert_eq!(app.focus, Focus::Detail, "the click pressed o");
}

#[test]
fn q_and_question_mark_are_the_reviewers_own() {
    use differential_tui::keymap::KeyProblem;
    for key in ["q", "?"] {
        let taken = KeysConfig(
            [(Action::Copy, vec![key.to_string()])]
                .into_iter()
                .collect(),
        );
        let err = Keymap::new(&taken).unwrap_err();
        assert!(matches!(err.0[..], [KeyProblem::Reserved { .. }]), "{err}");
    }
    let (_r, mut app) = make_app();
    // `q` closes a list, and quits from the review.
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    assert!(app.handle_key(key('q')).is_empty());
    assert!(matches!(app.mode, Mode::Normal), "q closed the list");
    // `?` opens help from a list too, with the shift a terminal sends.
    app.handle_key(key('f'));
    app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT));
    assert!(matches!(app.mode, Mode::Help(_)));
    app.handle_key(key('x'));
    assert!(
        matches!(app.mode, Mode::FileList { .. }),
        "and gives it back"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.handle_key(key('q'))[..], [Effect::Quit]));
}

#[test]
fn an_unbound_key_leaves_the_footer_and_the_messages() {
    let (_r, mut app) = make_app_with(with_keys(&[(Action::ToggleWrap, &[])]));
    sized(&mut app);
    app.focus = Focus::Detail;
    app.handle_key(key('w'));
    assert!(!app.wrap_on_for_test(), "w is unbound");
    // No message names a key that does nothing.
    let help = {
        app.handle_key(key('?'));
        screen(&app, SCREEN.width, SCREEN.height).join("\n")
    };
    assert!(!help.contains("soft wrap long lines"), "{help}");
}

#[test]
fn the_rows_name_the_rebound_fold_key() {
    let (_r, mut app) = make_app_with(with_keys(&[(Action::Fold, &["o"])]));
    app.handle_key(key('j'));
    put_cursor_on(&mut app, |k| *k == RowKind::Fold);
    // Wide, so the hint at the fold row's end is on the screen.
    let text = screen(&app, 200, 40).join("\n");
    assert!(text.contains("o to show"), "{text}");
    assert!(!text.contains("z to show"), "{text}");
    app.handle_key(key('o'));
    assert!(
        !app.rows.iter().any(|r| r.kind == RowKind::Fold),
        "o opened it"
    );
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_keys`
///
/// The help modal and the footer, with the default keys and then with a
/// `[keys]` table laid over them.
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_keys() {
    let dump = |title: &str, opts: ReviewOptions, help: KeyEvent| {
        let (_r, mut app) = make_app_with(opts);
        sized(&mut app);
        println!("── {title}: the plan pane ──");
        println!(
            "{}",
            screen(&app, SCREEN.width, SCREEN.height).last().unwrap()
        );
        app.focus = Focus::Detail;
        println!("── {title}: the diff pane ──");
        println!(
            "{}",
            screen(&app, SCREEN.width, SCREEN.height).last().unwrap()
        );
        app.handle_key(key('f'));
        println!("── {title}: the file list ──");
        for row in screen(&app, SCREEN.width, SCREEN.height)
            .iter()
            .rev()
            .take(4)
            .rev()
        {
            println!("{row}");
        }
        app.mode = Mode::Normal;
        app.focus = Focus::Groups;
        app.handle_key(help);
        println!("── {title}: help in the plan pane ──");
        for row in screen(&app, SCREEN.width, SCREEN.height) {
            println!("{row}");
        }
    };
    dump("defaults", ReviewOptions::default(), key('?'));
    dump(
        "rebound",
        with_keys(&[
            (Action::Down, &["m", "down"]),
            (Action::Up, &["u", "up"]),
            (Action::NextGroup, &["ctrl-n"]),
            (Action::PrevGroup, &["ctrl-p"]),
            (Action::Delete, &["X"]),
            (Action::Findings, &["ctrl-f"]),
            (Action::Close, &["esc", "ctrl-g"]),
        ]),
        key('?'),
    );
}

/// Type `text` on whatever has the keyboard, then `enter`.
fn type_and_enter(app: &mut App, text: &str) -> Vec<Effect> {
    for c in text.chars() {
        app.handle_key(key(c));
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
}

/// `:` then a command, run.
fn command(app: &mut App, text: &str) -> Vec<Effect> {
    app.handle_key(key(':'));
    assert!(matches!(app.mode, Mode::Command(_)), "`:` opens the line");
    type_and_enter(app, text)
}

#[test]
fn colon_opens_the_command_line_and_esc_drops_it() {
    let (_r, mut app) = make_app();
    app.handle_key(key(':'));
    assert!(matches!(app.mode, Mode::Command(_)));
    // `?` and `q` are characters on the line, not help and quit.
    assert!(app.handle_key(key('q')).is_empty());
    assert!(matches!(app.mode, Mode::Command(_)));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
    // Backspace on an empty line leaves it, as vim's does.
    app.handle_key(key(':'));
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
}

#[test]
fn every_command_goes_where_its_key_does() {
    let (_r, mut app) = make_app();
    command(&mut app, "help");
    assert!(matches!(app.mode, Mode::Help(_)));
    app.handle_key(key('x'));

    command(&mut app, "config");
    assert!(matches!(app.mode, Mode::Config(_)));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    command(&mut app, "search helper");
    assert_eq!(app.search().expect("the search is open").query(), "helper");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    app.focus = Focus::Detail;
    command(&mut app, "files");
    assert!(matches!(app.mode, Mode::FileList { .. }));
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // A refusal is the key's own refusal, in the key's own words.
    command(&mut app, "findings");
    assert!(matches!(app.mode, Mode::Normal));
    assert!(app.status.contains("no findings yet"), "{}", app.status);
    command(&mut app, "publish");
    assert!(
        matches!(app.mode, Mode::Normal),
        "not a request: nothing to publish to"
    );

    command(&mut app, "nope");
    assert_eq!(app.status, "no command :nope · :help lists them");

    assert!(matches!(
        command(&mut app, "copy")[..],
        [Effect::CopySummary(_)]
    ));

    assert!(matches!(command(&mut app, "q")[..], [Effect::Quit]));
    assert!(matches!(command(&mut app, "quit")[..], [Effect::Quit]));
}

#[test]
fn tab_completes_a_commands_name() {
    let (_r, mut app) = make_app();
    app.handle_key(key(':'));
    app.handle_key(key('c'));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        matches!(app.mode, Mode::Config(_)),
        "`c` completed to config"
    );
}

#[test]
fn the_command_key_is_fixed_and_opens_from_a_list() {
    use differential_tui::keymap::KeyProblem;
    // `:` reaches `:config`, where a keymap is repaired: no table may take it.
    let taken = KeysConfig(
        [(Action::Search, vec![":".to_string()])]
            .into_iter()
            .collect(),
    );
    let err = Keymap::new(&taken).unwrap_err();
    assert!(matches!(err.0[..], [KeyProblem::Reserved { .. }]), "{err}");
    assert!(err.to_string().contains("command line"), "{err}");

    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;
    app.handle_key(key('f'));
    assert!(matches!(app.mode, Mode::FileList { .. }));
    // A terminal reports `:` with its shift; it is the same key.
    app.handle_key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::SHIFT));
    assert!(
        matches!(app.mode, Mode::Command(_)),
        "the line takes the list's place"
    );
}

/// Move the config modal's selection onto the row named `label`.
fn select_config_row(app: &mut App, label: &str) {
    app.handle_key(key('g'));
    for _ in 0..80 {
        if app
            .config_edit()
            .expect("the modal is open")
            .field()
            .label()
            == label
        {
            return;
        }
        app.handle_key(key('j'));
    }
    panic!("no config row {label}");
}

#[test]
fn the_config_modal_previews_and_esc_puts_it_back() {
    use differential_engine::config::ThemeName;
    let (_r, mut app) = make_app();
    command(&mut app, "config");
    select_config_row(&mut app, "theme");
    app.handle_key(key('l'));
    let previewed = app.options().theme;
    assert_ne!(
        previewed,
        ThemeName::Dark,
        "the theme changed under the modal"
    );
    select_config_row(&mut app, "context");
    app.handle_key(key('l'));
    assert_eq!(app.options().context, 4, "context previews too");

    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(matches!(app.mode, Mode::Normal));
    assert_eq!(
        app.options().theme,
        ThemeName::Dark,
        "esc restores the theme"
    );
    assert_eq!(app.options().context, 3, "and the context");
    assert!(app.status.contains("nothing was saved"), "{}", app.status);
}

/// Retype the selected config row as `text`.
fn retype_config_row(app: &mut App, text: &str) {
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for _ in 0..60 {
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    }
    type_and_enter(app, text);
}

#[test]
fn a_clashing_key_blocks_the_save_and_says_why() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("config.toml");
    let (_r, mut app) = make_app_with(ReviewOptions {
        user_config_path: Some(path.clone()),
        ..ReviewOptions::default()
    });
    command(&mut app, "config");
    select_config_row(&mut app, "delete");
    retype_config_row(&mut app, "[\"j\"]");
    let edit = app.config_edit().unwrap();
    assert!(
        edit.problems
            .iter()
            .any(|p| p.contains("down") && p.contains("delete")),
        "{:?}",
        edit.problems
    );
    app.handle_key(ctrl('s'));
    assert!(matches!(app.mode, Mode::Config(_)), "still open");
    assert!(app.status.starts_with("not saved"), "{}", app.status);
    assert!(!path.exists(), "nothing was written");
}

/// `back` is a row of the config modal like any action, and a key saved
/// there is the key `back` answers to at once.
#[test]
fn back_is_rebound_from_the_config_modal() {
    use differential_engine::config::Config;
    use differential_engine::store::OsConfigSource;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("config.toml");
    let (_r, mut app) = app_with_symbols_and(ReviewOptions {
        user_config_path: Some(path.clone()),
        ..laid_out(false)
    });
    command(&mut app, "config");
    select_config_row(&mut app, "back");
    retype_config_row(&mut app, "[\"backspace\"]");
    assert!(app.config_edit().unwrap().problems.is_empty());
    app.handle_key(ctrl('s'));
    assert!(app.status.starts_with("saved"), "{}", app.status);
    let saved = Config::load_user(&OsConfigSource, Some(&path)).unwrap();
    assert_eq!(saved.keys.0[&Action::Back], ["backspace"]);

    let call = cursor_on_text(&mut app, "helper_one() + helper_two()");
    app.handle_key(key('z'));
    assert!(app.peek.is_some(), "the float opened");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_ne!(app.cursor, call);
    app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(app.cursor, call, "the saved key goes back");
}

#[test]
fn saving_writes_the_whole_file_and_applies_it_at_once() {
    use differential_engine::config::{Config, ThemeName};
    use differential_engine::store::OsConfigSource;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("differential").join("config.toml");
    let (_r, mut app) = make_app_with(ReviewOptions {
        user_config_path: Some(path.clone()),
        ..ReviewOptions::default()
    });
    command(&mut app, "config");
    select_config_row(&mut app, "theme");
    app.handle_key(key('l'));
    let theme = app.options().theme;
    select_config_row(&mut app, "down");
    // `m`, not `e`: `e` is `external-editor`'s default, and binding `down`
    // to it would be a clash — which is a different test's subject.
    retype_config_row(&mut app, "[\"m\", \"down\"]");
    // `j` is free now, but nothing else asks for it: no clash.
    assert!(app.config_edit().unwrap().problems.is_empty());
    app.handle_key(ctrl('s'));
    assert!(matches!(app.mode, Mode::Normal), "{}", app.status);
    assert!(app.status.starts_with("saved"), "{}", app.status);

    let saved = Config::load_user(&OsConfigSource, Some(&path)).unwrap();
    assert_eq!(saved.review.theme, theme);
    assert_ne!(theme, ThemeName::Dark);
    assert_eq!(saved.keys.0[&Action::Down], ["m", "down"]);
    assert_eq!(&saved, &app.options().user_config);

    // The keys apply now, not next time.
    assert_eq!(app.selected_group, 0);
    app.handle_key(key('m'));
    assert_eq!(app.selected_group, 1);
}

#[test]
fn saving_with_nowhere_to_save_is_refused() {
    let (_r, mut app) = make_app();
    command(&mut app, "config");
    select_config_row(&mut app, "context");
    app.handle_key(key('l'));
    app.handle_key(ctrl('s'));
    assert!(matches!(app.mode, Mode::Config(_)));
    assert!(app.status.contains("no config directory"), "{}", app.status);
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_command_line`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_command_line() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    let bottom = |app: &App, n: usize| {
        let rows = screen(app, SCREEN.width, SCREEN.height);
        for row in &rows[rows.len() - n..] {
            println!("{row}");
        }
    };
    app.handle_key(key(':'));
    println!("── the command line, empty ──");
    bottom(&app, 12);
    app.handle_key(key('f'));
    println!("── typed `f` ──");
    bottom(&app, 6);
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    println!("── `f`, tab, tab ──");
    bottom(&app, 6);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key('?'));
    println!("── help, with the commands row ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_config_modal`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_config_modal() {
    let (_r, mut app) = make_app_with(ReviewOptions {
        user_config_path: Some("/home/reader/.config/differential/config.toml".into()),
        ..ReviewOptions::default()
    });
    sized(&mut app);
    command(&mut app, "config");
    println!("── opened ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
    select_config_row(&mut app, "theme");
    app.handle_key(key('l'));
    println!("── theme cycled once ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(key('j'));
    app.handle_key(key('j'));
    println!("── the theme list, two down ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    select_config_row(&mut app, "delete");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    println!("── typing a key row ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
    for _ in 0..60 {
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    }
    type_and_enter(&mut app, "[\"j\"]");
    println!("── a clash, wrapped ──");
    for row in screen(&app, SCREEN.width, SCREEN.height) {
        println!("{row}");
    }
}

#[test]
fn the_command_line_lists_what_the_name_could_become() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    app.handle_key(key(':'));
    let listed = |app: &App| -> Vec<&'static str> {
        app.command()
            .unwrap()
            .candidates()
            .iter()
            .map(|c| c.name)
            .collect()
    };
    assert_eq!(listed(&app).len(), 9, "an empty line lists every command");
    app.handle_key(key('f'));
    assert_eq!(listed(&app), ["findings", "files"]);
    let text = screen(&app, SCREEN.width, SCREEN.height).join("\n");
    assert!(
        text.contains(":findings") && text.contains("every finding and thread"),
        "{text}"
    );
    assert!(
        text.contains(":f▏indings"),
        "the dim rest follows the caret:\n{text}"
    );

    // Tab walks what `f` matched, filling the line; the list stays.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(app.command().unwrap().input.value(), "files");
    assert_eq!(listed(&app), ["findings", "files"]);
    app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(app.command().unwrap().input.value(), "findings");

    // `→` takes the dim rest.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.handle_key(key(':'));
    app.handle_key(key('c'));
    app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(app.command().unwrap().input.value(), "config");

    // An argument ends the list: it is the reader's own text.
    app.handle_key(key(' '));
    assert!(listed(&app).is_empty());
}

#[test]
fn the_theme_list_wears_each_theme_and_esc_goes_back() {
    use differential_engine::config::ThemeName;
    let (_r, mut app) = make_app();
    command(&mut app, "config");
    select_config_row(&mut app, "theme");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.config_edit().unwrap().dropdown.is_some(),
        "enter opens the list"
    );
    app.handle_key(key('j'));
    assert_eq!(app.options().theme, ThemeName::ALL[1], "a step wears it");
    app.handle_key(key('j'));
    assert_eq!(app.options().theme, ThemeName::ALL[2]);
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        app.config_edit().unwrap().dropdown.is_none(),
        "esc closes the list only"
    );
    assert_eq!(
        app.options().theme,
        ThemeName::Dark,
        "and goes back to the theme before"
    );

    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(key('G'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.config_edit().unwrap().dropdown.is_none());
    assert_eq!(
        app.config_edit().unwrap().draft.review.theme,
        ThemeName::Monokai,
        "enter keeps it"
    );
    assert_eq!(app.options().theme, ThemeName::Monokai);
}

#[test]
fn r_puts_any_setting_back_to_its_default() {
    let (_r, mut app) = make_app();
    command(&mut app, "config");
    for label in [
        "agent",
        "timeout_secs",
        "theme",
        "diff",
        "context",
        "context_step",
    ] {
        select_config_row(&mut app, label);
        app.handle_key(key('l'));
        let (_, default) = app
            .config_edit()
            .unwrap()
            .value(app.config_edit().unwrap().field());
        assert!(!default, "{label} changed");
        app.handle_key(key('r'));
        let (_, default) = app
            .config_edit()
            .unwrap()
            .value(app.config_edit().unwrap().field());
        assert!(default, "r reset {label}");
    }
    select_config_row(&mut app, "down");
    retype_config_row(&mut app, "[\"m\"]");
    app.handle_key(key('r'));
    let edit = app.config_edit().unwrap();
    assert!(edit.draft.keys.is_empty(), "r drops a key override");
    assert!(!edit.dirty(), "every row is back where it started");
}

#[test]
fn the_config_modals_errors_wrap_and_are_read_to_the_end() {
    let (_r, mut app) = make_app();
    sized(&mut app);
    command(&mut app, "config");
    // A value the row refuses: said in the modal, whole, and the box stays
    // open on what was typed.
    select_config_row(&mut app, "down");
    retype_config_row(&mut app, "ctrl-j");
    let edit = app.config_edit().unwrap();
    assert!(edit.editing.is_some(), "the box stays open");
    assert!(edit.error.is_some());
    let text = screen(&app, SCREEN.width, SCREEN.height).join("\n");
    assert!(
        text.contains("[\"j\", \"down\"]"),
        "the whole hint is on screen:\n{text}"
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        app.config_edit().unwrap().error.is_none(),
        "esc drops it with the edit"
    );

    // A clash in two screens: both messages, every word of them — and the
    // row that caused it still in view above them.
    select_config_row(&mut app, "delete");
    retype_config_row(&mut app, "[\"j\"]");
    let rows = screen(&app, SCREEN.width, SCREEN.height);
    assert!(
        rows.iter()
            .any(|r| r.contains("delete") && r.contains("[\"j\"]")),
        "the selected row scrolled away:\n{}",
        rows.join("\n")
    );
    let flat = screen(&app, SCREEN.width, SCREEN.height)
        .iter()
        .map(|r| r.split('│').nth(1).unwrap_or_default().trim().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    for p in &app.config_edit().unwrap().problems {
        let words: Vec<&str> = p.split_whitespace().collect();
        assert!(
            flat.contains(&words.join(" ")),
            "{p:?} is not on screen in full:\n{flat}"
        );
    }
}

/// The path and the line an `e` press asks for, or nothing.
fn pressed_e(app: &mut App) -> Option<(String, u32)> {
    match app.handle_key(key('e')).as_slice() {
        [Effect::OpenInEditor { path, line }] => Some((path.clone(), *line)),
        [] => None,
        other => panic!("e produced {} effects", other.len()),
    }
}

#[test]
fn e_opens_the_new_side_line_under_the_cursor() {
    let (_r, mut app) = make_app_with(with_editor());
    app.focus = Focus::Detail;
    // Walk to a row that IS a line, which is the common case and the only one
    // that can answer exactly.
    let row = app
        .rows
        .iter()
        .position(|r| {
            r.line
                .as_ref()
                .is_some_and(|l| l.side == "new" || l.other.is_some_and(|(s, _)| s == "new"))
        })
        .expect("a fixture with no new-side row tests nothing");
    app.cursor = row;
    let want = app.rows[row]
        .line
        .as_ref()
        .and_then(|l| l.line_on("new"))
        .unwrap();
    let (path, line) = pressed_e(&mut app).expect("a code row must open");
    assert_eq!(line, want, "e must take the row's own new-side line");
    assert!(app.files().iter().any(|f| f.path == path), "{path}");
}

#[test]
fn e_on_a_row_that_is_not_a_line_falls_back_to_the_hunk_then_the_file() {
    let (_r, mut app) = make_app_with(with_editor());
    app.focus = Focus::Detail;

    // A hunk header is not a line. It is inside a hunk, so the hunk's first
    // new-side line is the answer.
    let header = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .expect("every fixture file has a hunk header");
    app.cursor = header;
    let RowKind::HunkHeader { hunk, .. } = app.rows[header].kind else {
        unreachable!()
    };
    let want = {
        let h = &app.session.doc().hunks[hunk];
        (h.file.clone(), h.new_start.max(1))
    };
    assert_eq!(
        pressed_e(&mut app),
        Some(want),
        "a hunk header must open its hunk's first new-side line"
    );

    // A file header is inside no hunk at all, so the file opens at the top.
    let file_row = app
        .rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::FileHeader(_)))
        .expect("every fixture file has a file header");
    app.cursor = file_row;
    let RowKind::FileHeader(ref path) = app.rows[file_row].kind else {
        unreachable!()
    };
    let path = path.clone();
    assert_eq!(
        pressed_e(&mut app),
        Some((path, 1)),
        "a row under no hunk must open the file at the top"
    );
}

#[test]
fn e_in_the_plan_pane_opens_the_selected_file_and_refuses_a_group() {
    let (_r, mut app) = make_app_with(with_editor());
    app.focus = Focus::Groups;

    // The reading plan lists groups. A group is not a file, and saying so is
    // better than opening whatever the diff cursor happens to sit on.
    app.view_mode = ViewMode::Groups;
    assert_eq!(pressed_e(&mut app), None, "a group is not a file");
    assert_eq!(app.status, "no file here");

    // The file tree lists files. The selected one opens at the top: the plan
    // pane has no line to offer.
    app.handle_key(key('f'));
    assert_eq!(app.view_mode, ViewMode::Files);
    app.focus = Focus::Groups;
    let file = app
        .tree
        .iter()
        .position(|e| matches!(e.kind, differential_tui::app::TreeKind::File { .. }))
        .expect("the tree must hold a file");
    app.selected_file = file;
    let differential_tui::app::TreeKind::File { file_idx } = app.tree[file].kind else {
        unreachable!()
    };
    assert_eq!(
        pressed_e(&mut app),
        Some((app.files()[file_idx].path.clone(), 1)),
        "the plan pane opens the selected file at the top"
    );
}

/// With no editor resolved the key must say so. It is the one case where the
/// reader has done nothing wrong and the feature still cannot run, so a silent
/// no-op would read as a broken key.
#[test]
fn e_with_no_editor_configured_says_so_and_spawns_nothing() {
    let (_r, mut app) = make_app();
    app.focus = Focus::Detail;
    let row = app
        .rows
        .iter()
        .position(|r| r.line.is_some())
        .expect("a fixture with no line row tests nothing");
    app.cursor = row;
    assert_eq!(pressed_e(&mut app), None);
    assert!(app.status.contains("no editor"), "{}", app.status);
    assert!(app.status.contains("$EDITOR"), "{}", app.status);
}

/// `e` is an action like any other, so a reader may put it elsewhere. The
/// whole point of ADR 0036 is that nothing else has to be told.
#[test]
fn the_external_editor_key_is_rebindable() {
    let mut opts = with_keys(&[(Action::ExternalEditor, &["ctrl-e"])]);
    opts.editor = with_editor().editor;
    let (_r, mut app) = make_app_with(opts);
    app.focus = Focus::Detail;
    let row = app
        .rows
        .iter()
        .position(|r| r.line.is_some())
        .expect("a fixture with no line row tests nothing");
    app.cursor = row;

    assert!(app.handle_key(key('e')).is_empty(), "e is not bound now");
    let pressed = app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert!(
        matches!(pressed.as_slice(), [Effect::OpenInEditor { .. }]),
        "ctrl-e is"
    );

    // And the help names the bound key rather than the default, because the
    // row names the action and the keymap answers.
    sized(&mut app);
    app.handle_key(key('?'));
    let text = screen(&app, SCREEN.width, SCREEN.height);
    let row = text
        .iter()
        .find(|r| r.contains("open this line in your editor"))
        .expect("the help must name the action");
    assert!(row.contains("ctrl-e"), "{row:?}");
}

/// An action nobody can reach is a key that quietly does nothing, so an
/// unbound `external-editor` must leave `e` free rather than half-work.
#[test]
fn the_external_editor_key_can_be_unbound() {
    let mut opts = with_keys(&[(Action::ExternalEditor, &[])]);
    opts.editor = with_editor().editor;
    let (_r, mut app) = make_app_with(opts);
    app.focus = Focus::Detail;
    app.cursor = app.rows.iter().position(|r| r.line.is_some()).unwrap();
    assert!(app.handle_key(key('e')).is_empty());
    assert!(app.status.is_empty(), "an unbound key says nothing");
}

/// `?` must name the key, or a reader only finds it in the spec. The help
/// table is the one place the footer and the modal both read.
#[test]
fn the_help_table_names_e_in_both_panes() {
    let (_r, mut app) = make_app();
    for focus in [Focus::Groups, Focus::Detail] {
        app.focus = focus;
        let acts = app.acts();
        assert!(
            acts.iter().any(|a| a.key == "e"),
            "e missing from {focus:?}: {:?}",
            acts.iter().map(|a| a.key.clone()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn the_config_modal_names_its_whole_file() {
    let long =
        "/Users/someone-with-a-long-name/Library/Application Support/differential/config.toml";
    let (_r, mut app) = make_app_with(ReviewOptions {
        user_config_path: Some(long.into()),
        ..ReviewOptions::default()
    });
    sized(&mut app);
    command(&mut app, "config");
    let flat: String = screen(&app, SCREEN.width, SCREEN.height)
        .iter()
        .map(|r| r.split('│').nth(1).unwrap_or_default().trim().to_string())
        .collect();
    let squeezed: String = flat.split_whitespace().collect();
    assert!(
        squeezed.contains(&long.split_whitespace().collect::<String>()),
        "{flat}"
    );
    // The rows still answer clicks on the right line under a taller header.
    select_config_row(&mut app, "agent");
    assert_eq!(app.config_edit().unwrap().field().label(), "agent");
}

#[test]
fn every_multiple_choice_row_opens_a_list() {
    use differential_engine::config::Agent;
    let (_r, mut app) = make_app();
    sized(&mut app);
    command(&mut app, "config");

    select_config_row(&mut app, "agent");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let edit = app.config_edit().unwrap();
    assert!(edit.dropdown.is_some(), "enter on the agent opens its list");
    let text = screen(&app, SCREEN.width, SCREEN.height).join("\n");
    for a in Agent::ALL {
        assert!(text.contains(a.key()), "{} is listed:\n{text}", a.key());
    }
    app.handle_key(key('j'));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.config_edit().unwrap().draft.grouping.agent,
        Some(Agent::ALL[1])
    );

    // The diff layout too, previewed as it is passed and put back by esc.
    select_config_row(&mut app, "diff");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(key('j'));
    assert!(!app.options().split_diff, "unified is worn while it is on");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.options().split_diff, "esc goes back to split");

    // A typed row still types.
    select_config_row(&mut app, "context");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let edit = app.config_edit().unwrap();
    assert!(edit.dropdown.is_none() && edit.editing.is_some());
}

/// A removed line is the case the fallback exists for. In the unified layout
/// it carries no new-side number at all — the line is not in the file any
/// more — so the hunk's first new-side line is what `e` can honestly offer.
#[test]
fn e_on_a_removed_line_opens_the_hunk_it_was_cut_from() {
    let (_r, mut app) = make_app_with(ReviewOptions {
        split_diff: false,
        ..with_editor()
    });
    app.focus = Focus::Detail;
    let removed = app
        .rows
        .iter()
        .position(|r| {
            r.line
                .as_ref()
                .is_some_and(|l| l.side == "old" && l.other.is_none())
        })
        .expect("a unified fixture with no removed row tests nothing");
    app.cursor = removed;
    let hunk = app.rows[removed]
        .kind
        .hunk()
        .expect("a diff row has a hunk");
    let want = {
        let h = &app.session.doc().hunks[hunk];
        (h.file.clone(), h.new_start.max(1))
    };
    assert_eq!(
        pressed_e(&mut app),
        Some(want),
        "a removed line must open the hunk that cut it"
    );
}

/// `cargo test -p differential-tui --test tui -- --ignored --nocapture render_dump_editor_key`
#[ignore = "a dump for the author's eyes, not an assertion"]
#[test]
fn render_dump_editor_key() {
    let (_r, mut app) = make_app_with(with_editor());
    sized(&mut app);
    app.focus = Focus::Detail;
    app.handle_key(key('?'));
    println!("\n=== ? in the diff pane — the new `e` row ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.focus = Focus::Groups;
    app.handle_key(key('?'));
    println!("\n=== ? in the plan pane — the same key, the other words ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // Rebound: the row names the action, so the help follows the keymap.
    let mut rebound = with_keys(&[(Action::ExternalEditor, &["ctrl-e"])]);
    rebound.editor = with_editor().editor;
    let mut app = make_app_with(rebound).1;
    sized(&mut app);
    app.focus = Focus::Detail;
    app.handle_key(key('?'));
    println!("\n=== external-editor = [\"ctrl-e\"] — the help follows ===");
    for row in screen(&app, 100, 40) {
        println!("{row}");
    }
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // The `:config` row, unset and set.
    let mut cfg = make_app_with(ReviewOptions::default()).1;
    sized(&mut cfg);
    command(&mut cfg, "config");
    select_config_row(&mut cfg, "editor");
    println!("\n=== :config — the editor row, unset ===");
    for row in screen(&cfg, 100, 40) {
        println!("{row}");
    }
    retype_config_row(&mut cfg, "zed --wait {file}:{line}");
    println!("\n=== :config — the editor row, set ===");
    for row in screen(&cfg, 100, 40) {
        println!("{row}");
    }

    // The status the key writes when no editor is resolved. The rest of the
    // footer strings are the loop's, after a spawn a test cannot make; they
    // are pinned in `launch.rs`'s own unit test instead.
    app.focus = Focus::Detail;
    let mut bare = make_app_with(ReviewOptions::default()).1;
    sized(&mut bare);
    bare.focus = Focus::Detail;
    bare.handle_key(key('e'));
    println!("\n=== e with no editor resolved ===");
    println!("  {}", bare.status);
}

/// `:config` edits `[review].editor` like any other row: typed, validated on
/// enter, reset by `r`, saved with the rest of the file — and the command it
/// saves is the one `e` uses at once, not at the next start.
#[test]
fn the_config_modal_edits_the_editor_command() {
    use differential_engine::config::Config;
    use differential_engine::store::OsConfigSource;
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("differential").join("config.toml");
    let (_r, mut app) = make_app_with(ReviewOptions {
        user_config_path: Some(path.clone()),
        ..ReviewOptions::default()
    });
    sized(&mut app);
    command(&mut app, "config");

    // Unset, the row is empty and marked as the default, and the screen says
    // what unset MEANS — an empty row would read as "no editor at all".
    select_config_row(&mut app, "editor");
    let (value, default) = {
        let e = app.config_edit().unwrap();
        e.value(e.field())
    };
    assert_eq!(value, "");
    assert!(default);
    let text = screen(&app, SCREEN.width, SCREEN.height).join("\n");
    assert!(text.contains("$VISUAL, then $EDITOR"), "{text}");

    // A value the crate could not run is refused, in the modal, with the box
    // still open on what was typed.
    retype_config_row(&mut app, "{file}");
    let e = app.config_edit().unwrap();
    assert!(e.editing.is_some(), "the box stays open on a refusal");
    assert!(
        e.error.as_deref().is_some_and(|m| m.contains("first word")),
        "{:?}",
        e.error
    );
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    // A good one takes, and saves with the rest of the file.
    retype_config_row(&mut app, "nvim +{line} {file}");
    assert_eq!(
        app.config_edit().unwrap().draft.review.editor.as_deref(),
        Some("nvim +{line} {file}")
    );
    app.handle_key(ctrl('s'));
    assert!(app.status.starts_with("saved"), "{}", app.status);
    let saved = Config::load_user(&OsConfigSource, Some(&path)).unwrap();
    assert_eq!(saved.review.editor.as_deref(), Some("nvim +{line} {file}"));

    // And `e` uses it NOW. Before this row existed the reviewer held the
    // command the application layer resolved at start.
    app.focus = Focus::Detail;
    app.cursor = app.rows.iter().position(|r| r.line.is_some()).unwrap();
    assert!(
        matches!(
            app.handle_key(key('e')).as_slice(),
            [Effect::OpenInEditor { .. }]
        ),
        "the saved command is the one e uses"
    );
}

/// Clearing the row means "use the environment", not "no editor". The
/// application layer hands the environment's own answer over for this.
#[test]
fn clearing_the_editor_row_falls_back_to_the_environment() {
    use differential_engine::config::{EditorCommand, ReviewConfig, UserConfig};
    // The file names one and the environment names another, which is the only
    // state where "what does clearing the row mean" has two possible answers.
    let (_r, mut app) = make_app_with(ReviewOptions {
        editor: Some(EditorCommand::parse("nvim {file}", "test").unwrap()),
        editor_env: Some(EditorCommand::parse("vi", "test").unwrap()),
        user_config: UserConfig {
            review: ReviewConfig {
                editor: Some("nvim {file}".to_string()),
                ..ReviewConfig::default()
            },
            ..UserConfig::default()
        },
        ..ReviewOptions::default()
    });
    sized(&mut app);
    command(&mut app, "config");
    select_config_row(&mut app, "editor");
    retype_config_row(&mut app, "hx {file}:{line}");
    assert_eq!(app.options().editor.as_ref().unwrap().program(), "hx");

    // `r` is the reset, and the row's default is the environment's command —
    // not nothing, which would leave `e` dead for a reader who never wrote
    // `[review].editor` in the first place.
    app.handle_key(key('r'));
    assert_eq!(app.config_edit().unwrap().draft.review.editor, None);
    assert_eq!(app.options().editor.as_ref().unwrap().program(), "vi");

    // And `esc` puts back what the modal opened on, as it does for the theme.
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.options().editor.as_ref().unwrap().program(), "nvim");
}

// ------------------------------------------- joined directory chains (#155)

/// The issue's shape: `root/parent1/parent2` holds nothing but one directory
/// each, and branches only at `parent2`. `lib.rs` and `top.rs` sit beside
/// `root/`, so the chain is not the whole tree and not its first row — a
/// cursor that fell back to row 0 would not pass for one that found it.
///
/// Each file's edit is a different shape, so each file is its own class and
/// its own group, and the map can be drawn for exactly one file.
fn app_with_a_directory_chain(store: &str) -> (TestRepo, App) {
    let r = TestRepo::new();
    let files = [
        (
            "root/parent1/parent2/parent3/file1.rs",
            "fn f() { g(); }\n",
            "fn f() { h(); }\n",
        ),
        (
            "root/parent1/parent2/parent3/file2.rs",
            "let x = 1;\n",
            "let x = 2;\n",
        ),
        (
            "root/parent1/parent2/parent4/file3.rs",
            "use a::b;\n",
            "use a::c;\n",
        ),
        ("top.rs", "enum E { A, B }\n", "enum E { A, C }\n"),
        ("lib.rs", "const K: u8 = 1;\n", "const K: u8 = 2;\n"),
    ];
    for (path, before, _) in files {
        r.write(path, before.as_bytes());
    }
    r.commit_all("base");
    for (path, _, after) in files {
        r.write(path, after.as_bytes());
    }
    r.commit_all("head");
    let app = open_app_with(&r, &one_group_per_class(), store);
    (r, app)
}

/// The left `w` columns of the screen, trailing blanks trimmed.
fn left_pane(app: &App, w: usize) -> Vec<String> {
    screen(app, 100, 16)
        .into_iter()
        .map(|l| l.chars().take(w).collect::<String>().trim_end().to_string())
        .collect()
}

/// Park the plan pane on the group that owns `file`, so the map is drawn for it.
fn select_group_owning(app: &mut App, file: &str) {
    app.focus = Focus::Groups;
    for _ in 0..app.groups().len() {
        if screen(app, 100, 16)
            .iter()
            .any(|l| l.contains(&format!("● {file}")))
        {
            return;
        }
        app.handle_key(key('j'));
    }
    panic!("no group lights {file}");
}

const CHAIN: &str = "root/parent1/parent2";

/// A chain of directories that each hold one directory and nothing else is
/// ONE row, keyed by its deepest directory; the level that branches nests
/// beneath it as before, one step in (#155).
#[test]
fn a_chain_of_single_child_directories_is_one_row() {
    use differential_tui::app::TreeKind;
    let (_r, mut app) = app_with_a_directory_chain(".dfr-chain-row-store");
    switch_left_pane(&mut app);
    assert_eq!(
        tree_paths(&app),
        [
            "lib.rs",
            "root/parent1/parent2/",
            "root/parent1/parent2/parent3/",
            "root/parent1/parent2/parent3/file1.rs",
            "root/parent1/parent2/parent3/file2.rs",
            "root/parent1/parent2/parent4/",
            "root/parent1/parent2/parent4/file3.rs",
            "top.rs",
        ]
    );
    let depths: Vec<usize> = app.tree.iter().map(|e| e.depth).collect();
    assert_eq!(depths, [0, 0, 1, 2, 2, 1, 2, 0]);
    // A directory that holds files is never joined into them.
    assert!(matches!(&app.tree[2].kind, TreeKind::Dir { name, .. } if name == "parent3"));

    let pane = left_pane(&app, 40);
    assert!(
        pane.iter()
            .any(|l| l.contains("├─▾ root/parent1/parent2/  +3 −3")),
        "{pane:#?}"
    );
    assert!(
        pane.iter().any(|l| l.contains("│ ├─▾ parent3/")),
        "{pane:#?}"
    );
}

/// The joined row folds as one: everything beneath the chain goes, and comes
/// back, and selecting it shows every hunk under it.
#[test]
fn a_joined_row_folds_and_selects_as_one() {
    let (_r, mut app) = app_with_a_directory_chain(".dfr-chain-fold-store");
    switch_left_pane(&mut app);
    let before = tree_paths(&app);

    app.focus = Focus::Groups;
    app.selected_file = 0;
    app.handle_key(key('j'));
    assert_eq!(app.selected_path().as_deref(), Some(CHAIN));
    let hunks = app
        .rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::HunkHeader { .. }))
        .count();
    assert_eq!(hunks, 3, "the joined row spans all three files under it");

    fold(&mut app, CHAIN);
    assert_eq!(
        tree_paths(&app),
        [
            "lib.rs".to_string(),
            format!("{CHAIN}/"),
            "top.rs".to_string()
        ]
    );
    fold(&mut app, CHAIN);
    assert_eq!(tree_paths(&app), before, "unfold gives back the same rows");
}

/// The resume cursor keys on the joined row's path and comes back to it.
#[test]
fn the_resume_cursor_comes_back_to_a_joined_row() {
    let (r, mut app) = app_with_a_directory_chain(".dfr-chain-resume-store");
    switch_left_pane(&mut app);
    app.focus = Focus::Groups;
    app.handle_key(key('j'));
    assert_eq!(app.selected_path().as_deref(), Some(CHAIN));
    app.handle_key(key('q'));
    drop(app);

    let app2 = open_app_with(&r, &one_group_per_class(), ".dfr-chain-resume-store");
    assert_eq!(app2.selected_path().as_deref(), Some(CHAIN));
    drop(app2);

    // A cursor saved on a directory the chain now absorbs — as one written
    // before the join would be — lands on the joined row, not on the top.
    let state_path = r.root.join(".dfr-chain-resume-store/state.json");
    let state = std::fs::read_to_string(&state_path).unwrap();
    let quoted = format!("\"{CHAIN}\"");
    assert!(
        state.contains(&quoted),
        "the cursor names the chain: {state}"
    );
    std::fs::write(&state_path, state.replace(&quoted, "\"root/parent1\"")).unwrap();

    let app3 = open_app_with(&r, &one_group_per_class(), ".dfr-chain-resume-store");
    assert_eq!(app3.selected_path().as_deref(), Some(CHAIN));
}

/// The group map reads the same joined tree: a chain the group DOES enter is
/// one live row now, not one row per directory.
#[test]
fn the_group_map_joins_a_chain_the_group_enters() {
    let (_r, mut app) = app_with_a_directory_chain(".dfr-chain-map-store");
    select_group_owning(&mut app, "file3.rs");
    let rows = screen(&app, 100, 16);
    assert!(
        rows.iter().any(|l| l.contains("├─root/parent1/parent2/ ")),
        "{rows:#?}"
    );
    assert!(
        !rows.iter().any(|l| l.contains("└─parent1/")),
        "no row per directory in the chain: {rows:#?}"
    );
    assert!(
        rows.iter().any(|l| l.contains("▸ parent3/  2 files")),
        "{rows:#?}"
    );
}

/// Eyes-only: the file view and the group map on the issue's shape.
/// `cargo test -p differential-tui --test tui render_dump_joined_dirs -- --ignored --nocapture`
#[test]
#[ignore]
fn render_dump_joined_dirs() {
    let (_r, mut app) = app_with_a_directory_chain(".dfr-dump-chain-store");
    select_group_owning(&mut app, "file3.rs");
    println!("--- group map, on the group owning file3.rs");
    for l in screen(&app, 100, 16) {
        println!("{}", l.trim_end());
    }
    switch_left_pane(&mut app);
    app.focus = Focus::Groups;
    println!("--- file view");
    for l in left_pane(&app, 44) {
        println!("{l}");
    }
}

/// A line of wide characters, long enough that the preview shifts sideways
/// to show the hit. The preview measured columns and cut by bytes, so the cut
/// landed inside a character (a panic) or a full column off.
#[test]
fn the_search_preview_cuts_wide_text_by_columns() {
    let r = TestRepo::new();
    r.write("wide.txt", b"start\n");
    r.commit_all("base");
    let line = format!("{} needle\n", "中".repeat(70));
    r.write("wide.txt", format!("start\n{line}").as_bytes());
    r.commit_all("head");
    let mut app = open_app_with(&r, &skim_first_backend(), ".dfr-wide-store");
    sized(&mut app);
    search_for(&mut app, "needle");
    let rows = screen(&app, SCREEN.width, SCREEN.height);
    // The preview's line 2: the hit shifted into view, and the box's own
    // right edge still where it belongs — nothing overran it.
    let preview = rows
        .iter()
        .find(|r| r.contains("│ 2 │") && r.contains("needle"))
        .unwrap_or_else(|| panic!("the shifted preview shows the hit:\n{}", rows.join("\n")));
    let top = rows
        .iter()
        .find(|r| r.contains(" search "))
        .expect("the search box's top edge");
    let edge = top.chars().position(|c| c == '┐').expect("its corner");
    assert_eq!(
        preview.chars().nth(edge),
        Some('│'),
        "the preview row stops at the box's edge: {preview:?}"
    );
}
