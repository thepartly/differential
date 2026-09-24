# Terminology

One word per concept. Each entry names the canonical term, the code identifier where there
is one, a line of definition, and the spec or ADR that owns it. **Avoid** lists words the
docs have used for the same thing; do not use them in new text. This file defines words and
decides nothing: when an entry and the spec it points to disagree, the spec is right and this
file is the bug.

A new term earns an entry only when it names a concept no existing entry covers.

## Diff and git

- **hunk** (`model::Hunk`, wire `hunks[]`). One canonical hunk of the `-U0 --no-renames` diff.
  [json-contract.md](json-contract.md), [ADR 0003](../adr/0003-dual-diff-views.md).
- **pseudo-hunk**. The `Subproject commit` lines of a gitlink change: counted, never
  reconstructed. [json-contract.md](json-contract.md).
- **hunk id** (wire `h<N>`, in memory `plan::HunkId`). A positional id; it does not survive
  regeneration. [json-contract.md](json-contract.md).
- **hunk digest** (`hunks[].digest`). The exact content hash of a hunk's removed and added
  bytes, un-normalised. It survives regeneration and a clean rebase, and it is what
  persistence keys on. [ADR 0025](../adr/0025-reviewed-marks-key-per-hunk.md).
- **canonical view** / **rename-detected view**. `--no-renames` drives enumeration, ids,
  reconstruction and the invariants. `-M` only annotates, for classification.
  [ADR 0003](../adr/0003-dual-diff-views.md).
- **enumeration**. The first stage. It lists every changed file and hunk, and it is total:
  no extension filter, no path exclusion, no size cutoff. Withholding symbols is
  classification, never enumeration. [ADR 0005](../adr/0005-no-extension-filter.md),
  [ADR 0012](../adr/0012-config-never-excludes.md).
- **disposition** (`Disposition`: A/D/M). A file's add, delete or modify. It is part of the
  class key.
- **rename similarity** (`rename_similarity`) / **relocation gate**. A similarity below ~95
  is a modification, not a relocation. The gate lifts such classes into a "Modified during
  move" focus group, the **gate group**. [grouping.md](grouping.md).
- **zero-hunk file**. An empty-file add or delete, a mode-only change, or a binary. It is a
  real entry. A submodule change is not one: its pseudo-hunk is counted.
  [json-contract.md](json-contract.md).
- **submodule change** (wire `submodule`, `SubmoduleChange`). A gitlink's pointer moved. It
  contributes no bytes to reconstruction and no symbols. [json-contract.md](json-contract.md).
- **range** (`plan::RangeSpec`) / **base** / **head**. `a..b`, `a...b` (base = merge-base)
  or two revs. [consumers.md](consumers.md).
- **review source** (`plan::ReviewSource`, `source.kind`). Where the diff comes from:
  `range | commit | staged | worktree | pr | mr`. **Uncommitted** sources are `staged` and
  `worktree`. [ADR 0017](../adr/0017-uncommitted-review-sources.md).
- **plumbing**. The only git the tool runs, bar `git fetch` behind the Fetcher.
  [ADR 0011](../adr/0011-plumbing-over-porcelain.md).
- **Fetcher** (`ports::Fetcher`). The one port that runs `git fetch`, for a request's refs.
  [ADR 0029](../adr/0029-the-forge-consumer.md), [forge.md](forge.md).

## Classification

- **shape class**, **class** for short (`schema::ClassEntry`, ids `C0…`). Hunks with the same
  class key: identical diff text after normalising identifiers and literals on both sides,
  the same disposition, the same generated flag. Numbered by descending member count. The
  partition of hunks into classes is the **mechanical partition**. Avoid: "shape" alone, "cluster".
  [ADR 0004](../adr/0004-shape-hash-both-sides.md).
- **class key** (computed by `shape::shape_hash`, the **shape hash**). What a class forms on.
  Avoid: "shape key". [ADR 0004](../adr/0004-shape-hash-both-sides.md).
