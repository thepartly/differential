//! Monokai, in Monokai Pro's palette. A warm grey ground with a faint violet
//! cast, and a softer set of accents than classic Monokai's.
//!
//! Classic Monokai's olive `#272822` read as green once the chrome was taken
//! from the ground it sits on (ADR 0039), so the name now wears the palette's
//! successor. two-face ships only classic Monokai, so this is the one palette
//! whose syntax theme is bundled rather than embedded (ADR 0040):
//! `monokai.tmTheme`, written here from Monokai Pro's published palette
//! values. Not affiliated with or endorsed by Monokai Pro.
//!
//! Its comments are quiet by design — `#727072` sits under 3:1 on this ground
//! — so the noise tier takes the derived grey rather than the comment colour.

use super::{Seed, Syntax, rgb};

pub(super) fn seed() -> Seed {
    Seed {
        syntax: Syntax::Bundled(include_bytes!("monokai.tmTheme")),
        add: rgb(0xA9, 0xDC, 0x76),
        del: rgb(0xFF, 0x61, 0x88),
        // The palette's blue is `#78DCE8`, at chroma 0.095: pale enough to
        // read as off-white on the one cell that has to say "here", which is
        // Dracula's cyan problem again. Lifted to 0.102, hue kept.
        accent: rgb(0x6C, 0xD9, 0xE8),
        skim: rgb(0xFF, 0xD8, 0x66),
        // The palette's orange, since `skim` already wears its yellow and the
        // fill has to be a colour of its own.
        highlight: rgb(0xFC, 0x98, 0x67),
        finding: rgb(0xAB, 0x9D, 0xF2),
    }
}
