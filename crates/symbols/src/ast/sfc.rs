//! The reader for a single-file component.
//!
//! A `.vue` file is a `<template>`, a `<script>` and a `<style>` in one file,
//! and the part a dependency graph needs is the script: what the component
//! imports, what it calls, what it binds.
//!
//! **No Vue grammar can give us that.** Every tree-sitter Vue grammar models a
//! `<script>` body as a single `raw_text` node — editors resolve it by
//! *injection*, running a second grammar over that node's range, and the Rust
//! query engine does not do injections. A tuned query over a Vue grammar would
//! read the template and find no definitions at all, which is worse than the
//! regex floor it would replace.
//!
//! So this reader does the injection itself, in the one way that costs no
//! arithmetic: it **masks** the file. Every byte outside a `<script>` region
//! becomes a space and every newline is kept, then the TypeScript grammar and
//! the TypeScript query run over the result. Byte offsets, line numbers and
//! columns are all unchanged, so `Site` needs no fixing up and there is no
//! offset to get wrong. The alternative — parsing the substring and adding a
//! line and column offset afterwards — is the same answer with a bug in it
//! waiting for the first `<script>` that shares its line with code.
//!
//! One grammar for both dialects: TypeScript reads plain JavaScript, and a
//! `<script setup lang="ts">` and a bare `<script>` are masked in together when
//! a file has both. `.vue` already shares the `js` namespace with TypeScript
//! and JavaScript (ADR 0031), so a component and the modules that import it
//! compare names as they should.
//!
//! **A component defines no name of its own**, and that is not an oversight.
//! `<script setup>` exports nothing and a classic block is `export default
//! { … }`, so the TypeScript query's `export` gate finds nothing to take. The
//! name importers use is the file's, and nothing in the file says it — see
//! ADR 0035 for why that stays out of here.

use std::sync::LazyLock;

use differential_engine::artefact::symbols::{FileSymbols, SymbolSource};
use regex::bytes::Regex;
use tree_sitter::{Language, Query};

/// `<script …>` … `</script>`, case-insensitively, across lines, and the
/// shortest match so two blocks do not become one.
///
/// A regex rather than a parser: the crate is already here for the floor, and
/// an SFC's block boundary is a lexical fact about the file, not a grammar to
/// write (design rule 5). `regex` has no backtracking, so a stray `<script`
/// costs linear time and not a hang.
///
/// The one known edge: `[^>]*` stops at the first `>`, so an attribute value
/// holding a literal `>` would end the tag early and mask out the script it
/// opened. The file would then read as template-only and fall to the floor —
/// the same answer it got before this reader existed, which is why the tag is
/// not worth parsing properly.
static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<script\b[^>]*>(.*?)</script\s*>").expect("a fixed pattern compiles")
});

pub struct SfcSymbols {
    language: Language,
    /// `None` if the query would not compile. A test asserts it is `Some`: the
    /// query and the grammar are both ours and both pinned, exactly as they are
    /// for the tuned reader.
    query: Option<Query>,
}

impl Default for SfcSymbols {
    fn default() -> Self {
        Self::new()
    }
}

impl SfcSymbols {
    /// Bump when the mask or the query behind it changes. It reaches the
    /// grouping cache key, so a stale grouping would otherwise be served for a
    /// graph that moved.
    const VERSION: &'static str = "sfc-v1";

    pub fn new() -> Self {
        let language: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(&language, include_str!("queries/typescript.scm")).ok();
        SfcSymbols { language, query }
    }

    /// Whether the query compiled against its pinned grammar.
    pub fn ready(&self) -> bool {
        self.query.is_some()
    }
}

/// Everything outside a `<script>` body replaced by a space, newlines kept.
///
/// `None` when the file holds no script block at all — a template-only
/// component. That is the port's "claimed it and could not read it" answer, and
/// the floor takes the file, exactly as it does for a failed parse.
fn script_only(content: &[u8]) -> Option<Vec<u8>> {
    let mut masked = vec![b' '; content.len()];
    for (i, &b) in content.iter().enumerate() {
        if b == b'\n' || b == b'\r' {
            masked[i] = b;
        }
    }
    let mut found = false;
    for m in SCRIPT.captures_iter(content) {
        let Some(body) = m.get(1) else { continue };
        found = true;
        masked[body.range()].copy_from_slice(&content[body.range()]);
    }
    found.then_some(masked)
}

impl SymbolSource for SfcSymbols {
    /// The top rung. A query knows this language's shape, and the mask is what
    /// gets the query to the part of the file the language is in.
    fn priority(&self, path: &[u8]) -> Option<u8> {
        (self.query.is_some() && path.ends_with(b".vue")).then_some(9)
    }

    fn file_symbols(&self, path: &[u8], content: &[u8]) -> Option<FileSymbols> {
        let query = self.query.as_ref()?;
        let masked = script_only(content)?;
        super::run_query(&self.language, query, &masked, crate::namespace::of(path))
    }

    /// The reader's version, then the query's — the same shape the tuned
    /// reader's fingerprint has, and for the same reason: a change to this
    /// file's Rust must cold the cache even when no `.scm` moved.
    fn fingerprint(&self) -> String {
        format!("{}[typescript-v4]", Self::VERSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(src: &str) -> String {
        String::from_utf8(script_only(src.as_bytes()).expect("a script block")).unwrap()
    }

    #[test]
    fn the_mask_keeps_every_byte_position_and_every_line() {
        let src = "<template>\n  <Child />\n</template>\n<script>\nconst a = 1;\n</script>\n";
        let out = mask(src);
        assert_eq!(out.len(), src.len(), "a byte offset must not move");
        assert_eq!(
            out.lines().count(),
            src.lines().count(),
            "a line number must not move"
        );
        assert!(out.contains("const a = 1;"), "the script survives: {out:?}");
        assert!(!out.contains("Child"), "the template does not: {out:?}");
    }

    #[test]
    fn both_blocks_of_a_setup_component_are_kept() {
        let src = "<script lang=\"ts\">\nexport default {};\n</script>\n\
                   <script setup lang=\"ts\">\nconst b = 2;\n</script>\n";
        let out = mask(src);
        assert!(out.contains("export default {};"), "{out:?}");
        assert!(out.contains("const b = 2;"), "{out:?}");
    }

    #[test]
    fn a_template_only_component_is_declined_rather_than_read_as_empty() {
        // Declining hands the file to the floor. Answering with an empty
        // `FileSymbols` would claim the file and say it has no symbols, which
        // is a different and wrong statement.
        assert!(script_only(b"<template>\n  <Child />\n</template>\n").is_none());
    }
}