- **normaliser** (`Language::normalize_line`, `lang/generic.rs`). It erases identifiers and
  literals from a line. Prose spells it the British way, and code keeps `normalize`. The
  **generic normaliser** is frozen. [ADR 0015](../adr/0015-language-abstraction.md).
- **language plugin** (`lang::Language`). Normalisation for one language, identified by its
  `id()`; the registry's `fingerprint()` combines those ids. It does normalisation and
  nothing else, and it is not a symbol reader.
  [ADR 0015](../adr/0015-language-abstraction.md),
  [ADR 0023](../adr/0023-symbol-extraction-is-a-domain-port.md).
- **exemplar**. The member a reader reads to verify the whole class. The members left over
  are the **skim remainder**. [json-contract.md](json-contract.md), [grouping.md](grouping.md).
- **pure substitution** (`pure_substitution`). Computed, never claimed: the removed and added
  lines match once identifiers and literals are erased. [json-contract.md](json-contract.md).
- **generated** (`generated`, `generated_by`: `builtin | attr | config`). A computed hint for
  the noise tier, and part of the class key, so a class is wholly generated or wholly not.
  It never affects enumeration. [ADR 0004](../adr/0004-shape-hash-both-sides.md).
- **symbol reader** (`artefact::symbols::SymbolSource`). A reader that extracts definitions
  and references from a file. The best **claimant** answers; a reader that claims a file and
  cannot read it **declines**, and the next one answers.
  [ordering.md](ordering.md), [ADR 0023](../adr/0023-symbol-extraction-is-a-domain-port.md).
- **rung**. One of the four symbol readers, ranked by precision: **tuned**, **field-rule**,
  **single-file**, **crude**. Falling a rung costs precision, never coverage. Avoid: "regex
  floor" and "naive" for crude, "field rules" for field-rule.
  [ordering.md](ordering.md), [symbols README](../crates/symbols/README.md).
- **definition** (`schema::SymbolDef`). A name others can use.
  [ordering.md](ordering.md), [ADR 0030](../adr/0030-what-counts-as-a-definition-and-how-far-it-reaches.md).
- **reference** (`schema::SymbolUse`). A name reached by path, whether or not it is called.
  In the symbol index, a reference on a line the change wrote is a **use**.
  [ordering.md](ordering.md).
- **global** / **file-local** (`Scope`). A global name draws edges across files. A
  file-local one draws edges only inside its own file.
  [ADR 0030](../adr/0030-what-counts-as-a-definition-and-how-far-it-reaches.md).
- **binding**. A declaration in every position that introduces one. [ordering.md](ordering.md).
- **namespace** (`FileSymbols.namespace`). An opaque token a reader attaches to a file's
  names, in practice a language family (`js`, `jvm`, `c`). Two global names match only when
  their namespaces match.
  [ADR 0031](../adr/0031-a-global-name-is-scoped-to-its-language.md).
- **site** (`artefact::symbols::Site`). Where a symbol is. A definition's site also says how
  far it reaches (`through`). [ADR 0032](../adr/0032-a-dependency-carries-its-site.md).
- **single-definer rule**. Only a symbol defined by exactly one class creates an edge.
  [json-contract.md](json-contract.md).
- **class graph** (`artefact::graph`, `defines` / `depends_on`, `ClassEdge`). Class-to-class
  dependencies, computed before the grouping stage, so they are a fact about the diff. An
  edge's **via** lists the symbols that produced it. Avoid: "symbol dependency graph".
  [ADR 0022](../adr/0022-the-model-fetches-its-own-context.md).
- **symbol index** (`symbols`, `schema::SymbolIndex`). Which token on which line resolves to
  which definition. [json-contract.md](json-contract.md).
- **masking**. A single-file component is read as its script blocks with everything else
  blanked. Masking is not a filter.
  [ADR 0035](../adr/0035-a-single-file-component-is-masked-not-injected.md).

