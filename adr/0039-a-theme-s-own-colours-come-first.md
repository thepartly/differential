# 0039 — A theme's own colours come first

Status: accepted

Amends [ADR 0024](0024-palettes-are-derived-and-threaded.md). The seed, the threading and
the refusal of per-field configuration all stand. What changes is where the colours that
are not accents come from.

## Context

Issue 157: the themes felt off beside the stock ones they are named after. ADR 0024 mixed
every colour but the ground and six accents from those accents, by one set of fractions
for all eleven palettes. The claim was that one derivation keeps the chrome and the code
in one palette.

An audit measured each theme's final colours against its official VS Code port, in Oklab.
The accents were not the problem: on the dark themes the seeds sit within ΔE 0.07 of
stock, and the light themes' deviations are the recorded ones that the legibility tests
forced. Four other things were.

- **The fractions pushed every fill too far from the ground.** Word emphasis, the cursor
  row and the line numbers sat 0.09 to 0.13 lighter than stock on the dark themes, and as
  much darker on the light ones. Word emphasis was `mix(add, bg, 0.58)`; the ports that
  state one sit near 0.80.
- **The cursor row and the selection wore the accent.** No port tints either one; each is
  a grey of the theme's own.
- **Colours the syntax theme already states were never read.** All ten syntax themes with a
  stock counterpart state a selection and a line highlight, seven state line numbers, and
  all colour a comment. `derive()` read the background and the foreground and nothing else.
- **Two themes painted the code in the wrong syntax theme.** One Dark used One Half Dark,
  whose text is `#DCDFE4` against One Dark's `#ABB2BF`. Monokai used `MonokaiExtended`, whose
  ground is a neutral `#222222` against Monokai's `#272822`. two-face ships both originals.

A single derivation cannot fix the third point. The information is in the theme, and a
mix only approximates it.

## Decision

### The syntax theme's own interface colours are taken, when they clear the bar

`derive()` reads four more things from the syntect theme: `selection`, `line_highlight`,
`gutter_foreground`, and the foreground of the `comment` scope. A colour stated with an
alpha is composited over the ground in sRGB, as an editor paints it.

Each one is taken **only if it clears the bar the tests already hold that field to**, and
the derivation is the fallback. So a theme's own value can never be less legible than the
mix it replaces. One Light's own line numbers are 1.4:1 on its ground, which an editor can
live with and a reviewer's gutter cannot. Line numbers must also stay quieter than the
text: Monokai's own are its foreground.

- `selected_bg` is the theme's selection, if it is visibly off the ground.
- `gutter_fg` is the theme's line numbers, then its comment colour, then a mix.
- `noise_fg` is the comment colour, then a mix. It is the quietest thing a theme paints,
  and noise is the quietest tier. Four themes' comments (One Dark, One Light, both
  Solarized) are under the muted bar, so they take the mix.

### The cursor row stays the stronger end of a selection

Every stock theme's line highlight is fainter than its selection, or equal to it. Four of
them (Dracula, Gruvbox Dark, Gruvbox Light, Catppuccin Latte) use one colour for both. In
an editor that costs nothing. Here the cursor's row is the moving end of a line selection,
and a run whose moving end is the quieter one reads backwards (the rule `step_band` already
keeps for changed rows).

So the line highlight is taken only when it is clearly stronger than the selection. That
is true of no shipped theme today. Otherwise the cursor row is the selection, one step
further from the ground in lightness alone, so it stays the theme's own grey. The author
chose this over the editors' order.

### The fractions that remain are fitted to the ports

Where a colour is still mixed, the fraction is the median fitted to the ports rather than
a number chosen by eye. That fit is recorded next to each one. Word emphasis moves to 0.80.
The status bar moves a shade toward black, darker than the ground on a light theme and on a
dark one, as every port that sets it apart does. Where a fitted value fails a legibility
test, the nearest passing one is used, and the comment names the test. The line-number
fallback is the one case: 0.46, not the fitted 0.59.

### The syntax theme is chosen to match the stock ground and text

One Dark paints with `TwoDark`, and Monokai with `MonokaiExtendedOrigin`.

## Consequences

- ADR 0024's line "the other thirty-odd colours are mixed from those" no longer holds.
  Its argument against a hand-tuned palette still does, and this does not bring one back.
  Every colour taken from a theme is still checked against the same bars across all
  palettes. The value is the theme author's rather than ours.
- `a_theme_keeps_its_own_colours_where_they_pass` fails if a theme's own colour passes its
  bar and is not used. Without it, a change to a bar could send every theme quietly back to
  the mixes. `the_cursor_row_is_tellable_from_the_ground_and_from_a_selection` holds the
  cursor-order rule.
- Some fields move away from the VS Code port where the syntax theme and the port
  disagree. One Light's syntect selection is a pale blue and its port's is a grey. The
  syntax theme wins, because it is the palette the code is painted in.
- `Dark` changes too. Base16 Eighties states a selection, and it now gets it. Its cursor
  row is that selection one step further out.
- The search fill (`highlight_bg`, ADR 0034) is untouched. It is a different kind of mark
  from an editor's find-match, and it stays a seed.
