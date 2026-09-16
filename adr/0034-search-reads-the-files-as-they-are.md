# 0034 — Search reads the files as they are, not the diff

Status: accepted

Extends [ADR 0006](0006-three-effort-tiers.md), whose clarification is what permits a hit
inside a deferred remainder. Nothing here changes the tiers, the counts or the audit.
Bounded by [ADR 0005](0005-no-extension-filter.md) and
[ADR 0012](0012-config-never-excludes.md): a search is a view's ordering and never a filter
on enumeration.

## Context

The reviewer could move to a group, to a file and to a hunk. It could not move to a **name**.
A reader who remembered what a thing was called and not where it lived had to leave the tool
for `grep`, and come back with a line number the tool had no key for reaching.

That is a plain gap, and the surrounding machinery already had every part of the answer:

1. **The blobs are in memory.** `RowFactory` caches both sides of every file it draws
   (ADR 0021), and reads many of them in one `git cat-file --batch` call.
2. **`declaration` already builds a highlighted snippet** with its own line numbers, for
   the symbol float (ADR 0032). A preview is that function with different arguments.
3. **`jump_to_finding` already reaches a row wherever it lives** — select the owner, let the
   rows rebuild, find the row again by what identifies it, opening a fold on the way.

So the decisions left were about what a search is *of*, and what it ranks.

## Decision

**1. The corpus is every changed file's head side — the files as they are now.**

Not the hunks, and not the hunks plus the removed lines. Unchanged lines are in scope,
because that is where most of the names a reader is hunting for live: a definition whose
call sites moved was not itself touched, and a search that only saw the diff would answer
"no" to half the questions worth asking.

What it costs, stated rather than discovered:

- **A line the change removed is not found.** It is not in the file any more.
- **A deleted file holds nothing at all**, for the same reason.

What it buys is one rule a reader can hold. Head plus the removed lines was the alternative,
and it covers deleted files for free — every line of a deletion is a removed line. It was
rejected because it makes a hit's *side* a thing the reader has to reason about, on every
row, to gain the one question this tool is worst at answering anyway: the old code is what
the branch is getting rid of.

A binary file holds no text to find. It is still enumerated, still counted and still in
every total; not searching it is classification, exactly as the noise tier is.

**2. A hit inside a deferred skim remainder or a folded noise group is reachable, and
`enter` opens the fold.**

ADR 0006 permits this in as many words — "deferring is an opinion, not a prohibition" — but
only where the reviewer overrides the default *deliberately, on a specific hunk, having been
told what it is*. So every occurrence row carries its group and its tier (`g1 skim`), and
the reader knows what they are opening before they open it.

Nothing else moves. A hit count is not coverage and is not a saving: `read_hunks` and
`skipped_hunks` describe the plan, never a reader's route through it.

**3. The ranking, in the order it answers.**

1. **Where the reader already is** — the file under the diff cursor, or the group they have
   open. A hit in front of them is the one they meant.
2. **Inside a hunk.** The change is what this tool is for; a match in code the branch never
   touched is context, and context comes second.
3. **Plan order.** The projection already holds groups in reading order, so a group's
   position *is* its rank.
4. **Path and line**, so equal hits have an order rather than an arbitrary one.

A group's rank fixes its tier, so a tier key could only ever break a tie between two groups
sharing a rank — which is to say never. The tier is therefore on every row and in none of
this arithmetic. That is the whole of what it is for here: telling the reader what they are
about to open.

**4. Plain text, not symbols.**

A symbol-aware search could rank a declaration above a mention, and the data exists per line
with byte offsets (ADR 0032). It was rejected: a file no reader claims has no symbols at all
(ADR 0023) — lockfiles, manifests, prose, Markdown — so a symbol-ranked search would be
silently blind on exactly the files a plain one handles without thinking about it. Silently
is the word that decides it.

The matcher is a literal with smart case, over `regex`, which is already a workspace
dependency. The crate compiles a literal to a memchr search and reports byte ranges in the
haystack, so a case fold that changes a string's length cannot put a mark in the wrong
column — which a hand-rolled ASCII fold would have had to guarantee by argument.

## Consequences

- **Every changed file is read before the reviewer opens**, in one batched call, rather than
  lazily per group. The rows would have read most of them anyway; what is new is that the
  files nobody opens are read too. It is what lets `/` answer on the keystroke, and it is
  one process rather than a stall the first time the key is pressed.
- **`/` is the one key that does not act on the pane it is pressed in.** `spec/tui.md` opens
  by stating that rule, so the exception is stated there too. A name is a fact about the
  branch, and the reader asking has by definition not found the pane it is in yet.
- **Reaching a line outside every window is capped.** The nearest hunk's gap is pulled open
  towards the line, which crosses nothing — by "nearest", no hunk lies inside that gap. Past
  four thousand lines it stops and says how far the line still is: every revealed line
  becomes a row and a syntect pass, and a generated file can put a match a very long way
  from anything.
- **Nothing lands in the engine.** The scan is a view over a cache the renderer already
  holds, and the ranking reads the projection. A second renderer wanting a search would be
  the reason to move the policy down, and there is not one.
