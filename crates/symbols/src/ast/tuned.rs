//! The reader for languages with a hand-written query.
//!
//! One `.scm` per language, capturing three things and nothing else:
//!
//! | capture | means |
//! |---|---|
//! | `@def` | a name this file introduces that others can use |
//! | `@call` | a function being called |
//! | `@type` | a type being used |
//! | `@ref` | a name reached by path, without being called |
//! | `@local_def` | a name that reaches only this file |
//! | `@local_ref` | an identifier that might be reading one |
//!
//! The last two are why a `const` inside a function is not thrown away
//! (ADR 0030). They are compared only against the same file's answers, so a
//! common word cannot become a symbol the whole change links to — which is the
//! failure the file-scope rule exists to prevent.
//!
//! Written here rather than vendored from nvim-treesitter: those files use
//! predicates the Rust query engine does not support (`#lua-match?`,
//! `#has-ancestor?`) and are pinned to grammar versions we do not control.
//!
//! **A wrong node name fails when the query compiles**, and the error names the
//! node. That loud failure is the reason for a query file over a hand-written
//! tree walk, which would return zero and say nothing.

use differential_engine::artefact::symbols::{FileSymbols, SymbolSource};
use tree_sitter::{Language, Query};

struct Tuned {
    /// Bump the `-vN` when the query changes. It reaches the grouping cache key,
    /// so a stale grouping would otherwise be served for a graph that moved.
    version: &'static str,
    extensions: &'static [&'static [u8]],
    language: fn() -> Language,
    /// Query text, joined in order. TSX is TypeScript's query plus the JSX
    /// patterns, and the plain TypeScript grammar has no nodes for those — a
    /// single shared file would fail to compile against one of the two.
    sources: &'static [&'static str],
}

impl Tuned {
    fn query_text(&self) -> String {
        self.sources.join("\n")
    }
}

static TUNED: &[Tuned] = &[
    Tuned {
        version: "rust-v4",
        extensions: &[b".rs"],
        language: rust,
        sources: &[include_str!("queries/rust.scm")],
    },
    Tuned {
        version: "python-v4",
        extensions: &[b".py", b".pyi"],
        language: python,
        sources: &[include_str!("queries/python.scm")],
    },
    Tuned {
        version: "go-v4",
        extensions: &[b".go"],
        language: go,
        sources: &[include_str!("queries/go.scm")],
    },
    Tuned {
        version: "typescript-v4",
        extensions: &[b".ts", b".mts", b".cts"],
        language: typescript,
        sources: &[include_str!("queries/typescript.scm")],
    },
    Tuned {
        version: "tsx-v4",
        extensions: &[b".tsx"],
        language: tsx,
        sources: &[
            include_str!("queries/typescript.scm"),
            include_str!("queries/tsx.scm"),
        ],
    },
    Tuned {
        version: "kotlin-v4",
        extensions: &[b".kt", b".kts"],
        language: kotlin,
        sources: &[include_str!("queries/kotlin.scm")],
    },
    Tuned {
        version: "java-v1",
        extensions: &[b".java"],
        language: java,
        sources: &[include_str!("queries/java.scm")],
    },
    Tuned {
        version: "csharp-v1",
        extensions: &[b".cs"],
        language: csharp,
        sources: &[include_str!("queries/csharp.scm")],
    },
    Tuned {
        version: "swift-v1",
        extensions: &[b".swift"],
        language: swift,
        sources: &[include_str!("queries/swift.scm")],
    },
    Tuned {
        version: "php-v1",
        extensions: &[b".php"],
        language: php,
        sources: &[include_str!("queries/php.scm")],
    },
    Tuned {
        version: "zig-v1",
        extensions: &[b".zig"],
        language: zig,
        sources: &[include_str!("queries/zig.scm")],
    },
];

