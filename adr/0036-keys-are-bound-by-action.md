# 0036 — Keys are bound by action

Status: accepted

Extends [ADR 0012](0012-config-never-excludes.md), whose user file this adds a table to, and
follows [ADR 0024](0024-palettes-are-derived-and-threaded.md) in threading a value built
once rather than reading config from the renderer. Issue #153.

## Context

Every key the reviewer answered to was a `match` arm in `crates/tui/src/app/keys.rs`. The
help modal and the footer read a second table (`help.rs`) that held the same keys as
display strings — `"J/K  { }"` — and parsed them back into presses for the footer's
buttons. Nothing tied the two together. Rebinding a key would have meant changing both, and
a reader's config could change neither.

Readers asked for their own keys. The request is small. What makes it a decision is the set
of questions it opens: what a binding names, what happens to a default once a key is bound,
what one key on two actions does, and where the check lives.

## Decision

**A binding names an ACTION, and a key string names the key.** `config::Action` is the
vocabulary (`down`, `next-group`, `delete`, …). `[keys]` in the user file maps each action
to a list of key strings:

```toml
[keys]
next-group = ["ctrl-j"]
delete = ["d d"]
publish = []
```

Seven things follow.

1. **Binding replaces; it never extends.** An action named in `[keys]` takes exactly the
   keys given, and its defaults are dropped. `[]` unbinds it. Extending would leave no way to
   free a default key for something else.
2. **A clash is an error that names both actions, never a quiet winner.** Two actions on
   one key in one screen fails the load, and so does a key that starts another action's
   sequence (`d` against `d d`), since the longer one could never be pressed. A reader who
   bound `j` to something and still sees it move has been told nothing. Every problem is
   reported at once, not the first.
3. **One flat namespace.** `down` moves in the plan pane, the file list and the findings
   list alike, so a reader binds it once. An action works in the screens its default row
   names (`keymap::DEFAULTS`). A rebinding holds in all of them.
4. **Clashes are checked per screen, and a screen is coarser than a pane.** There are three:
   the review, the file list and the findings list. Within the review a key means one action
   whichever pane has focus, and the action decides what it does there. So `l` is taken in
   the plan pane too, although it only moves the diff. A key that meant one thing on one row
   and another on the next is a key nobody could rebind with confidence.
5. **Fixed keys stay fixed.** The composer's and the search box's editing keys belong to
   their text widgets. A `y` question's `y` is deliberately the one answer. `ctrl-c` quits
   from everywhere and no action may take it: it is the way out a lost reader can always
   find.
6. **The check is a library call.** `differential_tui::Keymap::new(&KeysConfig)` touches no
   terminal and no file, and returns the keymap or every problem. `dfr review` calls it
   before the terminal opens, so a bad table is a usage error (exit 2) naming the file, not a
   reviewer that half works. Anything else that wants to validate a table calls the same
   function.
7. **One table serves the dispatch, the footer, `?` and every message that names a key.**
   The handler looks a key up once and matches on the action. The help rows name actions,
   and their keys come from the keymap, so a footer button presses exactly the key it shows.
   The messages that say "`w` turns it off" and the rows that say "`z` to show" name the
   bound key, and leave out a key the reader unbound.

## Where each half lives

**The engine holds the vocabulary; the renderer holds the keys.** `Action` and `KeysConfig`
are in `engine::config`, beside `ThemeName`, because an unknown action is a config error
and serde reports it with every valid name, the same way an unknown theme is reported.
Key strings stay strings there. Parsing one, the defaults, and the clash rules need
crossterm's idea of a key, and the engine does not learn a terminal's vocabulary.

Key strings are parsed by `crokey` (design rule 5), with two adjustments `keymap.rs` states
where it makes them. crokey lowercases its input, so `"J"` would read as `j`; an uppercase
letter is said as `shift-` first. Every character's shift is also dropped, so `J` and
`shift-j`, and `?` as each terminal reports it, compare equal. A sequence is written with
spaces (`"d d"`) and displayed as vim writes it (`dd`).

## What it costs

- **A modifier nobody bound no longer reaches a bare key.** The old arms matched `q`, `y`,
  `F` and others with any modifier, so `ctrl-q` quit. Now a key is one exact combination.
  Shift on a character is the exception, handled by the canonical form above.
- **The help's key column is as wide as its widest key**, and the box grows by the same
  amount. With the defaults it grew because every binding is listed: `j/k · down/up`,
  `alt-=/alt-- · alt-+`.
- The picker and the splash keep fixed keys. They are outside the review and have three
  keys each. Bringing them in is a screen and a row each, when someone asks.
