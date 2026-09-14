```
       ___ ________                     __  _       __
  ____/ (_) __/ __/__  ________  ____  / /_(_)___ _/ /
 / __  / / /_/ /_/ _ \/ ___/ _ \/ __ \/ __/ / __ `/ /
/ /_/ / / __/ __/  __/ /  /  __/ / / / /_/ / /_/ / /
\__,_/_/_/ /_/  \___/_/   \___/_/ /_/\__/_/\__,_/_/
```

# differential

**A terminal reviewer for diffs too large to read top to bottom.**

Most of a large branch is one decision repeated. A signature change reaches every call
site. A rename reaches every import. A lockfile is regenerated whole. The changes that
need careful reading are a small part of it, and a plain diff will not tell you which part.

`differential` works that structure out **mechanically, before any model sees the change**:
which hunks are the same edit repeated, and which changes depend on which. A model then
labels what the mechanism found. The result is a reading plan, and `dfr review` opens a
reviewer over it.

Enumeration is total. Every hunk in the range lands in exactly one group, and the partition
is checked by six structural invariants. No file is skipped, no extension is filtered, and
config cannot change that.

https://github.com/user-attachments/assets/afa7a1e6-47db-43a9-932f-f9d2b5cee321

## Requirements

- `git` on your PATH. All repository access shells out to real git.
- An agent CLI for the grouping stage. Five are supported, picked by name:
  [`claude-code`](https://claude.com/claude-code) (the default),
  [`codex`](https://developers.openai.com/codex/cli),
  [`droid`](https://docs.factory.ai/droid-exec/overview),
  [`copilot`](https://docs.github.com/en/copilot/how-tos/copilot-cli) and
  [`pi`](https://pi.dev). Each runs headless and allowed to read, never to write —
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
dfr review main..feature
```

