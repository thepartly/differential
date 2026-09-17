//! Which names are allowed to match which.
//!
//! A global symbol used to be a bare name, so `FormData` read from a Rust file
//! and `FormData` read from a TypeScript one were one symbol. In a monorepo
//! that is a coincidence far more often than a fact: nothing here parses the
//! build graph, so the tool cannot know whether a Rust crate and a TypeScript
//! package are connected at all, and a shared word is only ever a guess that
//! they are (ADR 0031).
//!
//! So a reader hands the graph a namespace along with the names, and two global
//! symbols match only when their namespaces do. The graph treats the token as
//! opaque — it still cannot tell which reader answered, only whether two
//! answers are comparable.
//!
//! **All three readers use this one table**, and they must: the crude reader is
//! the fallback for a file the tuned reader claimed and failed to parse, so a
//! per-reader answer would split one language across two namespaces the first
//! time a parse failed. The crude reader's CLAIM reads the same table
//! ([`is_code`]), for the same reason: a fallback that knew fewer extensions
//! than the readers it stands under is not a fallback for those files.
//!
//! Languages share a namespace where they genuinely share names. TypeScript,
//! TSX, JavaScript and the single-file component formats are one `js`; Java,
//! Kotlin and Scala are one `jvm`; C and C++ are one `c`.

/// Every extension a reader knows, and the namespace it belongs to.
///
/// A whitelist, never a blacklist: a file type nobody thought about gets
/// silence, which is the safe answer. On the validation corpus, 32% of every
/// dependency edge came from classes made entirely of manifests, lockfiles and
/// prose, and every one of those edges was false.
const TABLE: &[(&str, &[&str])] = &[
    ("rust", &[".rs"]),
    ("python", &[".py", ".pyi"]),
    ("go", &[".go"]),
    (
        "js",
        &[
            ".ts", ".mts", ".cts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".vue", ".svelte",
        ],
    ),
    ("jvm", &[".kt", ".kts", ".java", ".scala"]),
    ("c", &[".c", ".h", ".cc", ".cpp", ".cxx", ".hpp", ".hh"]),
    ("csharp", &[".cs"]),
    ("ruby", &[".rb"]),
    ("php", &[".php"]),
    ("swift", &[".swift"]),
    ("shell", &[".sh", ".bash", ".zsh"]),
    ("perl", &[".pl", ".pm"]),
    ("lua", &[".lua"]),
    ("elixir", &[".ex", ".exs"]),
    ("erlang", &[".erl"]),
    ("haskell", &[".hs"]),
    ("ocaml", &[".ml", ".mli"]),
    ("dart", &[".dart"]),
    ("sql", &[".sql"]),
    ("proto", &[".proto"]),
    ("zig", &[".zig"]),
];

/// The table's answer for `path`, when it has one.
fn lookup(path: &[u8]) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|(_, extensions)| extensions.iter().any(|e| path.ends_with(e.as_bytes())))
        .map(|(namespace, _)| *namespace)
}

/// Whether `path` is a file the table names at all.
///
/// This is the crude reader's claim. It used to keep a list of its own that
/// had drifted three extensions behind the tuned reader's, so a `.pyi` whose
/// parse failed fell past the floor to nothing. One table, one answer.
pub fn is_code(path: &[u8]) -> bool {
    lookup(path).is_some()
}

/// The namespace for `path`, by extension.
///
/// Falls back to the extension itself, so an unclaimed file cannot land in a
/// shared bucket with an unrelated one. No reader claims such a file today, so
/// the fallback is a guard rather than a path anyone takes.
pub fn of(path: &[u8]) -> Vec<u8> {
    if let Some(namespace) = lookup(path) {
        return namespace.as_bytes().to_vec();
    }
    match path.iter().rposition(|&b| b == b'.') {
        Some(dot) => path[dot..].to_vec(),
        None => path.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(path: &str) -> String {
        String::from_utf8(of(path.as_bytes())).unwrap()
    }

    #[test]
    fn one_language_is_one_namespace_however_it_is_spelled() {
        // The case that motivated this: two `FormData`s, one per language.
        assert_ne!(ns("src/model.rs"), ns("tests/api.test.ts"));
        // And the case it must not break: a language reached by more than one
        // extension, or by more than one reader, stays one namespace. The
        // crude reader is the tuned reader's fallback, so a split here would
        // appear the first time a parse failed.
        assert_eq!(ns("a.ts"), ns("b.tsx"));
        assert_eq!(ns("a.ts"), ns("c.js"));
        assert_eq!(ns("A.kt"), ns("B.java"));
        assert_eq!(ns("a.c"), ns("b.hpp"));
        assert_eq!(ns("a.rs"), "rust");
    }

    #[test]
    fn the_claim_and_the_namespace_read_one_table() {
        // The three the crude reader's own list had missed, and two it had.
        for path in ["a.pyi", "a.mts", "a.cts", "a.rs", "q.sql"] {
            assert!(is_code(path.as_bytes()), "{path} is in the table");
        }
        for path in ["Cargo.toml", "README.md", "CODEOWNERS"] {
            assert!(!is_code(path.as_bytes()), "{path} is not");
        }
    }

    #[test]
    fn an_unclaimed_file_gets_its_own_extension_not_a_shared_bucket() {
        assert_eq!(ns("Cargo.lock"), ".lock");
        assert_eq!(ns("notes.md"), ".md");
        assert_ne!(ns("notes.md"), ns("Cargo.lock"));
        assert_eq!(ns("CODEOWNERS"), "CODEOWNERS");
    }
}
