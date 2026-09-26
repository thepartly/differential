//! Flexoki, light. The same palette on its warm paper ground, `#FFFCF0`.
//!
//! Bundled like its dark twin (ADR 0040): `flexoki_light.tmTheme`. The accents
//! are Flexoki's 600 tones, the ones it gives a light ground.

use super::{Seed, Syntax, rgb};

pub(super) fn seed() -> Seed {
    Seed {
        syntax: Syntax::Bundled(include_bytes!("flexoki_light.tmTheme")),
        // Flexoki's green 600, `#66800B`, is 4.39:1 on paper, a shade under
        // AA. Darkened for that, and turned 4° towards green: the reviewed
        // mark is this ink 0.10 darker still, and at that lightness Flexoki's
        // own yellow-green hue cannot reach the 0.10 chroma floor in sRGB.
        add: rgb(0x4B, 0x6B, 0x00),
        del: rgb(0xAF, 0x30, 0x29),
        accent: rgb(0x20, 0x5E, 0xA6),
        // Flexoki's yellow 600 is 3.39:1 on paper, well under AA for the
        // skim tier's label. One step darker, hue kept.
        skim: rgb(0x8E, 0x6B, 0x01),
        // Flexoki's 400 yellow: a fill the ground's own ink has to read on,
        // where `skim` is the 600 ink that reads on the ground.
        highlight: rgb(0xD0, 0xA2, 0x15),
        finding: rgb(0x5E, 0x40, 0x9D),
    }
}