## Grouping

- **group** (`schema::Group`, ids `g0…`). Shape classes merged and labelled by intent, and
  rated with an effort tier. [grouping.md](grouping.md).
- **grouping**. The stage that produces groups, or one cached model answer. Never a single
  group.
- **grouping stage** (`group`). It turns the mechanical partition into labelled groups. The
  model merges and labels class ids, never hunks.
  [ADR 0001](../adr/0001-llm-merges-class-ids-never-hunks.md).
- **agent** (`config::Agent`, `[grouping].agent`). The CLI tool (`claude`, `codex`, …) that
  runs the grouping stage, chosen by name.
  [ADR 0033](../adr/0033-what-makes-an-agent-supportable.md).
- **grouping model**. The model inside the agent, as the prompt addresses it.
- **LLM backend** (`llm::LlmBackend`). One-shot completion: prompt in, raw text out. Its
  **name** is shown to the reader. Its **identity** is everything about it that could change
  a grouping, and is part of the grouping cache key.
  [ADR 0016](../adr/0016-llm-backend-abstraction.md), [consumers.md](consumers.md).
- **how read-only is enforced** (`config::ReadOnly`). Per agent: OS sandbox, tool allowlist,
  agent default, or not enforced. It has no one-word name; do not call it a tier.
  [ADR 0033](../adr/0033-what-makes-an-agent-supportable.md).
- **fetch command** (`dfr agent`). The executable the grouping model runs to read the
  pre-group document. [consumers.md](consumers.md).
- **probe** / **proven** (`dfr agents`). The probe checks an agent spawns, reads its prompt,
  can run the fetch command and is refused a write. A proven agent is one someone probed.
  [grouping.md](grouping.md).
- **pre-group document** (`cache/document/`). The document with `groups: null`, written for
  the grouping model to read. [ADR 0022](../adr/0022-the-model-fetches-its-own-context.md).
- **offered class**. A non-generated class sent to the model. Generated classes go to one
  folded noise group and are never offered. [grouping.md](grouping.md).
- **coverage audit** (`audit.classes_missing` / `duplicated` / `hallucinated`). It checks the
  model's answer against the offered id set. [grouping.md](grouping.md).
- **back-fill group** (`[unclassified]`). The trailing focus group that holds every class
  the model omitted (invariant 5).
  [invariants.md](invariants.md).
- **grouping cache** (`GroupingCache`, `cache/grouping/`). Groupings are pinned by a key over
  `PROMPT_VERSION`, the backend identity, the fingerprints and the offered classes' hunk
  digests. [ADR 0009](../adr/0009-groupings-pinned-by-content-hash.md).

## The document

- **document** (`schema::PlanDocument`). The one JSON document the pipeline produces. Avoid:
  "plan" alone. The pre-group document's store is the **artefact store**
  (`FsArtefactStore`). [json-contract.md](json-contract.md).
- **reading plan** (`reading_plan[]`). Groups ordered foundation-first, each with its read
  action. It is also what the whole tool produces, and the title of the plan pane.
- **schema version** (`SCHEMA_VERSION`, 3). Additive changes keep it and breaking changes
  bump it. [ADR 0022](../adr/0022-the-model-fetches-its-own-context.md).
- **`generator.stages`**. Exactly the stages that ran. A consumer reads it, never field
  presence. [json-contract.md](json-contract.md).
- **audit** (`schema::Audit`). The structural checks on every document, plus the coverage
  fields the grouping stage fills. [json-contract.md](json-contract.md).
- **read hunks** / **skipped hunks**. Focus hunks plus exemplars, against skim remainders
  plus folded noise. Only the second is a saving. [overview.md](overview.md).
- **forge position** (`forge_position`). A hunk's first line on each side, in the canonical
  view. It is not a posting anchor. [json-contract.md](json-contract.md).
- **plan hash** (`plan::plan_hash`). The content address of a stored document.
  [persistence.md](persistence.md).

## Ordering

