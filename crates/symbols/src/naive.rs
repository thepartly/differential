//! The crude reader: one regex pass per line, no parser.
//!
//! Moved verbatim from `engine::lang::generic`, where it was the `Language`
//! trait's default answer. A port must not ship an answer, so it is a named
//! reader now, ranked below anything that actually parses.
//!
//! It reads only files whose extension the namespace table names
//! (`namespace::is_code`) — the same table that gives every reader its
//! namespace, so it stands under every file an AST reader claims. A
//! `Cargo.toml`, a `README.md` or a lockfile is claimed by nobody and
//! contributes nothing: on the validation corpus, 32% of every dependency edge
//! came from classes made entirely of such files, and every one of those edges
//! was false.

use std::sync::LazyLock;

use differential_engine::artefact::symbols::{FileSymbols, Symbol, SymbolSource};
use regex::bytes::Regex;

// (?-u): byte-level ASCII classes, matching the validated prototype.
static DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?-u)\b(?:fn|struct|enum|trait|class|interface|type|def|func|impl|const|static|mod|module|package|protocol)\s+([A-Za-z_][A-Za-z0-9_]{2,})",
    )
    .unwrap()
});
static REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u)[A-Za-z_][A-Za-z0-9_]{3,}").unwrap());

pub struct NaiveSymbols;

impl SymbolSource for NaiveSymbols {
    /// The floor. Anything that parses outranks it.
    ///
    /// The claim is the namespace table's, not a list of this reader's own.
    /// It had one, and the list lagged the tuned reader by `.pyi`, `.mts` and
    /// `.cts`: a file of those kinds whose parse failed fell past the floor to
    /// no symbols at all, against what the fallback promises.
    fn priority(&self, path: &[u8]) -> Option<u8> {
        crate::namespace::is_code(path).then_some(1)
    }

    /// Split on `\n` only. A `\r` survives into the line, where the identifier
    /// patterns cannot match it — exactly as it could not in a diff line.
    ///
    /// Never fails: a regex has nothing to choke on.
    fn file_symbols(&self, path: &[u8], content: &[u8]) -> Option<FileSymbols> {
        let lines: Vec<&[u8]> = content.split(|&b| b == b'\n').collect();
        Some(FileSymbols {
            // The SAME table the AST readers use. This reader is their
            // fallback when a parse fails, so a namespace of its own would
            // split one language in two the first time that happened.
            namespace: crate::namespace::of(path),
            defines: lines.iter().map(|l| global(definitions(l))).collect(),
            references: lines.iter().map(|l| global(references(l))).collect(),
        })
    }

    /// `-v4`: the claim moved onto the namespace table, so this reader now
    /// stands under `.pyi`, `.mts` and `.cts` too. No reachable input answers
    /// differently — the tuned reader outranks it on all three and a parse
    /// there does not fail today — but the contract below says bump anyway.
    ///
    /// `-v3` was the columns on every name; `-v2` the namespace (ADR 0031).
    /// The reason to bump is the same every time and does not depend on the
    /// change mattering to the graph: the port's contract is that a reader
    /// which ANSWERS differently colds the cache. This reader is the only one
    /// for Ruby, PHP, Swift, Elixir, shell and the rest, and the AST readers'
    /// fallback when a parse fails, so a forgotten bump here is a stale
    /// grouping nothing would catch.
    fn fingerprint(&self) -> String {
        "naive-v4".to_string()
    }
}

/// Symbol names introduced by common declaration keywords. Deliberately crude:
/// ordering tolerates low precision — a wrong edge misorders, it can never hide
/// content (ADR 0007).
///
/// **Global, and deliberately so.** A regex cannot tell a file-scope
/// declaration from one inside a function, so scoping these to their file
/// would silently delete every cross-file edge for the languages that reach
/// this reader — Ruby, PHP, Swift, Elixir and the rest have no other. That is a
/// precision question of its own, with its own corpus measurement; it is not
/// this one.
fn definitions(line: &[u8]) -> Vec<Found> {
    DEF_RE
        .captures_iter(line)
        .filter_map(|c| c.get(1))
        .map(|m| (m.as_bytes().to_vec(), m.start() as u32, m.end() as u32))
        .collect()
}

/// Identifiers used in the line. A superset of definitions; the graph
/// intersects against what other classes define, so most noise cancels out.
fn references(line: &[u8]) -> Vec<Found> {
    REF_RE
        .find_iter(line)
        .map(|m| (m.as_bytes().to_vec(), m.start() as u32, m.end() as u32))
        .collect()
}

/// A name and its byte range within the line the regex was run over.
///
/// The regex matches a LINE, so its offsets are already the raw-line columns
/// `Site` wants — there is no file-wide range to subtract.
type Found = (Vec<u8>, u32, u32);

/// Every name this reader finds reaches beyond its file, as far as it can
/// tell. See [`definitions`].
///
/// No `through`: a regex has no tree to ask how far a declaration runs, and
/// zero says that honestly rather than guessing at the next blank line.
fn global(names: Vec<Found>) -> Vec<Symbol> {
    names
        .into_iter()
        .map(|(name, start, end)| Symbol::global(name).at(start, end))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Vec<Symbol>], line: usize) -> Vec<String> {
        rows[line - 1]
            .iter()
            .map(|s| String::from_utf8_lossy(&s.name).into_owned())
            .collect()
    }

    #[test]
    fn it_reads_source_and_declines_data_and_prose() {
        assert_eq!(NaiveSymbols.priority(b"src/lib.rs"), Some(1));
        assert_eq!(NaiveSymbols.priority(b"queries/get.sql"), Some(1));
        // Claimed by the tuned reader, and missing from the list this reader
        // used to keep of its own.
        assert_eq!(NaiveSymbols.priority(b"typings/api.pyi"), Some(1));
        // The four the corpus indicted, plus a file with no extension at all.
        assert_eq!(NaiveSymbols.priority(b"Cargo.toml"), None);
        assert_eq!(NaiveSymbols.priority(b"README.md"), None);
        assert_eq!(NaiveSymbols.priority(b"pnpm-lock.yaml"), None);
        assert_eq!(NaiveSymbols.priority(b"package.json"), None);
        assert_eq!(NaiveSymbols.priority(b"CODEOWNERS"), None);
    }

    #[test]
    fn symbols_land_on_the_line_that_carries_them() {
        let s = NaiveSymbols
            .file_symbols(
                b"a.rs",
                b"// nothing\nstruct WidgetCore {}\nlet c = WidgetCore;\n",
            )
            .unwrap();
        assert_eq!(text(&s.defines, 2), ["WidgetCore"]);
        assert!(s.defines[2].is_empty(), "line 3 defines nothing");
        assert_eq!(text(&s.references, 3), ["WidgetCore"]);
    }
}
