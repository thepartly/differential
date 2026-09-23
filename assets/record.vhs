# The demo video linked from the README.
#
#   cargo build --release --bin dfr
#   export PATH="$PWD/target/release:$PATH"
#   dfr findings ecc9400..cfea95f
#   cd assets && vhs record.vhs
#   cd assets && vhs shot.vhs      # the README image, separately
#
# Run the last one FROM THE assets DIRECTORY. `Output` and `Screenshot` below
# are relative paths, and vhs resolves them against the process directory, not
# the tape's.
#
# The first two are not optional either. The tape records the `dfr` it finds on
# PATH, and an installed one is whatever was last released — this video is a
# feature demo, so it has to be this tree.
#
# The third warms the grouping cache, and it warms it BEFORE vhs starts. On a
# miss that is a live agent call of a minute or more. Inside the tape it cannot
# be waited on: `Wait+Screen` starts its clock when vhs types, not when the
# shell finishes, so a cold cache expires the wait below rather than delaying
# it. Outside, a forgotten warm is a loud vhs timeout instead of a wrong video.
#
# The README image is `assets/shot.vhs`, not this tape. This one runs inside
# tmux so it can caption itself, and the image should not advertise a
# multiplexer `dfr` does not need. Either order; both reset the session first.
#
# The range is fixed on purpose. This tape used to type a bare `dfr review`
# and walk the commit picker, so every recording was a different diff and no
# two runs could be compared. `ecc9400..cfea95f` is 11 commits and 79 files,
# and its plan has all three tiers in it. Every beat below has real material to
# land on.
#
# The plan itself is the grouping model's answer, so it CHANGES when `dfr`
# does: different labels, a different order, different hunks under the cursor.
# Every `Down N` below is therefore read off the screen rather than derived,
# and a release that moves the plan moves them. The waits are what catch it.
#
# Needs `vim` for the last beat, and `tmux` for the captions — the reviewer
# runs inside one. `assets/demo.tmux.conf` is its whole configuration, and
# the note at the top of it says why a caption has to be a keystroke.

# mp4, and only mp4. GitHub plays it inline the same as a webm, and it is the
# format everything else takes too — a slide, a post, a player that is not a
# browser. A second output of the same frames earned nothing and was one more
# file to upload and keep straight.
Output demo.mp4
Set Shell zsh
Set FontSize 12
Set Height 800
Set Width 1600

# Every run starts from the same screen. The reviewer records its cursor, its
# layout and its reviewed marks per review, so without this the recording
# resumes wherever the last one stopped. Delete ONLY the sessions for this
# range: this is a real repository with real reviews in it, and the blanket
# `rm -rf .../reviews` that assets/themes.sh uses is safe only there, on a
# throwaway fixture.
#
# Ask git where that directory is; do not spell `.git` out. `dfr` resolves it
# with `rev-parse --git-common-dir` (crates/engine/src/gitio.rs), and in a
# worktree `.git` is a FILE — the state lives in the bare parent. A hard-coded
# `.git/differential` matches nothing there, deletes nothing, and the run
# resumes wherever the last one stopped, silently. The `cd` above is what lets
# the relative `.git` a plain clone answers with still resolve.
#
# The grouping cache is a SIBLING of reviews/ and is deliberately kept, so the
# splash below is a cache hit rather than a live agent call. The `dfr findings`
# in the header is what fills it. Do not reach for `dfr clean`: it deletes the
# cache and leaves the sessions, which is the wrong way round.
Hide
Type "cd $(git rev-parse --show-toplevel)"
Enter
Type "grep -rl cfea95f $(git rev-parse --git-common-dir)/differential/reviews/*/identity.json 2>/dev/null | xargs -n1 dirname | xargs -r rm -rf"
Enter
Type "cd assets && clear"
Enter

# The reviewer runs inside tmux so the tape can caption itself: `ctrl-q` is
# bound to a prompt that stores the words typed after it, and the status line
# draws them and nothing else. The reviewer never sees the key or the text.
# See assets/demo.tmux.conf.
#
# Kill the session before starting it, so a second recording starts fresh
# rather than attaching to wherever the last one stopped.
Type "tmux kill-session -t differential 2>/dev/null"
Enter
Type "tmux -f demo.tmux.conf new-session -s differential"
Enter

# The opening caption, set in the config, doubles as the word that says tmux
# has drawn.
Wait+Screen@20s /a reading plan for a diff/
Sleep 800ms
Type "clear"
Enter
Show

Sleep 1.2s

# The picker: `dfr review` with no range offers the last 30 commits as a base.
# Walk it, then leave. `q` cancels the whole command.
Ctrl+q
Sleep 150ms
Type@4ms "dfr review  ·  no range, so it offers a base to pick"
Enter
Sleep 500ms

Type "dfr review"
Enter
Wait+Screen@30s /q cancel/
Sleep 500ms
Down@180ms 5
Up@180ms 2
Sleep 900ms
Type "q"
Sleep 800ms

