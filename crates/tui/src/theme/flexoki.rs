//! Flexoki, dark. Steph Ango's ink-on-paper palette, on its near-black ground.
//!
//! two-face does not ship Flexoki, so its syntax theme is bundled
//! (ADR 0040): `flexoki.tmTheme`, written here from Flexoki's published
//! palette (MIT). The accents are Flexoki's 400 tones, the ones it gives a
//! dark ground.

use super::{Seed, Syntax, rgb};

pub(super) fn seed() -> Seed {
    Seed {
        syntax: Syntax::Bundled(include_bytes!("flexoki.tmTheme")),
        add: rgb(0x87, 0x9A, 0x39),
        // Flexoki's red 400 is 4.42:1 on this ground, a shade under AA for
        // text that names a deletion. One step lighter, hue kept.
        del: rgb(0xE8, 0x70, 0x5F),
        // Flexoki's blue 400, `#4385BE`, muted for a foreign hunk falls to
        // 2.81:1. Lifted in lightness with its hue and chroma kept (0.105).
        accent: rgb(0x60, 0x9E, 0xD6),
        skim: rgb(0xD0, 0xA2, 0x15),
        // Flexoki's orange, since `skim` wears its yellow and the fill has to
        // be a colour of its own.
        highlight: rgb(0xDA, 0x70, 0x2C),
        finding: rgb(0x8B, 0x7E, 0xC8),
    }
}
