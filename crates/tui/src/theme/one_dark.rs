//! One Dark. The Atom-descended palette, over `TwoDark`, the syntax theme
//! that paints it on its own `#282C34` in its own `#ABB2BF`.
//!
//! It was One Half Dark, which shares the ground and not the text: `#DCDFE4`,
//! ΔE 0.14 brighter than One Dark's, on every line of code (issue 157).
//!
//! Its published red sits at 4.4:1 on this ground and its yellow at chroma
//! 0.097 — both a shade under the bars, so both are nudged. Everything else is
//! One's own.

use two_face::theme::EmbeddedThemeName;

use super::{Seed, Syntax, rgb};

pub(super) fn seed() -> Seed {
    Seed {
        syntax: Syntax::Embedded(EmbeddedThemeName::TwoDark),
        add: rgb(0x98, 0xC3, 0x79),
        del: rgb(0xE8, 0x79, 0x7F),
        accent: rgb(0x61, 0xAF, 0xEF),
        skim: rgb(0xE8, 0xBC, 0x5F),
        // Brighter and purer than One's warning gold, which `skim` already wears.
        highlight: rgb(0xFF, 0xD2, 0x4A),
        finding: rgb(0xC6, 0x78, 0xDD),
    }
}
