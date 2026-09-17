//! The terminal for one session: entered once, restored on every way out.
//!
//! ratatui's `try_init` and `restore` own raw mode and the alternate screen,
//! and `try_init` chains a panic hook that restores both. Two more modes are
//! ours, because ratatui has no reason to want them: mouse capture, so one
//! wheel notch is one `ScrollDown` rather than the three arrow keys a
//! terminal fakes for an alternate screen (three rows per notch was the
//! complaint; it costs the terminal's own drag-select, which shift-drag or
//! option-drag still gives), and bracketed paste, so a multi-line paste into
//! the composer arrives as one `Event::Paste` instead of driving normal-mode
//! keys one character at a time.
//!
//! The guard turns those two on after ratatui's setup and off before its
//! restore — on the normal exit, on an early `?`, and in a panic hook
//! installed after ratatui's so that it runs first.

use std::io;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

/// The entered terminal. Dropping it restores the terminal if nothing else
/// has, so an early return never leaves the shell in raw mode.
pub struct Guard {
    terminal: DefaultTerminal,
    active: bool,
}

impl Guard {
    pub fn enter() -> anyhow::Result<Self> {
        let terminal = ratatui::try_init()?;
        if let Err(e) = execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste) {
            ratatui::restore();
            return Err(e.into());
        }
        // Chained after ratatui's hook, so it runs before it: our modes off,
        // then raw mode and the alternate screen.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            disable_modes();
            hook(info);
        }));
        Ok(Self {
            terminal,
            active: true,
        })
    }

    pub fn terminal(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Restore on the normal exit path, reporting a failure; `Drop` then has
    /// nothing left to do.
    pub fn restore(&mut self) -> anyhow::Result<()> {
        if self.active {
            self.active = false;
            disable_modes();
            ratatui::try_restore()?;
        }
        Ok(())
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.active {
            self.active = false;
            disable_modes();
            ratatui::restore();
        }
    }
}

/// Best effort, as every teardown is: a terminal that refuses the sequence
/// is not one we can do anything more for.
fn disable_modes() {
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
}