# Now the real thing, on the fixed range.
Ctrl+q
Sleep 150ms
Type@4ms "or name the range yourself"
Enter
Sleep 500ms

Type "dfr review ecc9400..cfea95f"
Enter

# Wait on what is on screen, never on a fixed delay. The pipeline is a cache
# hit here but still enumerates, and a sleep long enough to be safe on a slow
# machine is a sleep wasted on every other one.
#
# Wait on a word the REVIEWER draws and the splash cannot. This used to read
# `/reading plan/`, which the splash prints too: its grouping stage is called
# "labelling the reading plan", and it says so both before that stage starts
# and after it ticks (crates/tui/src/splash.rs). So the wait returned on the
# first frame and the `Sleep 1s` below was carrying the whole tape into the
# reviewer. `reviewed` is the plan pane's own footer hint (app/help.rs).
Wait+Screen@60s /reviewed/

# A beat after the first paint. `Wait+Screen` returns as soon as the reviewer
# has DRAWN, which is a moment before it is reading keys — a key sent on the
# frame it appeared was dropped.
Sleep 1s

# The dependency graph. There is no key for it: the relation always draws in
# the plan pane, as a gutter connector from the selected group up to each
# group it follows, plus an `after:` line under the group and a `depends on:`
# line in the detail header. So the beat is a cursor move and a pause. Six
# down from the top lands on a group with several `after:` entries, which is
# what makes the connector worth looking at.
Ctrl+q
Sleep 150ms
Type@4ms "the reading plan  ·  what to read first, and what each group follows"
Enter
Sleep 500ms

Down@200ms 6
Sleep 1.7s

# The file tree. `f` in the LEFT pane swaps the reading plan for the tree;
# `f` in the diff pane is a different key entirely, and opens a file list.
# `z` folds the directory under the cursor.
Ctrl+q
Sleep 150ms
Type@4ms "f  ·  every file in the change, as a tree"
Enter
Sleep 500ms

Type "f"
Sleep 900ms
Down@140ms 5
Sleep 400ms
Type "z"
Sleep 900ms
Type "z"
Sleep 600ms
Type "f"
Sleep 800ms

# A word, found anywhere in the change. `/` is the one key that does not act on
# the pane it is pressed in, so the plan pane is as good a place to press it as
# the diff. What it reads is every changed file as it IS — unchanged lines
# included, not the hunks.
#
# Arrows here, never `j`/`k`. Every printable key types into the query, so a
# `j` would search for a `j` (crates/tui/src/app/search.rs).
Ctrl+q
Sleep 150ms
Type@4ms "/  ·  a word, found anywhere in the change"
Enter
Sleep 500ms

Type "/"
Wait+Screen@10s /type to search every changed file/
Sleep 700ms
Type "TreeReport"

# Wait on the count rather than on a sleep. A query nothing holds draws no
# count at all, so a fixture that has drifted stops the recording here instead
# of filming an empty box.
#
# A TYPE, and the row is chosen rather than counted to. `TreeReport` has nine
# hits here — a struct the change adds, and every place that names it. The
# third is `pub tree: Option<TreeReport>`, one token of interest on the line,
# so the peek below has exactly one thing to resolve and the float is the
# struct itself. A row that resolves nothing opens no float at all, which is
# the case the wait after `z` exists to catch.
Wait+Screen@10s / found /
Sleep 1.4s
Down@200ms 2
Sleep 1.2s

# `enter` puts the cursor on the line and opens whatever was in the way — a
# folded skim remainder, or a context gap the pane was not showing. It leaves
# the focus in the DIFF pane, which is what lets the next beat be one key.
Enter
Sleep 1.4s

# What declares the name under the cursor. Every reference the change can
# resolve on this row is underlined already; `z` floats the declaration, with
# its own lines and its own numbers, under a title of
# `name · file:line · class · group`.
#
# This wait is the beat's guard, not its pacing. `z` means four things in the
# diff pane and picks by what the cursor is on; a row with no symbol falls all
# the way through to folding the plan's selected group (app/keys.rs). That is
# invisible from here and would run every later beat on the wrong plan. No
# float, no recording.
#
# One press, not two. `z` steps a line's names in column order and closes past
# the last one rather than wrapping, and this line has one it can resolve.
#
# The regex matches the float's TITLE and not a symbol name: which name wins is
# a fact about the line rather than about this tape. `· <path>:<line>` is drawn
# nowhere else on this screen — the search box has closed, and no finding is
# written yet.
Ctrl+q
Sleep 150ms
Type@4ms "z  ·  what declares the name under the cursor"
Enter
Sleep 500ms

Type "z"
Wait+Screen@10s /· .*\.rs:[0-9]+/
Sleep 2s

# `esc` closes it. So does moving the cursor, which is why the float is one
# field and not a mode.
Escape
Sleep 600ms

# Back to the plan pane: the peek left the focus in the diff, and the beat
# below walks the PLAN.
Tab
Sleep 400ms