- **effort tier**, **tier** for short (`schema::Effort`, wire `effort`): **focus** (read
  every hunk), **skim** (read one exemplar per class), **noise** (folded, no exemplars). An
  unknown value reads as focus. Focus was once named `close`.
  [ADR 0006](../adr/0006-three-effort-tiers.md),
  [ADR 0019](../adr/0019-focus-tier-and-schema-v2.md).
- **reading split** / **deferral** / **fold** (`plan::ReadingSplit`, `Deferral`, `Fold`).
  What a tier shows, and what it defers by default. Deferring is an opinion, not a
  prohibition. [ADR 0006](../adr/0006-three-effort-tiers.md).
- **ordering stage** (`order`). It contracts the class graph onto groups and sorts them.
  Deterministic and model-free. [ordering.md](ordering.md).
- **foundation-first**. The topological sort over the contiguous focus prefix.
  [ordering.md](ordering.md).
- **role** (`schema::Role`): foundation, consumer, mechanical, noise.
  [ordering.md](ordering.md).
- **group edge** (`schema::Edge`). The class graph contracted onto groups. It may form a
  **cycle**: an **artefact** cycle exists only because classes were contracted, and a
  **mutual** cycle is in the change itself. [ordering.md](ordering.md).
- **rank** / **pivot**. Rank is a group's final reading position. Pivot counts its leading
  classes that depend on nothing ranked later. [json-contract.md](json-contract.md).

## Consumers and the terminal reviewer

- **consumer**. A view over the document that must not influence its shape. There are three:
  the shadow branch, the terminal reviewer and the forge consumer. [overview.md](overview.md).
- **renderer**. A library crate that draws the document: `crates/stack`, `crates/tui`. The
  forge consumer is a consumer but not a renderer.
  [ADR 0018](../adr/0018-crate-consolidation-and-renderer-crates.md).
- **shadow branch** (`dfr stack`, `crates/stack`). The document rewritten as a synthetic
  commit stack. [stack.md](stack.md).
- **terminal reviewer**, the **TUI** (`dfr review`, `crates/tui`). The two-pane reviewer.
  [tui.md](tui.md).
- **reader** / **reviewer**. In new text the reader is the human and the reviewer is the
  program. Older spec text uses "reviewer" for the human too.
- **plan pane** / **file view** / **diff pane**. The left pane shows the groups, or the file
  tree on `f`. The right pane shows the diff. [tui.md](tui.md).
- **float**. An overlay over the panes: the map, the symbol float, the finding composer.
  Avoid: "peek modal". [tui.md](tui.md).
- **window** / **boundary row**. The reviewer renders only computed line ranges. A boundary
  row is the control that widens one. [ADR 0021](../adr/0021-windowed-diff-reconstruction.md).
- **crossed hunk** / **foreign hunk**. A crossed hunk is another group's hunk pulled into a
  window. A foreign hunk is any hunk not on the current group's reading list. [tui.md](tui.md).
- **picker**. What `dfr review` opens with no range: a base and an uncommitted checkbox.
  [tui.md](tui.md).
- **external editor** (`[review].editor`, `crates/tui/src/launch.rs`). The program `e` hands
  the terminal to, on the line under the cursor. Its command is the reader's, with `{file}`
  and `{line}` placeholders. Always qualified, because the **finding composer** is an editor
  too. [tui.md](tui.md), [ADR 0038](../adr/0038-the-reviewer-hands-the-terminal-to-an-editor.md).
- **theme** / **seed** / **accent**. A theme declares a seed: a syntax theme and six accents,
  from which every other colour is derived.
  [ADR 0024](../adr/0024-palettes-are-derived-and-threaded.md).
- **action** / **binding** (`config::Action`, `keymap::Binding`). An action is something the
  reviewer does on a key, by the name `[keys]` uses: `down`, `delete`. A binding is the key,
  or the sequence of keys, it answers to. A **screen** (`keymap::Screen`) is where a key is
  looked up: the review, the file list, the findings list. Avoid: "shortcut", "command".
  [ADR 0036](../adr/0036-keys-are-bound-by-action.md).
