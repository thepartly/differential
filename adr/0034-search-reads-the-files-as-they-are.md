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

**A row says `g1 skim C0`** — the group, its tier, and the shape class of the hunk holding
the line. The class rather than the words "in hunk", which the class already implies and
which says less: four hits in four files sharing one class are one shape, and the plan may
well have deferred three of them for exactly that reason. It is the answer to *have I read
this already*, which is the question a reader scanning the list is really asking. A line
inside no hunk has no class, so the absence carries what "in hunk" used to say.

**The list takes a third of the box, the preview the rest.** The two are not worth the same
per row: a list row is a path and a badge, and eight of them is already more than a reader
compares at once, where the preview is code and every line of it is a line they may not have
to go and open at all.

**4. Literal by default, a pattern on `ctrl-r`.**

A toggle rather than a mode the reader chooses up front, and rather than a second key that
opens a second box. The word someone typed as a literal is usually most of the pattern they
now want, so the toggle keeps it and re-reads it. The query row carries which it is — `/word`
or `/~word` — because a mode the reader has to remember is a mode they will be wrong about.

A pattern that does not compile finds nothing, which is indistinguishable from a word nothing
holds unless the box says so. It says `bad pattern`. The distinction is the difference between
a typo the reader can fix and an answer they should believe.

Smart case holds in both readings. In a pattern the metacharacters are the reader's business
and the case rule is not one of them.

**5. A hit is a filled block in an accent of the palette's own.**

The symbol float marks with an underline and explicitly rejected a background, because a
background in the diff pane would have to be told apart from `added_bg`, `deleted_bg` and
their word-level twins by `Theme::step_band`, which dispatches on colour VALUES. That
reasoning is about the diff pane. Nothing in the search box goes through it, so the box can
have the fill the pane could not.

It should have one. The two marks say different things: the symbol underline marks a name on
a row the reader is ALREADY reading and has to stay out of the way, where a search hit is the
thing they went looking for, on a line they have not read yet, and being out of the way is
the one thing it must not be.

The colour is a **sixth seed accent**, `highlight` — each palette's own yellow, at fill
strength — with whichever of the theme's extremes reads on it reversed out. Deriving it from
`skim`, which is the same hue in every palette, was the obvious saving and is wrong: `skim`
is an INK that has to read on the ground, which on a light theme makes it a dark brown, where
this is a FILL that the ground's ink has to read on, which on the same theme makes it a
bright amber. One value cannot be both. That is what separates this from `reviewed_fg`, the
field ADR 0024 removed — that one's eleven values were identical to another field's.

The palette test holds it to three things: the ink clears WCAG AA on the fill, the fill is
not one of the theme's other five accents, and its chroma clears the module's own 0.10 floor.
**Chroma, not luminance**, and the light themes are why: a ground that is near-white cannot
be 3:1 from any yellow, so a luminance bar would have forced a dark brown — which is `skim`
again, and not a highlight at all. Every editor's yellow-on-white search mark is low-contrast
and perfectly visible, because a saturated hue on a neutral ground is not a lightness
difference. The module had already recorded this lesson once, about an invisible green.

`highlight` is a seed rather than a derived value so that anything else meaning *this is what
you asked for* can be derived from it later.

**6. The reading is a pill, and its key is on the footer.**

`ctrl-r` is a footer act rather than a quiet one, and it is the only key in this box that has
to be: `?` types here, so the help modal cannot be opened from it and the footer is the only
place the key is written down anywhere the reader can reach. Its label says what the press
WILL do — `pattern` while reading a literal — which is the footer's own rule. `presses_for`
learned the `ctrl-<c>` chords so that clicking the button presses the key it names; a footer
button that reads and does nothing would have been the wrong half of the convention.

The reading itself is on the query row twice: as the lead (`/word`, `/~word`) and as a
`pattern` pill. It is a fact about what the next keystroke will do, which is what a pill says
here — as `selecting 4 lines` does on the window footer, and as a group's role and a hunk's
class do in the panes. It appears while the reading is on and goes when it goes, so there is
no pill meaning "literal": the absence is the statement, and the default needs no badge.

**7. A query that comes back is selected.**

The query and the hit survive a close, so walking a word's occurrences is `/` and an arrow.
That same behaviour is in the way of a reader looking for something else, who now has to
clear a field before they can type.

Selecting it serves both without a second key, and without either reader having to know which
state they are in — the row is drawn on the band a selected list row wears, so they can see
it. The next character typed replaces the whole query, backspace takes all of it, and
anything that is not typing drops the selection and leaves the word alone. That is what a
text field does everywhere else, which is the point.

**8. Plain text, not symbols.**

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
- **A palette gains a field**, and a theme that picks a bad one fails in the palette test
  rather than in somebody's terminal.
- **Nothing lands in the engine.** The scan is a view over a cache the renderer already
  holds, and the ranking reads the projection. A second renderer wanting a search would be
  the reason to move the policy down, and there is not one.
