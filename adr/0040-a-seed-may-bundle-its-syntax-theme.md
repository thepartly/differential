# 0040 — A seed may bundle its syntax theme

Status: accepted

Follows [ADR 0024](0024-palettes-are-derived-and-threaded.md), which made a theme a seed that
names a syntax theme, and [ADR 0039](0039-a-theme-s-own-colours-come-first.md), which made
that syntax theme the source of more of the palette. It replaces 0039's choice of
`MonokaiExtendedOrigin` for `monokai`.

## Context

A seed named its syntax theme as a two-face `EmbeddedThemeName`, so a palette could only be
one of the thirty-odd themes two-face ships.

ADR 0039 matched classic Monokai to its stock olive ground, `#272822`. Every grey that is
mixed from the ground then took the olive too: the selection, the cursor row, the pills and
the status bar. Monokai's yellow-green addition tint sits close to that hue, so a screen
full of additions read as green. That is what classic Monokai looks like, and the author
did not want it. They chose Monokai Pro, the palette's warm-grey successor, and asked for
it to replace classic Monokai under the same name.

two-face ships four classic Monokai variants and no Monokai Pro. The code's colours come
from the syntax theme, and after ADR 0039 so do the selection, the line numbers and the
comment colour. So Monokai Pro's accents on a classic Monokai syntax theme would not be
Monokai Pro. It needs a syntax theme of its own.

## Decision

### A bundled syntax theme

`Seed::syntax` is a `Syntax`. It is either `Embedded`, a two-face name as before, or
`Bundled`, the bytes of a plist `.tmTheme` in `crates/tui/src/theme/`. The file is compiled
in with `include_bytes!` and parsed by syntect's own `ThemeSet::load_from_reader`. This is a
two-variant enum with a concrete second case, not a plug-in point. Nothing is loaded from
disk or the network at run time. So a theme is still a name that always works, and
`[review].theme` is still a name rather than a path (ADR 0024, "Named, not configurable
field by field").

A bundled file that fails to parse is a bug in this crate, so it panics.
`every_named_theme_builds` runs every seed, and that is where such a bug fails.

### `monokai` is Monokai Pro

The name stays `monokai`, so every existing config keeps working and no new name is added.
Classic Monokai is no longer shipped.

### The file is written for this crate, from the palette alone

Monokai Pro is a paid product, and its theme files are licensed with it. Its colour values
are published facts, and a palette is not a work. So `monokai.tmTheme` takes the palette and
maps scopes to it in this crate's own way. No line of an official or ported theme file is
copied. The seed file says the palette is Monokai Pro's and that this crate is not affiliated
with it. The same rule applies to any future bundled theme: take the colours, write the file.

## Consequences

- A reader on `monokai` sees a different palette after upgrading: a warm grey ground in
  place of olive, and softer accents. The release notes carry it as a `tui` feature.
- Adding a palette that two-face does not ship is a variant, a seed file and a `.tmTheme`.
- A bundled theme's code colours are our mapping, not the original's. The right colours
  land on the right kinds of token, but not always on the same tokens the official theme
  picks.
- The legibility, chroma, distinctness and own-colour tests run over a bundled theme exactly
  as over an embedded one. Monokai Pro's blue needed a lift for the chroma floor, like
  Dracula's cyan. Its comment colour is under the muted bar, so noise takes the mix.
