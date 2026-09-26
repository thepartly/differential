//! Monokai. Near-black ground, and the brightest accents of the set.
//!
//! The high-contrast end of the range — its foreground sits 15:1 from its
//! ground, so the adaptive muting runs at full strength here and the quiet
//! inks are genuinely quiet rather than merely dimmed.
//!
//! Over `MonokaiExtendedOrigin`, which keeps Monokai's own olive-black
//! `#272822`. Plain `MonokaiExtended` repaints it a neutral `#222222`, and the
//! warmth of that ground is most of what makes Monokai look like Monokai
//! (issue 157).

use two_face::theme::EmbeddedThemeName;

use super::{Seed, rgb};

pub(super) fn seed() -> Seed {
    Seed {
        syntax: EmbeddedThemeName::MonokaiExtendedOrigin,
        add: rgb(0xA6, 0xE2, 0x2E),
        del: rgb(0xF9, 0x5C, 0x8E),
        accent: rgb(0x66, 0xD9, 0xEF),
        skim: rgb(0xE6, 0xDB, 0x74),
        // Monokai's yellow, warmed off the yellow-green its own `skim` sits on.
        highlight: rgb(0xFF, 0xD8, 0x66),
        finding: rgb(0xAE, 0x81, 0xFF),
    }
}