fn rust() -> Language {
    tree_sitter_rust::LANGUAGE.into()
}
fn python() -> Language {
    tree_sitter_python::LANGUAGE.into()
}
fn go() -> Language {
    tree_sitter_go::LANGUAGE.into()
}
fn typescript() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
fn tsx() -> Language {
    tree_sitter_typescript::LANGUAGE_TSX.into()
}
fn kotlin() -> Language {
    tree_sitter_kotlin_ng::LANGUAGE.into()
}
fn java() -> Language {
    tree_sitter_java::LANGUAGE.into()
}
fn csharp() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}
fn swift() -> Language {
    tree_sitter_swift::LANGUAGE.into()
}
/// `LANGUAGE_PHP`, not `LANGUAGE_PHP_ONLY`: a `.php` file may open in HTML and
/// reach its first `<?php` some lines down, and the PHP-only grammar cannot
/// parse that at all.
fn php() -> Language {
    tree_sitter_php::LANGUAGE_PHP.into()
}
fn zig() -> Language {
    tree_sitter_zig::LANGUAGE.into()
}

pub struct AstSymbols {
    ready: Vec<(&'static Tuned, Language, Query)>,
    /// Languages whose query would not compile, and why.
    ///
    /// **A test asserts this is empty.** A query and its grammar are both ours
    /// and both pinned, so a mismatch is a bug to fix, never a state to ship.
    failures: Vec<(&'static str, String)>,
}

impl Default for AstSymbols {
    fn default() -> Self {
        Self::new()
    }
}

impl AstSymbols {
    pub fn new() -> Self {
        let mut ready = Vec::new();
        let mut failures = Vec::new();
        for tuned in TUNED {
            let language = (tuned.language)();
            match Query::new(&language, &tuned.query_text()) {
                Ok(query) => ready.push((tuned, language, query)),
                Err(e) => failures.push((tuned.version, e.to_string())),
            }
        }
        AstSymbols { ready, failures }
    }

    /// Every query that would not compile against its pinned grammar.
    pub fn failures(&self) -> &[(&'static str, String)] {
        &self.failures
    }

    /// Every query, as `(version, text)`.
    ///
    /// Exists for the pin test: the version reaches the grouping cache key, so
    /// editing a query without bumping it serves a stale grouping for a graph
    /// that moved. The test hashes the patterns against the version, which is
    /// the only way the bump cannot be forgotten.
    pub fn queries() -> Vec<(&'static str, String)> {
        TUNED.iter().map(|t| (t.version, t.query_text())).collect()
    }

    fn entry(&self, path: &[u8]) -> Option<&(&'static Tuned, Language, Query)> {
        self.ready
            .iter()
            .find(|(t, _, _)| t.extensions.iter().any(|e| path.ends_with(e)))
    }
}

impl SymbolSource for AstSymbols {
    /// The top rung: a query knows this language's shape, so nothing outranks it.
    fn priority(&self, path: &[u8]) -> Option<u8> {
        self.entry(path).map(|_| 9)
    }

    fn file_symbols(&self, path: &[u8], content: &[u8]) -> Option<FileSymbols> {
        let (_, language, query) = self.entry(path)?;
        super::run_query(language, query, content, crate::namespace::of(path))
    }

    /// `ast-tuned-vN[<query versions>]` — the reader's OWN version, then the
    /// queries'.
    ///
    /// The reader's version has to be here, and it was not. Every earlier
    /// change to this reader happened to edit a `.scm` as well, so the query
    /// versions carried the bump and nothing noticed that a Rust-only change
    /// could not. [ADR 0031](../../../../adr/0031-a-global-name-is-scoped-to-its-language.md)
    /// named exactly this hole for the crude reader and closed it there; this
    /// is the same hole one rung up, and the change that added `Site` is the
    /// first Rust-only change to walk into it.
    fn fingerprint(&self) -> String {
        let mut parts: Vec<&str> = self.ready.iter().map(|(t, _, _)| t.version).collect();
        parts.sort_unstable();
        format!("ast-tuned-v2[{}]", parts.join(","))
    }
}
