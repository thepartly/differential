```
       ___ ________                     __  _       __
  ____/ (_) __/ __/__  ________  ____  / /_(_)___ _/ /
 / __  / / /_/ /_/ _ \/ ___/ _ \/ __ \/ __/ / __ `/ /
/ /_/ / / __/ __/  __/ /  /  __/ / / / /_/ / /_/ / /
\__,_/_/_/ /_/  \___/_/   \___/_/ /_/\__/_/\__,_/_/
```

# differential

**A TUI reviewer with semantic grouping.**

> A differential is a gear system in vehicles that lets driven wheels rotate at different
> speeds while still receiving power from the engine.

Agents made writing code easy. Reading it is the new bottleneck, and `differential` is an
attempt to make that part easier.

It is a terminal program. It runs where the agent worked — your machine or a remote one —
beside the worktree, and it needs no forge and no remote.

Reading a large diff well takes four things:

- separate the important changes from the unimportant ones;
- group the changes by semantic similarity;
- read the group the others depend on first;
- show the context a hunk needs to make sense.

[What it does that a diff viewer does not](#what-it-does-that-a-diff-viewer-does-not) is
how `differential` answers each one.

https://github.com/user-attachments/assets/1674e11c-cab4-42d4-a011-0ee1c4d889d0

## Requirements

- `git` on your PATH. All repository access shells out to real git.
- An agent CLI for the grouping stage. Five are supported, picked by name:
  [`claude-code`](https://claude.com/claude-code) (the default),
  [`codex`](https://developers.openai.com/codex/cli),
  [`droid`](https://docs.factory.ai/droid-exec/overview),
  [`copilot`](https://docs.github.com/en/copilot/how-tos/copilot-cli) and
  [`pi`](https://pi.dev). Each runs headless, allowed to read and never to write —
  **except `pi`, which can write**. Run `dfr agents` to see what you have installed.
  See [Agents](#agents).
- Rust stable to build from source. The version is pinned in `rust-toolchain.toml`.

## Install

```sh
cargo install differential
```

That installs two binaries, `dfr` and `differential`. They are the same program.

## Quick start

```sh
cd your-repo
dfr review                # pick a base, and include uncommitted work
dfr review main..feature  # or name the range yourself
```

With no range it opens a picker. Choose the base commit, and tick the box to include
uncommitted work. Staged changes and a dirty worktree are review sources like any other.

![The dfr review reviewer, mid-review](https://raw.githubusercontent.com/thepartly/differential/main/assets/screenshot.png)

Full detail, and every key: [`crates/tui/README.md`](https://github.com/thepartly/differential/blob/main/crates/tui/README.md).

### As a commit stack

The same plan can also be rendered as a stack of synthetic commits, which you can then
read in your IDE, in `tig`, or with plain `git log`.

```sh
dfr stack main..feature
```

```
refs/review/1a2b3c4-5d6e7f8/stack  (14 commits, 187 hunks, recount 187)
  ...