# Back up the plan to the first group, then into the diff. Walk it with `k`
# rather than `g`: `g` is the diff pane's top, and in the plan pane it leaves
# the selection where it is — the whole second half of an earlier take ran on
# a lockfile because of that. Extra presses clamp at the top, so 16 is safe
# for a 15-group plan.
#
# `enter` rather than `tab` to cross into the diff: it says move to the diff
# rather than toggle, so it cannot land back on the plan.
Ctrl+q
Sleep 150ms
Type@4ms "back to the top of the plan, and into the diff"
Enter
Sleep 500ms

Up@90ms 16
Sleep 800ms
Enter
Sleep 600ms
Down@70ms 12
Sleep 500ms

# Split and unified. Split is what a review opens in.
Ctrl+q
Sleep 150ms
Type@4ms "s  ·  side by side, or unified"
Enter
Sleep 500ms

Type "s"
Sleep 1.5s
Type "s"
Sleep 1.2s

# `f` here is the diff pane's key, and a different one from `f` in the plan
# pane: it lists the files this group touches, and `enter` jumps to one. The
# beats below are written against that file, so this is navigation as much as
# it is a feature of its own.
Ctrl+q
Sleep 150ms
Type@4ms "f  ·  jump to any file this group touches"
Enter
Sleep 500ms

Type "f"
Sleep 900ms
Down@140ms 4
Sleep 400ms
Enter
Sleep 1.2s

# A finding on one line, and `n` is how the cursor gets to one. It jumps to the
# next hunk in this view and lands on its HEADER, skipping any hunk crossed in
# from another group. Walking down from where `enter` left the cursor instead
# lands on whatever context the file opens with — which is how an earlier take
# filed both of its findings against a doc comment and a `use` line.
#
# One row past the header is the change itself in this hunk. How many rows it
# takes depends on the leading context the hunk carries, so the count is read
# off the screen — see the note about the plan at the top of this file.
Ctrl+q
Sleep 150ms
Type@4ms "n jumps to the next hunk  ·  c writes a finding on the line"
Enter
Sleep 500ms

Type "n"
Sleep 700ms
Down@80ms 1
Sleep 300ms
Type "c"
Sleep 500ms
Type "explain this"
Enter
Sleep 900ms

# A finding over a range, on the next hunk along. `v` starts the selection,
# `j`/`k` extend it, `c` writes over it. The trailing `\` is the continuation
# marker: it makes the `enter` after it a newline instead of a save.
Ctrl+q
Sleep 150ms
Type@4ms "v selects  ·  c writes one finding over the whole range"
Enter
Sleep 500ms

Type "n"
Sleep 700ms
Down@80ms 1
Sleep 400ms
Type "v"
Down@170ms 2
Sleep 500ms
Type "c"
Sleep 700ms
Type "range comment\"
Enter
Type "and multiple lines comments"
Enter
Sleep 800ms
Sleep 1.2s

# Mark a group reviewed, then move on.
Ctrl+q
Sleep 150ms
Type@4ms "space  ·  mark the whole group reviewed"
Enter
Sleep 500ms

Tab
Sleep 500ms
Type " "
Sleep 800ms
Down@180ms 2
Sleep 600ms

# Leave for another file entirely, so the jump back has somewhere to come
# from.
Tab
Type "f"
Sleep 700ms
Down@180ms 6
Enter
Sleep 1.2s

# Every finding in one list, wherever it lives. `enter` jumps to one — across
# files, across groups, opening a folded remainder if it has to.
Ctrl+q
Sleep 150ms
Type@4ms "F  ·  every finding, wherever in the branch it lives"
Enter
Sleep 500ms

Type "F"
Sleep 1.2s
Down@200ms 1
Sleep 600ms
Enter
Sleep 1.8s

# Copy the findings. One key, and it copies the whole open-findings summary as
# markdown — file, lines and note per bullet.
Ctrl+q
Sleep 150ms
Type@4ms "y  ·  copy them all as markdown"
Enter
Sleep 500ms

Type "y"
Sleep 1.3s

# Clean up: clear the findings before quitting. `D` asks, and only a bare `y`
# confirms. The list closes itself once nothing is left in it.
Type "F"
Sleep 900ms
Type "D"
Sleep 1.1s
Type "y"
Sleep 900ms
Type "q"
Sleep 700ms

# What the summary is for. Open insert mode BEFORE pasting: the summary's
# first line starts with `- `, and in normal mode vim reads `- c r a` as
# commands and swallows them — an earlier take pasted a first line missing
# its first five characters.
Ctrl+q
Sleep 150ms
Type@4ms "and paste them to your agent"
Enter
Sleep 500ms

Type "vim"
Enter
Sleep 700ms
Escape
Type "i"
Paste
Sleep 500ms
Enter@150ms 2
Sleep 500ms
Type "paste to your agent!"
Sleep 1.8s
Escape
Type ":q!"
Enter
Sleep 600ms
