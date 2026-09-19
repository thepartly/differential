# 0035 — A single-file component is masked, not injected

Status: accepted

Extends [ADR 0023](0023-symbol-extraction-is-a-domain-port.md), which set the three reader
rungs and the rule that the best claimant answers. Bounded by
[ADR 0030](0030-what-counts-as-a-definition-and-how-far-it-reaches.md), whose definition of a
definition is what decides the open question at the end, and
[ADR 0031](0031-a-global-name-is-scoped-to-its-language.md), which already put `.vue` in the
`js` namespace.

## Context

A `.vue` file had only the regex floor: declaration keywords for definitions, every
identifier of four characters or more for references, and comments and strings counted along
with the code. Promoting it looked like the other six promotions — add a grammar, write a
`.scm`, done.

It is not, and the reason is worth writing down because it applies to every single-file
format after it.

**No tree-sitter Vue grammar parses a `<script>` body.** It is one `raw_text` node. Editors
resolve it by *injection*: an `injections.scm` names that node's range and a second grammar
runs over it. The Rust query engine does not do injections, and nothing in this crate could
make it. A tuned query over a Vue grammar would therefore read the template, find no
definitions at all, and answer worse than the floor it replaced — while outranking it, so
the floor would never get the file.

That was checked against both the original `ikatyang/tree-sitter-vue` and the maintained
fork. It is a property of how the format is modelled, not of one grammar's quality.

## Decision

**Mask the file down to its script blocks, and run the TypeScript grammar and query over the
result.** Every byte outside a `<script …>` … `</script>` body becomes a space; every newline
is kept.

Four things follow, and the first is the whole reason for this shape rather than the obvious
one:

1. **Byte offsets, line numbers and columns are the FILE's, untouched.** `Site` needs no
   fixing up and there is no offset arithmetic to get wrong. The obvious alternative — parse
   the substring, add a line offset and a column offset afterwards — is the same answer with
   a bug in it, waiting for the first `<script>` that shares a line with code.
2. **One grammar for both dialects.** TypeScript reads plain JavaScript, so `lang="ts"` needs
   no sniffing, and a file with both a `<script>` and a `<script setup>` is masked in
   together.
3. **A file with no script block is DECLINED, not answered empty.** Declining is the port's
   "claimed it and could not read it", and the floor takes the file — exactly what a failed
   parse already does. Answering with an empty `FileSymbols` would claim the file and state
   that it has no symbols, which is a different thing and a false one.
4. **The query is shared, not copied.** `ast::run_query` was lifted out of the tuned reader
   for this, its second consumer. The TypeScript query has one text and one version, so the
   two readers cannot drift.

The reader is its own `SymbolSource` rather than a row in the tuned table, because it needs a
step the table cannot express — the mask — and because the next single-file format lands
beside it rather than inside a growing struct.

## A component defines no name of its own

`<script setup>` exports nothing, and a classic block is `export default { … }`. So the
TypeScript query's `export` gate — the predicate ADR 0030 settled on for a module language —
finds nothing to take, and a `.vue` file draws **no incoming edge at all**.

Deriving the name from the path was the obvious answer, and it was wrong. `Panel.vue` is
imported as `Panel`, so the file stem is genuinely the name importers use — but **an importer
records that import as a FILE-LOCAL binding**, by ADR 0030's own rule
(`(import_clause (identifier) @local_def)`). Measured on the readers as they ship:
`import ChildWidget from './ChildWidget.vue'` puts `ChildWidget` in the importing file's
local definitions and in nobody's global references.

A global definition with no global consumer is not a neutral addition. It is the
false-definition shape ADR 0023 measured at 64% of one range's edges: a unique name that any
stray global mention anywhere then links to. So the component's name stays out, and a test
pins its absence rather than leaving the question to be re-litigated.

What would actually buy a component-to-component edge is reading the `<template>` — capturing
`<ChildWidget />` the way `tsx.scm` captures JSX. That needs a Vue grammar after all, for the
template alone, and it is a separate change with its own measurement.

## What this is not

**Not a filter.** The mask decides what a reader can read, never which files or hunks exist.
A `.vue` file is enumerated, counted and classified exactly as before
([ADR 0005](0005-no-extension-filter.md), [ADR 0012](0012-config-never-excludes.md)), and a
template-only component still reaches the reviewer in full.

**Not a new namespace.** `.vue` was already `js` (ADR 0031), which is what lets a component
and the modules it imports compare names at all.

## Alternatives rejected

**A Vue grammar for the template, plus the mask for the script.** Strictly more information,
and the right eventual answer. But it doubles the reader for a gain nothing yet measures, and
the only crates-io Vue grammar on the current ABI is a single-version personal fork. When the
template is worth reading, that is the change to make, with a corpus range to show it.

**Leaving `.vue` on the floor.** Defensible before this was written and not after: the floor
counts comments and strings, and a component's `<template>` and `<style>` are almost all of
its bytes. The floor was reading a component's CSS class names as references.

**Teaching the tuned table an optional mask.** Two extra fields on `Tuned` for one language,
and the next format would want a third. A reader is the honest unit: it has a fingerprint,
it can decline, and it ranks itself.