- **command line** / **config modal**. `:` opens the command line on the status row, which
  runs a command by name (`:findings`). `:config` opens the config modal, which edits the
  user file and saves it whole. [ADR 0037](../adr/0037-a-command-line-and-a-config-modal.md).
- **projection**. A renderer-agnostic read model: `plan::ReviewView` over the document, and
  the findings a `ReviewSession` hands to `y` and `dfr findings`. Shared domain policy lives in
  `engine::plan`.

## Review state

- **sidecar** (`reviews/<review-id>/`). Review state kept beside the regenerated documents,
  never in them. [ADR 0013](../adr/0013-incremental-review-sidecar-state.md).
- **review session** (`ReviewSession`). The engine's persistence facade a renderer opens.
  [persistence.md](persistence.md).
- **named session** (`--name`). A review whose name is its whole identity.
  [ADR 0027](../adr/0027-a-named-review-session.md).
- **sitting**. One run of the terminal reviewer. What only lasts a sitting is not persisted.
- **review identity** (`ports::ReviewIdentity`: range, named, remote). What a review is filed
  under: base plus the head as typed (its **spelling**), a name, or a request.
  [persistence.md](persistence.md).
- **adoption**. A spelling with no review of its own adopts one on the same base when one head
  reaches the other. [ADR 0026](../adr/0026-a-review-adopts-an-ancestor.md).
- **finding** (`review_state::Finding`, `findings.jsonl`). The reader's own note on a line or
  range. Avoid: "comment" for a finding. [persistence.md](persistence.md).
- **anchor** (`Anchor`). A hunk digest plus an offset inside that hunk, never a line number.
  [persistence.md](persistence.md).
- **re-anchoring** (`reanchor`). Exact digest, then fuzzy (`moved`), then **orphaned**. An
  orphaned finding is never deleted. [persistence.md](persistence.md).
- **reviewed mark** (`reviewed_hunks`). Keyed per hunk digest.
  [ADR 0025](../adr/0025-reviewed-marks-key-per-hunk.md).

## Forge

- **forge** (`forge::Forge`). GitHub through `gh`, GitLab through `glab`.
  [ADR 0029](../adr/0029-the-forge-consumer.md).
- **request** (`forge::Request`). A pull request or a merge request, when the distinction does
  not matter. Say PR or MR only when it does. [forge.md](forge.md).
- **forge consumer**. The consumer that shows a request's threads and publishes findings.
  Avoid: "forge review", "forge poster" as its name. [forge.md](forge.md).
- **thread** (`RemoteThread`). One forge discussion: a root comment and its replies. A
  **comment** (`RemoteComment`) is always the forge's. [forge.md](forge.md).
- **outdated**. A thread whose line has left the request's diff. [forge.md](forge.md).
- **publish** (`P`, `forge::Batch`). Posting findings as one batched, confirmed act. A finding
  that was published has an **upstream**. [forge.md](forge.md).
- **head check**. Before publishing, the request's head must equal the review's head.
  [forge.md](forge.md).
- **marker** / **twin**. The hidden marker makes a publish idempotent. A published finding
  hides behind its fetched twin. [forge.md](forge.md).

## Architecture

- **engine**, the **core** (`differential-engine`). The library that owns the pipeline.
  Avoid: "backend" for the engine. [ADR 0014](../adr/0014-core-is-a-library.md).
- **application layer** (`crates/cli`). The `dfr` and `differential` binaries: presentation
  and dispatch only. [ADR 0018](../adr/0018-crate-consolidation-and-renderer-crates.md).
- **domain** / **port** / **adapter** (`engine::ports`, `gitio::Repo`). The domain owns the
  port, and the adapter implements it. A port describes the need, not the tool, and is used
  as a generic bound. [ADR 0020](../adr/0020-ports-and-static-dispatch.md).
