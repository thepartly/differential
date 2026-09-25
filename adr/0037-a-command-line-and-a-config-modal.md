# 0037 — A command line, and a config modal that saves the file whole

Status: accepted

Follows [ADR 0036](0036-keys-are-bound-by-action.md), whose `[keys]` table the modal edits.
It amends [ADR 0012](0012-config-never-excludes.md), under which the user file was only
ever read: the reviewer now writes it. Issue #154.

## Context

Every personal setting lives in `~/.config/differential/config.toml`, and until now the
only way to change one was to edit the file by hand and reopen the reviewer. Themes and
keys are the settings most worth trying out, and they are the hardest to judge in a text
editor.

The author asked for two things. A config modal that edits and saves every personal
setting, previewing as it goes. And a vim-style `:` line to open it, with the same line
reaching every other modal that can be opened from anywhere in the review.

## Decision

**`:` opens a command line on the status row**, from all three screens. **`:` is fixed,
as `ctrl-c` is, and reserved: no `[keys]` action may take it.** The command line is how
`:config` is reached, and `:config` is where a broken table gets repaired, so the key to it
cannot be one the table moves. The commands are:

- `config`, `help`, `findings`, `search [text]`, `files`, `publish`, `refetch` and `copy`
- `quit`, with the alias `q`

While a name is being typed, a popup lists the commands it could still become, each with
what it does, and the rest of the first is drawn dim after the caret. `tab` and the arrows
walk that list; completion is shown, never guessed.

Each one calls the same function its key calls, so it opens the same thing and refuses in
the same words. One table (`app::command::COMMANDS`) feeds the dispatch, `tab` completion
and the commands row in `?`. The line's own keys are fixed, as the search box's are: every
printable key types into it.

**`:config` opens a modal over a DRAFT of the user file.**

1. **The modal's keys are fixed, not bound.** The modal is where a broken `[keys]` table
   gets repaired, so it cannot answer to the table it is repairing.
2. **Every multiple-choice row is chosen from a list** — the agent, the theme and the
   diff layout. The list sets the row as the selection moves, so a theme is worn as it is
   passed.
   The popup and this list are ratatui's `List` + `ListState`, not hand-rolled; the older
   lists are to follow (#167).
3. **Theme, layout and context preview live, and `esc` puts them back.** One function,
   `show_review`, applies the draft, the original and the saved config alike, so the three
   cannot disagree. The layout is a default: a review that recorded its own with `s` keeps
   it, and the modal says so on that row.
4. **Keys and the agent are not previewed.** A key that moved while the reader was still
   choosing it would be a trap. The agent has already run, so its settings apply from the
   next grouping run, and the save's message says so.
5. **A key row is edited in the file's own syntax**, `["ctrl-j", "d d"]`
   (`KeysConfig::render_list` / `parse_list`), so nothing typed in the modal means something
   else in the file. The draft goes through `Keymap::new` on every change, and a save is
   refused while it reports a problem.
6. **Saving rewrites the whole file.** This was the author's choice over editing it in
   place. It keeps the writer to one `toml::to_string_pretty` over the same types the reader
   parses, and it is the one form that cannot disagree with what the modal shows. **The
   cost is the file's comments and layout.** The modal's headline names the file it writes;
   the cost is written down here and in `docs/cli.md`. `Config::save_user` parses its own text back before writing, so the file on disk is
   always one the reader accepts, and accepts as the value that was saved.
7. **The write is a port method**, `ConfigSource::save`, and `OsConfigSource` implements
   it: it creates the directory and writes atomically through `store::write_atomic`. The
   domain names no filesystem (ADR 0020).

## What it costs

- **A hand-kept file loses its comments** the first time the modal saves it. Anyone who
  wants to keep them edits the file by hand, which still works exactly as before.
- **Every `[review]` value is written out, defaults included.** A later change to a
  default does not reach a file the modal saved. `[grouping]` values are written only when
  set, and `[keys]` only for actions that are rebound.
- **The picker and the splash still read the theme once, at start.** Only the review
  previews.
