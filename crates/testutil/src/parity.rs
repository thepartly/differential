//! The real-corpus parity harness: the fixture file, and one fixture run.
//!
//! Committed code is fully generic: every repo path, rev and expected number
//! lives in an UNCOMMITTED local TOML (gitignored as `*.local.toml`), pointed
//! at by `DIFFERENTIAL_FIXTURE_CONFIG`. See `fixtures.example.toml` at the
//! workspace root for the shape. Two tests read it — the engine's, which pins
//! what git and the frozen normaliser must agree on, and the symbols crate's,
//! which measures the shipped readers — and this is the half they share. Each
//! keeps its own readers and its own assertions.

use std::path::Path;

use differential_engine::artefact::symbols::SymbolReaders;
use differential_engine::config::Config;
use differential_engine::gitio::Repo;
use differential_engine::lang::LanguageRegistry;
use differential_engine::pipeline::run_pipeline;
use differential_engine::plan::ReviewSource;
use differential_engine::schema::PlanDocument;
use serde::Deserialize;

#[derive(Deserialize)]
struct FixtureFile {
    #[serde(default)]
    fixture: Vec<Fixture>,
}

/// One `[[fixture]]` entry: a range in a local repository and what the
/// pipeline must say about it.
#[derive(Deserialize)]
pub struct Fixture {
    pub repo_path: String,
    pub base: String,
    pub head: String,
    pub expect: Expect,
}

/// The numbers a fixture pins. The first five are **facts** — what git and
/// the normaliser must agree on — and are required. `edges` and `sccs` pin
/// **heuristics**: they move whenever a reader changes, and are meant to, so
/// they are optional and read only by the symbols crate's test, where the
/// movement is deliberate and visible.
#[derive(Deserialize)]
pub struct Expect {
    pub files: u32,
    pub hunks: u32,
    pub classes: u32,
    pub applier: String,
    pub recount: u32,
    #[serde(default)]
    pub edges: Option<u32>,
    #[serde(default)]
    pub sccs: Option<u32>,
}

/// The fixtures `DIFFERENTIAL_FIXTURE_CONFIG` points at, or `None` — with a
/// line on stderr — when the variable is unset, so an ignored test can return
/// early instead of failing where no corpus exists.
pub fn load_fixtures() -> Option<Vec<Fixture>> {
    let Ok(cfg_path) = std::env::var("DIFFERENTIAL_FIXTURE_CONFIG") else {
        eprintln!("skipping: DIFFERENTIAL_FIXTURE_CONFIG is not set");
        return None;
    };
    let text = std::fs::read_to_string(&cfg_path)
        .unwrap_or_else(|e| panic!("cannot read {cfg_path}: {e}"));
    let fixtures: FixtureFile = toml::from_str(&text).expect("malformed fixture config");
    assert!(
        !fixtures.fixture.is_empty(),
        "fixture config contains no [[fixture]] entries"
    );
    Some(fixtures.fixture)
}

/// Fixture `i` through the pipeline and the verify stage with `readers`,
/// panicking on any failure and returning the document once every invariant
/// holds.
///
/// Invariants 3 and 4 build a tree, so they write and the pipeline no longer
/// runs them (ADR 0028). Both parity tests assert `all_ok`, which is never
/// true without them — so this has to ask, exactly as `dfr check` does.
/// Without it the corpus gate silently stopped checking the two invariants it
/// exists to check on real data.
pub fn run_fixture(i: usize, fx: &Fixture, readers: &SymbolReaders) -> PlanDocument {
    let repo_path = shellexpand_home(&fx.repo_path);
    let repo = Repo::open(Path::new(&repo_path))
        .unwrap_or_else(|e| panic!("fixture {i}: cannot open {repo_path}: {e}"));
    let mut out = run_pipeline(
        &repo,
        &ReviewSource::range(fx.base.clone(), fx.head.clone(), fx.head.clone()),
        &Config::default(),
        &LanguageRegistry::builtin(),
        readers,
    )
    .unwrap_or_else(|e| panic!("fixture {i}: pipeline failed: {e}"));
    differential_engine::verify(&repo, &mut out)
        .unwrap_or_else(|e| panic!("fixture {i}: verify failed: {e}"));

    assert!(
        out.report.all_ok(),
        "fixture {i}: invariants failed: {:#?}",
        out.report
    );
    out.document.expect("document")
}

fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    p.to_string()
}
