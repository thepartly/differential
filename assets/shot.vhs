# The README image: the reviewer, mid-review, with two findings on it.
#
#   cd assets && vhs shot.vhs
#
# Run it after `record.vhs` and from the same directory, with the same `dfr` on
# PATH. The gif it writes is a throwaway — vhs demands an `Output`, and this
# tape wants only the `Screenshot`.
#
# Separate from record.vhs for one reason: that tape runs inside zellij so it
# can caption itself, and the README image should not advertise a multiplexer
# `dfr` does not need. So this one opens the reviewer directly.
#
# It does NOT replay the video. It takes the shortest path to the same screen —
# split view, a context gap opened, one note on a line and one over a range —
# because two tapes that have to stay in step beat for beat is a worse trap
# than two tapes that agree on what the picture shows.

Output .shot.gif
Set Shell zsh
Set FontSize 12
Set Height 800
Set Width 1600

# Same reset as record.vhs, and the same reason: the reviewer records its
# cursor, layout and marks per review, so without this the shot is of wherever
# the last run stopped. Ask git where the state lives — in a worktree `.git` is
# a file and the real directory is in the bare parent.
Hide
Type "cd $(git rev-parse --show-toplevel)"
Enter
Type "grep -rl cfea95f $(git rev-parse --git-common-dir)/differential/reviews/*/identity.json 2>/dev/null | xargs -n1 dirname | xargs -r rm -rf"
Enter
Type "cd assets && clear"
Enter
Show

Type "dfr review ecc9400..cfea95f"
Enter

# Wait on a word the reviewer draws and the splash cannot: `reading plan` is in
# the splash too. See the note on the same line in record.vhs.
Wait+Screen@60s /reviewed/
Sleep 1s

# Into the diff of the first group. `enter` rather than `tab`: it says move to
# the diff rather than toggle, so it cannot land back on the plan.
Enter
Sleep 500ms

# `f` here is the diff pane's key: a file list, and `enter` jumps to that
# file's first context boundary — the row that says how many lines are hidden.
# So `z` lands every time, with no counting.
Type "f"
Sleep 600ms
Down@140ms 4
Enter
Sleep 700ms
Type "z"
Sleep 400ms
Type "z"
Sleep 400ms

# `n` jumps to the next hunk and lands on its header, so counting from there
# counts from the change rather than from the context the expansion pulled in.
Type "n"
Sleep 500ms
Down@80ms 6
Sleep 200ms
Type "c"
Sleep 400ms
Type "explain this"
Enter
Sleep 600ms

# And one finding over a range, on the next hunk along. The trailing `\` is the
# continuation marker: it makes the `enter` after it a newline, not a save.
Type "n"
Sleep 500ms
Down@80ms 4
Sleep 300ms
Type "v"
Down@170ms 5
Sleep 300ms
Type "c"
Sleep 500ms
Type "range comment\"
Enter
Type "and multiple lines comments"
Enter
Sleep 900ms

Screenshot screenshot.png
Sleep 400ms
