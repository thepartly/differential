# differential-symbols

The symbol readers for [`differential`](https://crates.io/crates/differential). They answer
one question about a file — **what does each line define, and what does it reference?** —
and the answer becomes the dependency graph that orders a review foundation-first.

Four readers ship. Each ranks itself for a given path, and the best claimant answers. The
regex floor stands under every extension the table below names, so a file an AST reader
claims and fails to parse still gets crude symbols rather than none. A file no reader
claims contributes no symbols at all.

Project home: <https://github.com/thepartly/differential>

## Language coverage

| language | extensions | reader |
|---|---|---|
| Rust | `.rs` | tuned query |
| Python | `.py` `.pyi` | tuned query |
| Go | `.go` | tuned query |
| TypeScript | `.ts` `.mts` `.cts` | tuned query |
| TSX | `.tsx` | tuned query |
| Kotlin | `.kt` `.kts` | tuned query |
| Java | `.java` | tuned query |
| C# | `.cs` | tuned query |
| Swift | `.swift` | tuned query |
| PHP | `.php` | tuned query |
| Zig | `.zig` | tuned query |
| Vue | `.vue` | tuned query, over the `<script>` blocks |
| JavaScript | `.js` `.jsx` `.mjs` `.cjs` | field rules |
| C | `.c` `.h` | field rules |
| C++ | `.cc` `.cpp` `.cxx` `.hpp` `.hh` | field rules |
| Ruby, Scala, shell, Perl, Lua, Elixir, Erlang, Haskell, OCaml, Dart, Svelte, SQL, Protobuf | `.rb` `.scala` `.sh` `.bash` `.zsh` `.pl` `.pm` `.lua` `.ex` `.exs` `.erl` `.hs` `.ml` `.mli` `.dart` `.svelte` `.sql` `.proto` | regex floor (only reader) |
| everything else — manifests, lockfiles, prose, data | — | none |

## What each rung costs you

| reader | definitions | references | comments and strings |
|---|---|---|---|
| tuned query | from the tree, per language | calls, types, JSX names and paths, per language | dropped |
| field rules | from the tree | calls and types, by field name | dropped |
| regex floor | declaration keywords | every identifier of four characters or more | **counted** |
| none | — | — | — |

Both AST readers answer with a **scope** as well as a name (ADR 0030). A global name is one
other files can use, and its edges may cross a file; a file-local name — a binding inside a
function, a parameter, an import, a method — is keyed by `(file, name)` and draws edges only
inside the file it was read from. The tuned readers take every identifier as a possible
file-local reference, so a value declared in one hunk and read in the next is visible; the
regex floor has no scope to offer and its answers stay global.

Four consequences worth stating outright, because each one surprises.

**A file no reader claims still exists.** Its hunks are counted, its classes are formed, it
is read like anything else. It simply draws no dependency edges. Withholding symbols is
classification, never enumeration — nothing here can add or remove a hunk (ADR 0005, 0012).

**Falling a rung costs precision, not coverage.** A missing edge misorders a reading plan.
It can never hide a change.

**What a tuned query buys is knowing what a definition is not.** `mod template;` names a
module, and `fn from` inside `impl From<X> for Y` is reached through the trait — neither
introduces a name other files can use. The regex floor cannot tell either from a real
definition, and on one measured range six such words produced 64% of every dependency edge.
A query can also tell that apart from `impl Service { fn load_batch }`, which is a
definition: one type owns it, and callers elsewhere name it exactly (ADR 0030).

**A name is consumed by being named, not only by being called.**
`route(api::widgets::handler)` hands a function to a router. Capturing only the callee
position meant every registration table — routers, dispatch maps — drew no edge at all.

**In a module language, `export` is the whole predicate** — for a type as much as a value.
`export const Panel = …` and `export interface PanelProps` define; a bare
`const send = vi.fn()` or `type FormData = …` in a test file does not, and counting those
linked production files to the test. Unexported, they are still read as file-local names, so
they keep every edge they can honestly draw.

**A global name is scoped to its language.** `FormData` read from a Rust file and
`FormData` read from a TypeScript one are two symbols: nothing here parses a monorepo's
build graph, so a shared word is never evidence that two packages are connected (ADR 0031).
`namespace::of` is the table, and all three readers share it — the crude reader is the AST
readers' fallback, so a namespace of its own would split a language in two the first time a
parse failed. The floor's claim (`namespace::is_code`) reads the same table, so it cannot
know fewer extensions than the readers it stands under. This *raises* the edge count on a
mixed repository: a name declared once per language used to have two definers and be
dropped for both.

Comments and strings are dropped by both AST readers without any query, because every
grammar names those nodes with those words. A token that reaches a string through an
interpolation is still code, so `"${resolve(id)}"` keeps its call.

## How a language moves up

**To the field rules:** one entry in the table in `src/ast/generic.rs`, plus the grammar
crate. The reader probes each grammar when it is built and declines any whose shape its
rules cannot read, so a grammar that does not fit falls to the floor rather than answering
empty.

**To a tuned query:** the same, plus a `.scm` in `src/ast/queries` capturing `@def`,
`@call` and `@type`. Roughly ten patterns. Queries compile when the reader is built, so a
wrong node name fails immediately and names the node.

**A single-file format is its own reader.** A `.vue` file's `<script>` body is one `raw_text`
node in every Vue grammar — editors resolve it by an injection the Rust query engine does not
do — so `src/ast/sfc.rs` masks every byte outside a script block to a space and runs the
TypeScript query over the result. Line numbers and columns stay the file's, which is the
point of masking rather than parsing a substring (ADR 0035).

Kotlin is the worked example of why the top rung exists. Its `call_expression` carries no
`function:` field and its `navigation_expression` names none of its children, so the field
rules found no calls there at all. Nine of the other ten grammars passed those rules on a
probe before any of this was written.

Each query carries a version in the reader's fingerprint, which is part of the grouping
cache key. Editing a query therefore colds the cache by itself, and
`every_query_version_pins_its_patterns` pins each query's PATTERNS — its text with comment
and blank lines removed — against its version, so the bump cannot be forgotten and a
reworded comment does not cold every cached grouping for nothing. Change a pattern, bump
the `-vN`, paste the hash the test prints.

A query is not the only thing that can change an answer, so
`every_reader_fingerprint_pins_its_answers` hashes what each reader actually extracts from a
fixed set of samples and pins that beside its version too. It covers a change to the
readers' Rust — the gap that let the crude reader gain a namespace and keep `naive-v1` — and
it fails on a tree-sitter grammar upgrade, which it should: a new grammar can change
extraction, and nothing else in the tree would notice.

## What is proven against what

| reader | evidence |
|---|---|
| tuned query | Rust, Python and TypeScript run against a real multi-language corpus; TypeScript and TSX additionally against a React corpus, and Rust against a service-backend one. Go, Kotlin, Java, C#, Swift, PHP, Zig and Vue are covered by per-language tests only, and their method and path rules are unmeasured — see ADR 0030. The corpus ranges the parity test pins hold no Java, C#, Swift, PHP, Zig or Vue change, so neither range's edge count moved when those six were promoted. |
| field rules | C, C++ and JavaScript, by per-language tests only. |
| regex floor | runs against the corpus wherever no grammar claims a file. |

On that corpus the readers took two ranges from 288 and 131 dependency edges down to 63 and
26, and the second range's twelve-class cycle — which left every class behind it impossible
to order — disappeared entirely.

## Known edges

- Rust's `(source_file (const_item …)) @def` does not check `pub`, so a private constant is
  global where an unexported TypeScript one is not. Predates the scope split, and moving it
  belongs to its own measurement (ADR 0030).
- `.jsx` goes to the field rules, which have no JSX rule — a rendered component draws no
  edge there. `.tsx` does, by query.
- Go's `@ref`, Python's, Java's, C#'s, Swift's, PHP's and Zig's take struct-field and
  attribute reads too: those languages spell a qualified name and a member read the same
  way. Rust's `scoped_identifier` does not have this problem.
- C# and Zig have no `type_identifier` node — a type is a plain identifier in a `type:`
  field — so their queries reach types through a wildcard on that field rather than through a
  node kind. A type position the grammar spells with some other field name is missed.
- Zig gates on `pub`, exactly as TypeScript gates on `export`. The shape of a Zig
  declaration says nothing about who can reach it — a type is a `const` bound to a container
  body and a value is a `const` bound to anything else — so a private
  `const Helper = @import("x.zig")` stays file-local while `pub const MAX = 100` and
  `pub const ReExport = @import(…)` are both global. Everything private in a Zig file is
  invisible across files, which is what the graph now says.
- **Zig is the only one of these where a visibility gate is expressible.** `pub` is required
  to export, so its absence means private and a positive match settles it. Swift's default is
  `internal` and PHP's is `public`, so there the gate would have to match an ABSENT modifier,
  which a tree-sitter query cannot do — a positive match on `public` would drop every method
  that leaves the modifier off. So a `private func` in Swift and a `private function` in PHP
  are still read as definitions, and the single-definer rule absorbs the ones that collide.
- PHP's `__construct` is a method like any other here, so a class that declares one defines
  that name. Every class does, so the single-definer rule drops it — which is the mechanism
  for exactly this, not an accident.
- A Vue file's named exports are taken like any module's — a classic `<script>` block's
  `export const`, `export function` and `export interface` all pass the `export` gate. The
  COMPONENT's own name is the one that is not: `<script setup>` has the compiler write the
  default export and a classic block writes `export default { … }`, so there is no identifier
  either way. Deriving one from the file stem was considered and dropped — the only use of a
  component is in a `<template>`, which is masked, so it would have no global consumer
  (ADR 0035).
- A Vue `<template>` is masked out, so `<ChildWidget />` draws no edge. That is `.jsx`'s
  known edge in another language, and reading the template needs a Vue grammar.

## Using it

```rust
// The application wires the readers. Registration order does not matter —
// each reader ranks itself for a given path.
let symbols = differential_symbols::readers();

let out = run_pipeline(&repo, &src, &config,
                       &LanguageRegistry::builtin(), &symbols)?;
```

The engine owns the `SymbolSource` port and the rule for choosing between readers
(`engine::artefact::symbols`). This crate owns the readers and depends on the engine, never
the reverse. See
[`adr/0023-symbol-extraction-is-a-domain-port.md`](../../adr/0023-symbol-extraction-is-a-domain-port.md).

## Licence

MIT or Apache-2.0, at your option.
