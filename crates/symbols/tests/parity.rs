//! Real-corpus measurement of the shipped readers.
//!
//! Committed code is fully generic: every repo path, rev and expected number
//! lives in an UNCOMMITTED local TOML (gitignored as `*.local.toml`), pointed
//! at by `DIFFERENTIAL_FIXTURE_CONFIG`. See `fixtures.example.toml` at the
//! workspace root for the shape, and `differential_testutil::parity` for the
//! harness both corpus tests share.
//!
//! Run with: DIFFERENTIAL_FIXTURE_CONFIG=… cargo test -- --ignored
//!
//! **`sccs` is the number that matters, not `edges`.** A topological sort works
//! if and only if every strongly connected component has size one. One knot of
//! twelve classes made every class with an out-edge unorderable on the second
//! corpus range, and the ordering stage fell back to sorting by size — which is
//! the failure it exists to fix. Removing edges is only useful insofar as it
//! removes the false ones holding that knot together.
//!
//! `edges` and `sccs` are both optional in the fixture file: they pin
//! **heuristics**, not facts. Files, hunks and classes are what git and the
//! normaliser must agree on, and the engine's own parity test owns those.
//! These two move whenever a reader changes, and they are meant to — they are
//! here so the movement is deliberate and visible.

use std::collections::HashMap;

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

use differential_testutil::parity::{load_fixtures, run_fixture};

#[test]
#[ignore = "needs DIFFERENTIAL_FIXTURE_CONFIG pointing at a local fixture file"]
fn real_corpus_graph() {
    let Some(fixtures) = load_fixtures() else {
        return;
    };

    for (i, fx) in fixtures.iter().enumerate() {
        let doc = run_fixture(i, fx, &differential_symbols::readers());

        let index: HashMap<&str, usize> = doc
            .classes
            .iter()
            .enumerate()
            .map(|(j, c)| (c.id.as_str(), j))
            .collect();
        // `depends_on` is already one entry per target, so no edge repeats.
        let mut graph = DiGraph::<(), ()>::new();
        let nodes: Vec<_> = doc.classes.iter().map(|_| graph.add_node(())).collect();
        for (j, c) in doc.classes.iter().enumerate() {
            for e in &c.depends_on {
                if let Some(&target) = index.get(e.on.as_str()) {
                    graph.add_edge(nodes[j], nodes[target], ());
                }
            }
        }
        // What the symbol index costs, printed rather than asserted: it is a
        // size to WATCH, not a number to freeze — every widening of what counts
        // as a declaration moves it, and a pinned figure would fail for reasons
        // that have nothing to do with the graph this test guards (ADR 0032).
        if let Some(ix) = &doc.symbols {
            let whole = doc.to_json().map(|j| j.len()).unwrap_or(0);
            let without = {
                let mut d = doc.clone();
                d.symbols = None;
                d.to_json().map(|j| j.len()).unwrap_or(0)
            };
            // How much of the index is the repeated path string, and how
            // much of the whole document is.
            let ix_bytes = serde_json::to_string(ix).map(|j| j.len()).unwrap_or(0);
            eprintln!("fixture {i}: index {ix_bytes} bytes");
            eprintln!(
                "fixture {i}: symbol index — {} definitions, {} uses; document \
                 {whole} bytes against {without} without it (+{:.0}%)",
                ix.definitions.len(),
                ix.uses.len(),
                (whole as f64 / without.max(1) as f64 - 1.0) * 100.0
            );
        }
        let edges = graph.edge_count();
        let knots: Vec<usize> = tarjan_scc(&graph)
            .into_iter()
            .map(|component| component.len())
            .filter(|&n| n > 1)
            .collect();
        let in_a_cycle: usize = knots.iter().sum();
        let biggest = knots.iter().copied().max().unwrap_or(0);

        eprintln!(
            "fixture {i}: {} classes, {edges} edges, {in_a_cycle} classes in {} cycles \
             (biggest {biggest})",
            doc.classes.len(),
            knots.len()
        );
        if let Some(expected) = fx.expect.edges {
            assert_eq!(
                edges as u32, expected,
                "fixture {i}: class dependency edges — a reader changed"
            );
        }
        if let Some(expected) = fx.expect.sccs {
            assert_eq!(
                in_a_cycle as u32, expected,
                "fixture {i}: classes inside a cycle — the ordering stage cannot \
                 order these, nor anything behind them"
            );
        }
    }
}