- **git ports**. The `engine::ports` traits for git. Their only implementation is
  `gitio::Repo`.
- **seam**. One of the four `dyn` abstractions whose implementation is a run-time answer:
  `LlmBackend`, `Language`, `SymbolSource`, `Forge`. Not a synonym of port.
- **bound list**. The ports in a function's generic bounds, which state how much git it may
  touch. Never merged into a supertrait. [ADR 0020](../adr/0020-ports-and-static-dispatch.md).
- **write boundary**. `run_pipeline` and `run_grouped_pipeline` carry no write port, so the
  pipeline is read-only; every write port sits in `pipeline::verify`.
  [ADR 0028](../adr/0028-the-verify-stage-is-separate.md).

## Pipeline and verification

- **pipeline** / **stage**. `enumerate → classify → group → order`, then `verify` when a
  caller opts in. [overview.md](overview.md).
- **invariant**. Checks on every document: **1b** no enumeration hole, **1** applier
  fidelity, **2** hunk accounting. Checks in the verify stage: **3** tree assertion, **4**
  independent recount. The grouping check: **5** nothing unassigned is dropped. 1b, 1 and 2
  are the **core** invariants. [invariants.md](invariants.md).
- **recount**. Invariant 4's deliberately dumb counter. It shares no code with the diff
  parser. [invariants.md](invariants.md).
- **verify stage** (`verify`, `dfr check`). The one stage a caller opts into. Its absence
  reads as "did not run", never as a pass.
  [ADR 0028](../adr/0028-the-verify-stage-is-separate.md).
- **validation corpus**. The private repository the parity test runs against. Nothing
  committed may reference it.
- **parity test**. The ignored test whose counts (files, hunks, classes, recount) must match
  the fixture exactly.
- **privacy sweep**. The `git grep` for corpus markers before every push.

## One word, several meanings

Each word below has one canonical meaning. Say the others another way, or qualify them.

| word | canonical meaning | other meanings, and how to say them |
|---|---|---|
| tier | effort tier | a reader precision level is a **rung**; how read-only is enforced is `config::ReadOnly` |
| consumer | a view over the document | the role is `consumer`, in code font; a second user of an abstraction is a **caller** |
| foundation | `Role::Foundation` | the sort is **foundation-first** |
| mechanical | the mechanical partition, no model | the role is `mechanical`, in code font |
| artefact | `engine::artefact`, the class graph and symbols | an **artefact cycle**; the pre-group **artefact store**; a stored document is a **document** |
| contract | the JSON contract | the ordering stage **contracts** the class graph onto groups |
| agent | the CLI tool that runs the grouping stage | `dfr agent` is the **fetch command**; a coding agent reads `AGENTS.md` |
| fetch | the grouping model reading the pre-group document | `git fetch` runs behind the **Fetcher**; threads are **refetched** |
| stack | the shadow branch's commit stack | the ordered groups are the **reading plan** |
| focus | the effort tier | the pane with input has **pane focus** |
| fold | one row standing for hidden content | say what is folded: a skim remainder, a noise group, a directory, the map |
| mark | a reviewed mark | the forge **marker**; an idle header's **marks** |
| identity | review identity | **backend identity**; commit identity |
| session | `ReviewSession` | a **named session**; one run is a **sitting** |
| plan | `engine::plan`, the domain policy module | the **document**, the **reading plan**, the **plan pane** |
| hash | the shape hash, which computes the class key | the **hunk digest**, the **plan hash**, the grouping cache key |
| gate | the relocation gate | the export gate of ADR 0030 |
| read-only | the pipeline, which carries no write port | how an agent's read-only is enforced (`config::ReadOnly`) |
| comment | a forge comment | the reader's is a **finding** |
| backend | `LlmBackend` | the library is the **engine** |
| editor | the reader's own, that `e` opens — the **external editor** | the in-reviewer one is the **finding composer** |
