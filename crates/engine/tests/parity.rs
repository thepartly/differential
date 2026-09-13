//! Real-corpus parity test. Committed code is fully generic: every repo path,
//! rev and expected number lives in an UNCOMMITTED local TOML (gitignored as
//! `*.local.toml`), pointed at by `DIFFERENTIAL_FIXTURE_CONFIG`. See
//! `fixtures.example.toml` at the workspace root for the shape, and
//! `differential_testutil::parity` for the harness both corpus tests share.
//!
//! Run with: DIFFERENTIAL_FIXTURE_CONFIG=… cargo test -- --ignored
//!
//! Files, hunks and classes only — what git and the frozen normaliser must
//! agree on. The dependency graph is NOT measured here: this crate owns the
//! port, not a reader, so the numbers it would produce belong to whichever
//! test double it wired. `crates/symbols/tests/parity.rs` measures the real
//! readers against the same fixture file.

use differential_testutil::parity::{load_fixtures, run_fixture};

#[test]
#[ignore = "needs DIFFERENTIAL_FIXTURE_CONFIG pointing at a local fixture file"]
fn real_corpus_parity() {
    let Some(fixtures) = load_fixtures() else {
        return;
    };

    for (i, fx) in fixtures.iter().enumerate() {
        let doc = run_fixture(i, fx, &differential_testutil::stub_readers());
        // Exact assertions: class-count drift is a normaliser-port bug, never
        // tolerance-adjusted away.
        assert_eq!(doc.stats.files, fx.expect.files, "fixture {i}: files");
        assert_eq!(doc.stats.hunks, fx.expect.hunks, "fixture {i}: hunks");
        assert_eq!(doc.stats.classes, fx.expect.classes, "fixture {i}: classes");
        assert_eq!(
            doc.audit.applier_exact, fx.expect.applier,
            "fixture {i}: applier"
        );
        assert_eq!(doc.audit.recount, fx.expect.recount, "fixture {i}: recount");
        assert_eq!(doc.audit.tree_assertion, "pass", "fixture {i}: tree");

        eprintln!(
            "fixture {i}: ok — {} files, {} hunks, {} classes",
            doc.stats.files, doc.stats.hunks, doc.stats.classes
        );
    }
}