review with: git log --oneline 1a2b3c4d5e6f..refs/review/1a2b3c4-5d6e7f8/stack
```

Now read the stack like any branch:

```
$ git log --oneline 1a2b3c4d5e6f..refs/review/1a2b3c4-5d6e7f8/stack
f00dfee  [unclassified] 1 hunks carried by no group
0ddba11  [noise] Lockfiles and generated artefacts — folded, 21 hunks
cafe007  [skim 2/2] Import swaps for the renamed module — 38 further hunks, same shapes
beefed5  [skim 1/2] Import swaps for the renamed module — 28 exemplars
add1c7e  [focus] Rework retry handling in the client
decade0  [focus] Introduce the storage backend trait and its implementations
```

Read from the bottom up. Read the `[focus]` commits first. Definitions come before their
callers. Then read one exemplar per shape in `[skim 1/2]`. Then skip `[skim 2/2]` and
`[noise]` on their subject lines alone. Every hunk in them repeats a shape you already
checked.

Full detail: [`crates/stack/README.md`](https://github.com/thepartly/differential/blob/main/crates/stack/README.md).

## What it does that a diff viewer does not

### The echo filter, as the floor

Every hunk gets a **shape class**: its diff text hashed after the parts that vary are
normalised away. Strings become `"S"`, numbers become `N`, identifiers of four characters
or more become `I`, and runs of whitespace collapse to one space.

```diff
-    let timeout = Duration::from_secs(30);
+    let timeout = Duration::from_secs(config.timeout);
```

```
- let I = I::I(N);
+ let I = I::I(I.I);
```

Any other hunk that reduces to those two lines is the same shape. **One signature change
reaching two hundred call sites is one class.** Both the removed and the added side are
hashed, each side sorted, so a deletion-only hunk cannot collapse into the same class as
everything else that deletes.

This is regex, not a parser, and deliberately so. It does not pay off on every diff. But it
is cheap, it needs no model, and on the diffs that do echo it removes a great deal. That is
why it is the floor.

### It works out what depends on what

The engine builds a **symbol dependency graph** from the AST. It is a cheap way to work out
what leans on what. The graph is a fact about the diff, not about the grouping, so the model
cannot change what depends on what.

The ordering stage then contracts that graph onto groups and sorts them foundation-first,
so you meet an abstraction before its consumers.

Each group gets a role from its edges: `foundation`, `consumer`, `mechanical` or
`noise`. A cycle is reported, never hidden.

Full rules: [`spec/ordering.md`](https://github.com/thepartly/differential/blob/main/spec/ordering.md).

### LLM semantic grouping, as the ceiling

The LLM merges and labels **class ids, never hunks**. There are far fewer classes than
hunks, so the call stays small and fast.

It reads the same graph you do. The mechanical layer hands it every shape class with its
`defines:`, `uses:` and `used by:` lines. From there the model is free to open the files it
wants, or to group on the paths alone.

The answer is cached by content, so a second run on the same range is deterministic and free.

Full rules: [`spec/grouping.md`](https://github.com/thepartly/differential/blob/main/spec/grouping.md).

### A symbol says what declares it

Stand on a code row and every reference the change can resolve is underlined. Press `z` and
a **peek modal** opens showing where it is declared:

```
 shape_hash · crates/engine/src/shape.rs:61 · C12 · g2
 61 │ pub fn shape_hash(
 62 │     hunk: &Hunk,
 63 │     disposition_letter: u8,
 64 │     generated: bool,
 65 │     lang: &dyn Language,
 66 │ ) -> String {
```

The modal carries the declaration's own lines, syntax-highlighted, with their own line
numbers. Its title says which shape class wrote it and which group that class landed in, so
the next move — go and read `g2` first — is on the screen already.

### A word, found anywhere in the change

`/` opens a search over every changed file — the files as they are now, unchanged lines
included, not just the hunks. Type, and every line holding the word is listed with the
line itself previewed under it and the word marked.

```
 reading_split                                      28 found
  crates/engine/src/plan/tiers.rs:118         g2 focus C12
  crates/tui/src/rows.rs:791                  g2 focus C31
  crates/stack/src/lib.rs:44                     g5 skim
```

Each row says which group reads the line, at what tier, and which shape class the hunk
holding it belongs to. A line inside no hunk carries no class — that is how a row says it is
unchanged code.

What ranks first is where you already are, then the hits inside a hunk, then plan order. A
hit inside something the plan deferred tells you before you go. `enter` puts the cursor on
the line and opens whatever was in the way — a folded skim remainder, or a context gap the
pane was not showing.

The query is a literal. `ctrl-r` reads it as a regular expression instead.

Two things it cannot find, both for the same reason: a line the change **removed**, and a
**deleted** file. Neither is in the file any more.

### Designed for incremental review

`differential` is built for local iteration, and nobody wants to re-read a hunk they have
already read. A review is filed under its base, and a reviewed mark keys on the hunk's own
content. So when a new commit lands, the review carries forward: what you had marked stays
marked, and your findings re-anchor themselves.

### A pull request, reviewed where you are

```sh
dfr review --pr 123      # a GitHub pull request
dfr review --mr 123      # a GitLab merge request
dfr review --pr          # the current branch's request
```

The request's review threads render under their lines, author and date on each. You can reply, comment, resolve or reopen them.

It runs `gh` or `glab`, which must be installed and logged in. Publishing is idempotent:
every comment carries a hidden marker, so a publish whose answer never came back heals on
the next one.

The review is filed under the request itself, so a force-push reopens the same review rather
than starting a new one.

Full detail: [`spec/forge.md`](https://github.com/thepartly/differential/blob/main/spec/forge.md).

## The three tiers

Every group gets one tier.

| tier | what is read |
|---|---|
| `focus` | Read every hunk, line by line. |
| `skim` | Read one exemplar per shape class. The remainder is deferred. |
| `noise` | Generated content. Folded. No exemplars. |

A fourth label appears in the output: `unclassified`. That is the back-fill group: the
model never named those classes, so nothing rated them. They are read in full.

### Read and skipped hunks

Skim exemplars are read, so a skim total is not a count of what was skipped. Every
document reports the two separately:

- `read_hunks` — focus hunks, plus one exemplar per skim class.
- `skipped_hunks` — skim remainders, plus folded noise.

`skipped_hunks` is the saving. `read_hunks` is not.

## Themes

Eleven palettes ship with it: `dark` (the default), `one-dark`, `one-light`, `gruvbox-dark`,
`gruvbox-light`, `solarized-dark`, `solarized-light`, `catppuccin-mocha`,
`catppuccin-latte`, `dracula` and `monokai`.

**[Screenshots of all eleven](https://github.com/thepartly/differential/blob/main/docs/cli.md#themes)** — the same change in the same reviewer,
only the palette differs.

## Commands

| command | what it does |
|---|---|
| `dfr review [<range>]` | Open the terminal reviewer. With no range it opens a picker. |
| `dfr stack <range>` | Build the review commit stack and land it on a ref. |
| `dfr check <range>` | Run the structural invariants. Use this in CI. |
| `dfr findings <range>` | Print the review's findings as JSON. |
| `dfr agent --doc <path>` | Print every class the grouping model may group. The model runs this, not you. |
| `dfr agents [--probe <name>]` | List the supported agents and mark the proven ones. `--probe` runs one for real. |
| `dfr clean [--dry-run]` | Delete the regenerable cache. Never touches findings. |

`--pr <N>` and `--mr <N>` stand in for a range on every command that takes one. Exit codes:
`0` success, `1` invariant or pipeline failure, `2` usage or config error.

Full reference, every flag and every default: [`docs/cli.md`](https://github.com/thepartly/differential/blob/main/docs/cli.md).

## Languages

Every file is read and every hunk is counted, whatever the language — that never depends on
a parser. What a parser buys is the **symbols**. They pay for two things: accuracy in the
dependency graph, which decides what to read before what, and the peek modal, which can only
show a declaration a parser found.

Rust, TypeScript, Python, Go and Kotlin get the most precise ordering.

See the full language coverage tables:
[`crates/symbols/README.md`](https://github.com/thepartly/differential/blob/main/crates/symbols/README.md).

## Embedding it

The mechanical layer is a library, and it runs **with no model and no network**. Link
`differential-engine` and `differential-symbols`, and `run_pipeline` gives you every hunk,
every shape class and the class dependency graph in process.

```rust
use differential_engine::{gitio::Repo, config::Config, lang::LanguageRegistry,
                          store::OsConfigSource, resolve_range, run_pipeline};

let repo = Repo::open(path)?;
let config = Config::load(&OsConfigSource, repo.root(), None, None)?;
let src = resolve_range(&repo, &["main..feature"])?;   // a ReviewSource
let out = run_pipeline(&repo, &src, &config,
                       &LanguageRegistry::builtin(),
                       &differential_symbols::readers())?;   // the symbol readers
// out.report   — the invariant report, always present
// out.document — Some(PlanDocument), or None if an invariant failed
```

`run_grouped_pipeline` adds the model stage, with the backend, the cache and the progress
sink all injected — composition is the caller's job.

Full surface: [`crates/engine/README.md`](https://github.com/thepartly/differential/blob/main/crates/engine/README.md) and
[`spec/consumers.md`](https://github.com/thepartly/differential/blob/main/spec/consumers.md).

## Config

Config is optional. There are two config files, split by who owns the setting:

- **`.differential.toml`** at the repository root — classification hints only, shared by
  everyone reviewing the repo. Which globs count as generated, and which never do. It
  honours GitHub's and GitLab's generated-file attributes by default.
- **`~/.config/differential/config.toml`** — personal. Which agent to run, which theme to wear,
  how much context the reviewer shows, and which diff layout it opens in.

A missing file means defaults. A malformed file, or an unknown key, is a hard error.

Both TOML samples and every key with its default: [`docs/cli.md#config`](https://github.com/thepartly/differential/blob/main/docs/cli.md#config).

### Agents

Five, by name. `dfr agents` lists them and says which are on your PATH.

| `agent` | CLI | what keeps it read-only | run for real? |
|---|---|---|---|
| `claude-code` | `claude` | a tool allowlist | yes |
| `codex` | `codex` | an OS sandbox — Seatbelt on macOS, bubblewrap on Linux | yes |
| `pi` | `pi` | **nothing. Pi can write, commit and push.** | yes |
| `droid` | `droid` | its own default; we pass no flag that lifts it | **no** |
| `copilot` | `copilot` | a tool allowlist, plus an explicit `--deny-tool write` | **no** |

**Read that last row before choosing it.** Pi ships no sandbox and no per-command
allowlist, and its tool switch is all-or-nothing: the shell tool the model needs to read
your diff is the same one that lets it write. Only the prompt asks it not to. Every other
agent is stopped by something.

```sh
dfr agents                 # free: what is installed, and what is configured
dfr agents --probe codex   # one real model call, four facts
```

The probe reports whether the agent started, read its prompt from stdin, could run the
fetch command, and was refused a write. The third is the one worth having: an agent that
cannot fetch does not fail, it groups worse and says nothing.

Which agent you run is part of the grouping cache key, so two agents never share an entry —
a different model may group differently. Where its binary happens to live is not, so a
cache survives a rebuild, a reinstall and a second checkout.

## The crates

| crate | what it is |
|---|---|
| [`differential`](https://github.com/thepartly/differential/blob/main/docs/cli.md) | The application. It owns the `dfr` and `differential` binaries. |
| [`differential-engine`](https://github.com/thepartly/differential/blob/main/crates/engine/README.md) | The core library: git io, diff parsing, shape classes, grouping, ordering, invariants. |
| [`differential-stack`](https://github.com/thepartly/differential/blob/main/crates/stack/README.md) | The shadow-branch renderer. The diff as a synthetic commit stack. |
| [`differential-symbols`](https://github.com/thepartly/differential/blob/main/crates/symbols/README.md) | Symbol readers: tree-sitter, and a crude fallback. |
| [`differential-tui`](https://github.com/thepartly/differential/blob/main/crates/tui/README.md) | The terminal reviewer behind `dfr review`. |

Dependency direction is strict: `cli → {tui, stack} → engine`.

## Status

`differential` is still in active development.

## Learn more

- [`docs/cli.md`](https://github.com/thepartly/differential/blob/main/docs/cli.md) — every command, every flag, every default.
- [`docs/architecture.md`](https://github.com/thepartly/differential/blob/main/docs/architecture.md) — how it works, and why it is built this
  way.
- [`spec/`](https://github.com/thepartly/differential/tree/main/spec) — the normative behaviour: the JSON contract, the invariants, each
  pipeline stage.
- [`adr/`](https://github.com/thepartly/differential/tree/main/adr) — decision records, with the measurements behind them.
- [`CREDITS.md`](https://github.com/thepartly/differential/blob/main/CREDITS.md) — third-party code and prior art.

## Development

Install the binaries from your working tree:

```sh
cargo install --path crates/cli
```

Then:

```sh
cargo test                                # unit tests and hermetic repo tests
cargo clippy --all-targets && cargo fmt
```

Changes land by pull request. CI runs format, clippy, tests and a release build on every
pull request. `main` is protected.

Releases are tag-driven. Bump the workspace version in a pull request, merge it, then push
a `vX.Y.Z` tag. The Release workflow writes the changelog into a GitHub Release and runs
`cargo publish --workspace`.

See [`AGENTS.md`](https://github.com/thepartly/differential/blob/main/AGENTS.md) for the working rules.

## Licence

MIT or Apache-2.0, at your option. See [`LICENSE-MIT`](https://github.com/thepartly/differential/blob/main/LICENSE-MIT) and
[`LICENSE-APACHE`](https://github.com/thepartly/differential/blob/main/LICENSE-APACHE).
