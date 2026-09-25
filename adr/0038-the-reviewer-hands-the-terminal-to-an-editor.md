# 0038 — The reviewer hands the terminal to an editor

Status: accepted

Follows [ADR 0034](0034-search-reads-the-files-as-they-are.md), whose rule this key reads
under: the corpus is the files as they are now. The key itself is an action in
[ADR 0036](0036-keys-are-bound-by-action.md)'s table, `external-editor`, so a reader who
wants it somewhere other than `e` rebinds it like any other. Bounded by
[ADR 0020](0020-ports-and-static-dispatch.md), which makes `crates/tui` and `crates/cli`
adapters — no new port and no new `dyn` seam is added here.

## Context

A reviewer reads a diff and finds the thing to change. Until now they left the tool,
remembered the path, remembered the line, and opened it by hand. `e` closes that
(issue #152).

Three things had to be decided, and each of them argues with something already written
down. That is what this record is for.

## Decision

**1. The command is the reader's, and it is a command rather than a name.**

`[review].editor` holds a command line. `{file}` and `{line}` say where the path and the
line go. Unset, the tool reads `$VISUAL` and then `$EDITOR`.

`spec/consumers.md` says twice that a config value here is a name and not an argv, and it
is right both times. `agent` is a name because the grouping stage hands its subprocess a
tool allowlist, a fetch command and a prompt written for what that agent can do
([ADR 0022](0022-the-model-fetches-its-own-context.md),
[ADR 0033](0033-what-makes-an-agent-supportable.md)); an argv got the prompt and none of
the rest. `theme` is a name because a palette is thirty-odd values derived together so the
chrome and the code cannot disagree ([ADR 0024](0024-palettes-are-derived-and-threaded.md)).
In both, the value the user supplies is a fraction of the thing it names, so a free-form
value was a knob that looked like it worked.

**Neither reason reaches an editor.** The invocation carries a path and a line and nothing
else. There is no allowlist to bind, no prompt to write, nothing derived. A command is the
whole of what the feature needs, so a command is what it takes.

The alternative was an enum — `vim`, `nvim`, `helix`, `vscode`, `emacs` — because every
editor spells the line differently and a table would have hidden that. It was rejected
because of who the table excludes. A name that is not in it cannot be used at all, so the
enum would have been this crate deciding which editors exist, and adding one would be a
release. That is the same failure as the knob that looked like it worked, pointed the
other way.

What the command costs, stated rather than discovered:

- **The reader has to know their editor's line flag.** `nvim +{line} {file}` is not
  discoverable from the key. The docs carry a table of the common ones.
- **A bare `EDITOR=vim` names no placeholder**, so the path is appended and the file opens
  at the top. That is the fallback working as designed, and it is indistinguishable from a
  broken jump unless the tool says so — which is why the footer says
  `the command has no {line}`.

**The two sources fail differently, and that is deliberate.** A `[review].editor` that
does not split is a hard error at startup, like every other malformed config value: the
reader wrote it for this tool. A `$VISUAL` or `$EDITOR` that does not split is skipped,
because it was written for every program on the machine, and stopping `dfr review` from
opening over it would punish a reader who never asked for this key. `e` then says there is
no editor, which is true.

`shlex` does the splitting rather than a hand-rolled scanner (design rule 5). Quoting is
the whole of the problem, and a program path with a space in it is the case a hand-rolled
splitter gets wrong.

**A placeholder may not be the first word.** Substitution reaches every word, so
`editor = "{file}"` would make the program the file under the cursor. The tempting reading
is that the spawn simply fails and the footer says so — but on a source file carrying the
executable bit it does not fail, it RUNS. A key for reading a diff must not be one keystroke
from executing what it is reading, so the command is refused at load, where every other
malformed config value is.

**2. The handoff is not `engine::subprocess`, and the reason is the terminal.**

Every other child in this workspace goes through `engine::subprocess::run`: the agent
([ADR 0016](0016-llm-backend-abstraction.md)) and `gh`/`glab`
([ADR 0029](0029-the-forge-consumer.md)). It pipes all three streams under a deadline and
a cancel flag. Both properties are correct there and wrong here.

- **Piped stdio.** An agent's bytes are the answer. An editor's bytes ARE the terminal,
  and a child that cannot see the keyboard cannot be typed in.
- **A deadline.** Right for a call that must not hang a pipeline. Here the reader is the
  deadline, and an editor killed at twenty minutes is an editor that ate an afternoon's
  note.

So `e` is a plain blocking spawn with inherited stdio, in `crates/tui/src/launch.rs`. That
is a second spawn shape in the workspace, and the boundary between them is the terminal: a
child whose output we read goes through `subprocess`; a child that takes the screen goes
through `launch`. There are exactly two of the second kind possible and one exists.

This reverses what was written when the terminal guard was vendored. `vendor/terminal.rs` cut
upstream's suspend/resume pair, saying "nothing here shells out to a foreground process".
That was true and is not any more, so the pair is back, and the note says why.

**Coming back costs three things**, and skipping any of them is visible on screen:

1. **Wipe the screen.** ratatui caches the frame it last drew and writes only the cells
   that changed. The editor wrote over all of them, so without the wipe the reviewer
   paints a handful of cells onto somebody else's output.

   **Not with `Terminal::clear`**, which is the obvious call and the wrong one. Since
   ratatui 0.30 it snapshots the cursor with a `CSI 6 n` round trip so it can put it back,
   and a terminal that does not answer inside crossterm's short window returns an error —
   which, on the way back from an editor, takes the whole reviewer down. That is not a
   hypothetical: it was written that way first, and a run under a pseudo-terminal failed
   with `the cursor position could not be read within a normal duration`. `Terminal::resize`
   clears the viewport and resets the back buffer through the same private helper and asks
   the terminal nothing, so that is what `resume` calls.
2. **Measure.** A terminal can be resized while the editor holds it, and the model's
   geometry would be a lie.
3. **Drain.** Keys the reader pressed at the editor that it did not consume are still
   queued, and replaying them at the reviewer is a `dd` nobody meant.

**3. The reviewer does not reload, and the footer says so.**

A plan document is a pure function of its range and is never patched (`spec/persistence.md`).
The diff on screen is therefore the diff that was generated, whatever was just written to
the file. Reloading is re-running the pipeline — a different feature, with its own question
about what happens to the reader's cursor, their marks and their notes.

`edited src/x.rs · the diff is not reloaded`, every time. A feature that quietly went stale
would be worse than one that says it has.

The line `e` opens at is a HEAD-side line, and the file it opens is the worktree's. Those
coincide for a review of uncommitted work and diverge for a committed range reviewed from a
moved checkout. ADR 0034 already took this trade for `/` and the reasoning is the same:
one rule a reader can hold beats a side they have to reason about on every row.

**4. Where the key lands when the row is not a line.**

The diff pane always has an answer, narrowing to what the row can say: the row's own
new-side line, else the hunk's first new-side line, else the file at the top. A removed
line, a hunk header, a `──` boundary, a finding and a thread all reach the second answer.
The
plan pane has one answer — the selected file, at the top — and a group and a directory get
none, because they are not files.

Refusing on a row with no line was the alternative, and it is what `z` does with no symbol.
It was rejected because the two keys answer different questions. `z` shows a thing that is
either there or not. `e` takes the reader somewhere, and the file is always there.

**5. The config modal edits the command, and clearing the row means the environment.**

`[review].editor` is a row in `:config` ([ADR 0037](0037-a-command-line-and-a-config-modal.md))
like every other personal setting, typed and checked on the spot: a command this tool could
not run is refused in the modal, not at the next press of `e`.

Clearing it is the case worth stating. Unset means `$VISUAL`, then `$EDITOR` — it does not
mean *no editor*, and an empty row beside the word `default` would read as the second. So
the row names the fallback, and the renderer is handed what the environment says
(`ReviewOptions::editor_env`) alongside the resolved command. Without that second value,
clearing the row while the reviewer ran would leave `e` dead until the next start for a
reader who had never written `[review].editor` in the first place — a setting that breaks
what it was not asked about.

The resolution goes through `show_review`, the one function ADR 0037 routes the draft, the
original and the saved config through, so a preview, an `esc` and a save cannot disagree
about which command `e` holds.

## Consequences

- **One dependency**, `shlex`, in the engine. It is pure and has no dependencies of its
  own; it was already in the lock file, pulled in transitively.
- **A second spawn shape**, with the boundary written down above so a third is a decision
  rather than a drift.
- **`crates/tui/src/launch.rs` is private.** It takes the terminal session, which is this
  crate's own, and only the run loop has one to give it.
- **`Effect` gains a third variant.** The model still decides and the loop still acts, so
  `handle_key` stays testable without a terminal — the tests assert the effect, never a
  spawn.
- **Nothing lands in the engine but the config type.** `EditorCommand` parses a value and
  builds an argv; it names no adapter, spawns nothing and reads no environment. The
  environment is read in `crates/cli`, beside `fetch_command`.
- **"Editor" now means two things in `crates/tui`**, and the other one came first: the
  finding composer is a `tui-textarea` in `Mode::Editing { editor, .. }`. The new module is
  named `launch` rather than `editor` for that reason, and its doc comment opens by saying
  so.