![The dfr review reviewer, mid-review](https://raw.githubusercontent.com/thepartly/differential/main/assets/screenshot.png)

Two panes: the reading plan on the left, the diff on the right. Each group carries its
effort tier, its label, and an `after:` line naming the groups it follows. Selecting one
draws a connector to everything it depends on.

The first run on a range calls the LLM once. On a big merge request that takes a minute or
two. A splash screen shows the stages while it runs, and your terminal gets a progress bar
and a desktop notification, so you can switch windows. The result is cached, so a later run
on the same range does not call the LLM again.

Work down the plan from the top. `tab` switches panes. `j` and `k` move. `space` marks the
hunk under the cursor reviewed, or the whole selected group in the left pane. `z` on a
symbol shows what declares it. `c` writes a finding against the line under the cursor, and
`F` lists every finding you have written. `y` copies them all to the clipboard as markdown;
over SSH it sends them to your own terminal instead, and names the command that prints them.
`?` answers for where the reader is standing. `q` quits; state is written on every change.

Full detail, and every key: [`crates/tui/README.md`](https://github.com/thepartly/differential/blob/main/crates/tui/README.md).

### Reviewing what an agent just wrote

This is the case it was built for. An agent writes a branch on your machine, and you have
to read it before it becomes yours. There is no pull request yet, often no commit, and
nothing to push.

So it is a terminal program. It runs where the agent worked, beside the worktree, and it
needs no forge, no remote and no network.

```sh
dfr review
```

With no range it opens a picker. Choose the base commit, and tick the box to include
uncommitted work. Staged changes and a dirty worktree are review sources like any other.

### As a commit stack

To read the same plan in an IDE, in `tig`, or with plain `git log`, render it as a stack of
synthetic commits:

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

The stack never touches your worktree, your index, or your branches. It is built with git
plumbing and lands one ref.

Full detail: [`crates/stack/README.md`](https://github.com/thepartly/differential/blob/main/crates/stack/README.md).

## What it does that a diff viewer does not

Five things. The first two need no model at all.

### A floor that needs no model

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
everything else that deletes (ADR 0004).

This is regex, not a parser, and deliberately. Every hunk in every file needs a class — a
lockfile, a translation catalogue, a language nobody has written a grammar for. **Coverage
is 100% by construction**, and no model is involved. It is the floor, and it holds on its
own.

### It works out what depends on what

The engine builds a **class dependency graph** — definitions to uses — **from the classes,
before the model runs** (ADR 0022). So it is a fact about the diff, not about the grouping,
and the model cannot change what depends on what.

The ordering stage then contracts that graph onto groups and sorts them foundation-first,
so you meet an abstraction before its consumers. The failure it fixes was measured: in the
model's own order, the group introducing the trait everything else consumed landed **9th of
13**.

Each edge carries `via`, the symbol that produced it, so you can judge the edge rather than
trust it. Each group gets a role from its edges: `foundation`, `consumer`, `mechanical` or
`noise`. A cycle is reported, never hidden.

Full rules: [`spec/ordering.md`](https://github.com/thepartly/differential/blob/main/spec/ordering.md).

### The model raises the ceiling

The LLM merges and labels **class ids, never hunks** (ADR 0001). It never sees or names a
hunk, so it cannot drop one. The alternative was measured and rejected: asked to assign
hunks to groups directly, a model silently dropped up to **~73%** of them on large refactors
while reporting success.

It reads the same graph you do. The engine writes the pre-group document to disk and the
model fetches its own context with `dfr agent --doc <path>` — one command, one answer —
which prints every class with its `defines:`, `uses:` and `used by:` lines (ADR 0022).

If it omits a class id, an audit catches that and back-fills the class into a group that
must be read. The answer is cached by content, so a second run on the same range is
deterministic and free.

Full rules: [`spec/grouping.md`](https://github.com/thepartly/differential/blob/main/spec/grouping.md).

### A symbol says what declares it

Stand on a code row and every name on it the change declares is underlined. Press `z` and a
float opens over the declaration:

```
tokenise · crates/engine/src/shape.rs:88 · C12 · g2
```

It shows the declaration's own lines, syntax-highlighted, with their own line numbers. A
line the change wrote takes the addition colour; a line that was already there takes none.
`z` again steps to the next symbol on the row, left to right; past the last one it closes.
It never covers the row it is about.

Only what the change itself declares can be resolved, so a call into an untouched helper
lights nothing. What it can resolve, it resolves on any line it draws — context included.

### A pull request, reviewed where you are

```sh
dfr review --pr 123      # a GitHub pull request
dfr review --mr 123      # a GitLab merge request
dfr review --pr          # the current branch's, asked of the tool
```

The request's review threads render under their lines, author and date on each, replies
stepped in. `r` replies to a thread. `x` resolves or reopens it, on the forge, at once. `P`
publishes your open findings back as one review, after showing what would go and what would
stay and why. `c` and `dd` edit and delete your own comments; on anyone else's they say
`not your comment`.

There is **no token, no hostname and no HTTP client**. It runs `gh` or `glab`, which must be
installed and logged in (ADR 0029). A publish is idempotent by a hidden marker, so a publish
whose answer never came back heals on the next one.

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

`skipped_hunks` is the saving. `read_hunks` is not. On the validation range, 283 hunks were
77 focus plus 126 exemplars plus 80 remainder — a genuine saving of **28%**, not 73%.

## Themes

Eleven palettes: `dark` (the default), `one-dark`, `one-light`, `gruvbox-dark`,
`gruvbox-light`, `solarized-dark`, `solarized-light`, `catppuccin-mocha`,
`catppuccin-latte`, `dracula` and `monokai`.

A theme is a name, not a colour list. Each is derived from one seed — a syntax theme plus
six declared accents, with thirty-odd more colours mixed against the syntax theme's ground —
so the chrome and the code cannot disagree (ADR 0024). It is a user setting only: a theme in
the repository's config file is rejected, because a palette is the reader's choice.

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
a parser. What a parser buys is the **reading order**: which change to open before which.

Rust, TypeScript, Python, Go and Kotlin get the most precise ordering. JavaScript, Java, C,
C++ and C# follow. Seventeen more are read by a regex, and anything else contributes no
dependency edges — it is still enumerated, classified and read.

Full table, and what each rung costs:
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

The JSON plan document is the contract every renderer reads. It is frozen at
`schema_version` 3, additive changes only. Ids are document-local and positional;
`hunks[].digest` is the stable anchor that survives regeneration.

Full surface: [`crates/engine/README.md`](https://github.com/thepartly/differential/blob/main/crates/engine/README.md) and
[`spec/consumers.md`](https://github.com/thepartly/differential/blob/main/spec/consumers.md).

## Config

Config is optional, and **it never removes a file or a hunk from analysis**. It tunes
classification hints and tool behaviour only. Every invariant depends on that.

Two files, split by who owns the setting:

- **`.differential.toml`** at the repository root — classification hints only, shared by
  everyone reviewing the repo. Which globs count as generated, and which never do.
- **`~/.config/differential/config.toml`** — yours. Which agent to run, which theme to wear,
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

**The last column is not a formality.** Every command line here was written from its
agent's documentation, and a test can check the string this repo builds but never that the
real CLI accepts it. Three have now been run. **Two of those three were broken** — Claude
Code's allowlist did not bind without `--permission-mode default`, and Codex was being
passed `--ask-for-approval`, which its `exec` subcommand rejects outright. Both errors came
from docs that were right about the product and wrong about the subcommand.

So treat `droid` and `copilot` as probably wrong rather than merely unconfirmed. If you have
one, `dfr agents --probe <name>` settles it in one call — and a passing run is the evidence
for promoting it.

**Read that last row before choosing it.** Pi ships no sandbox and no per-command
allowlist, and its tool switch is all-or-nothing: the shell tool the model needs to read
your diff is the same one that lets it write. Only the prompt asks it not to. Every other
agent is stopped by something. ADR 0033 records why Pi is offered anyway.

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

Shipped: the full pipeline, the review TUI (`dfr review`) with findings that survive
regeneration, and the shadow-branch renderer (`dfr stack`).

Shipped, first cut: reviewing a GitHub pull request or a GitLab merge request in place —
`dfr review --pr 123` / `--mr 123` shows the request's review threads under their lines,
`P` publishes the open findings back as one review, `x` resolves a thread. It runs `gh` or
`glab`, which must be installed and logged in ([spec/forge.md](https://github.com/thepartly/differential/blob/main/spec/forge.md)). GitLab is
not yet verified against a live instance.

## Name

> A differential is a gear system in vehicles that lets driven wheels rotate at different
> speeds while still receiving power from the engine.

The word already contains "diff", and the gear is the arrangement: each group is read at
its own speed, and every hunk is still carried.

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
