// Adapted from agavra/tuicr (0dacb6b), src/terminal_state.rs — enter, draw,
// suspend, resume, restore, and the Drop-safe teardown.
//
// The suspend/resume pair was cut when this was vendored, because nothing here
// shelled out to a foreground process. `e` does: it hands the terminal to the
// reader's editor and takes it back (ADR 0038), so the pair is back.
// MIT License — Copyright (c) 2025 tuicr contributors. See LICENSE-MIT.
use std::io::{self, Write};

use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::layout::Rect;
use ratatui::{Frame, Terminal, backend::Backend, backend::CrosstermBackend};

/// Terminal capabilities enabled for one tuicr TUI session.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalFeatures {
    mouse_enabled: bool,
    keyboard_enhancements_supported: bool,
}

impl TerminalFeatures {
    /// Returns an empty feature set for a terminal session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configures whether mouse capture should be active in the TUI.
    pub fn mouse_enabled(mut self, enabled: bool) -> Self {
        self.mouse_enabled = enabled;
        self
    }

    /// Configures whether keyboard enhancement flags should be pushed.
    ///
    /// tuicr only enables these flags after probing support because unsupported
    /// terminals can echo the probe escape sequences into stdout.
    pub fn keyboard_enhancements_supported(mut self, supported: bool) -> Self {
        self.keyboard_enhancements_supported = supported;
        self
    }

    /// Enters TUI terminal mode and returns the owning session.
    pub fn enter<W: Write>(self, writer: W) -> anyhow::Result<TerminalSession<W>> {
        TerminalSession::enter(writer, self)
    }
}

/// Owns terminal mode changes for the active TUI.
///
/// `TerminalSession` is the boundary that keeps raw mode,
/// alternate-screen state,
/// mouse capture,
/// bracketed paste,
/// and keyboard enhancement flags paired with the ratatui terminal.
/// Dropping the session performs a best-effort restore so early returns do not
/// leave the user's terminal in TUI mode.
pub struct TerminalSession<W: Write> {
    terminal: Terminal<CrosstermBackend<W>>,
    features: TerminalFeatures,
    active: bool,
}

impl<W: Write> TerminalSession<W> {
    fn enter(writer: W, features: TerminalFeatures) -> anyhow::Result<Self> {
        let backend = CrosstermBackend::new(writer);
        let mut terminal = Terminal::new(backend)?;
        if let Err(err) = activate_writer(terminal.backend_mut(), features) {
            deactivate_writer_best_effort(terminal.backend_mut(), features.mouse_enabled);
            return Err(err);
        }
        Ok(Self {
            terminal,
            features,
            active: true,
        })
    }

    /// Draws one frame while the TUI terminal session is active.
    pub fn draw<F>(&mut self, render_callback: F) -> std::io::Result<()>
    where
        F: FnOnce(&mut Frame),
    {
        self.terminal.draw(render_callback).map(|_| ())
    }

    /// Hands the terminal to a foreground child: raw mode off, alternate
    /// screen left, capture released.
    ///
    /// The ratatui `Terminal` is kept, so [`resume`](Self::resume) costs no
    /// rebuild. Calling it twice is a no-op, and a session left suspended is
    /// one `Drop` and the panic hook both already handle: `active` is false,
    /// so neither tries to deactivate a terminal that is already back.
    pub fn suspend(&mut self) -> anyhow::Result<()> {
        self.deactivate()
    }

    /// Takes the terminal back after the child exits.
    ///
    /// **Wiping the screen is load-bearing.** ratatui caches the frame it last
    /// drew and writes only the cells that changed. The child wrote over that
    /// screen, so without the wipe the reviewer paints a handful of cells onto
    /// somebody else's output.
    ///
    /// **`Terminal::clear` is the wrong call for it, and it is the obvious
    /// one.** Since ratatui 0.30 that method snapshots the cursor with a
    /// `CSI 6 n` round trip so it can put it back, and a terminal that does
    /// not answer within crossterm's short window returns an error — which,
    /// on the way back from an editor, takes the whole reviewer down. It was
    /// tried, and that is exactly what it did.
    ///
    /// `resize` clears the viewport and resets the back buffer through the
    /// same helper, and asks the terminal nothing. The size is read while we
    /// are at it because the terminal can be resized while the child holds
    /// it — an ioctl, not a round trip.
    pub fn resume(&mut self) -> anyhow::Result<()> {
        if self.active {
            return Ok(());
        }
        activate_writer(self.terminal.backend_mut(), self.features)?;
        self.active = true;
        let size = self.terminal.backend().size()?;
        self.terminal
            .resize(Rect::new(0, 0, size.width, size.height))?;
        Ok(())
    }

    /// Restores the terminal state for the normal exit path.
    ///
    /// After this succeeds,
    /// dropping the session will not attempt a second restore.
    pub fn restore(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.deactivate()?;
        Ok(())
    }

    fn deactivate(&mut self) -> anyhow::Result<()> {
        if !self.active {
            return Ok(());
        }
        deactivate_writer(self.terminal.backend_mut(), self.features.mouse_enabled)?;
        self.active = false;
        Ok(())
    }
}

impl<W: Write> Drop for TerminalSession<W> {
    fn drop(&mut self) {
        if self.active {
            deactivate_writer_best_effort(self.terminal.backend_mut(), self.features.mouse_enabled);
            self.active = false;
        }
    }
}

/// Restores terminal mode on stdout without requiring a live session object.
///
/// This is intended for panic hooks,
/// where ownership may already be unwinding and the session value might not be
/// reachable.
pub fn restore_stdio_best_effort() {
    let mut stdout = io::stdout();
    deactivate_writer_best_effort(&mut stdout, true);
}

fn activate_writer<W: Write>(writer: &mut W, features: TerminalFeatures) -> anyhow::Result<()> {
    enable_raw_mode()?;
    execute!(writer, EnterAlternateScreen)?;
    if features.mouse_enabled {
        execute!(writer, EnableMouseCapture)?;
    }

    // Bracketed paste makes multi-line and control-character pastes arrive as
    // one `Event::Paste` instead of letting each character drive normal-mode
    // actions like Enter submit or command-mode entry.
    execute!(writer, EnableBracketedPaste)?;

    // REPORT_EVENT_TYPES distinguishes Press from Repeat from Release so the
    // two-press file walk can require an actual key release between presses.
    // Without it, terminals emit Press for every auto-repeat tick,
    // and held j/k could walk past file boundaries.
    if features.keyboard_enhancements_supported {
        let _ = execute!(
            writer,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES,
            )
        );
    }
    Ok(())
}

fn deactivate_writer<W: Write>(writer: &mut W, mouse_enabled: bool) -> anyhow::Result<()> {
    let _ = execute!(writer, PopKeyboardEnhancementFlags);
    let _ = execute!(writer, DisableBracketedPaste);
    if mouse_enabled {
        let _ = execute!(writer, DisableMouseCapture);
    }
    disable_raw_mode()?;
    execute!(writer, LeaveAlternateScreen)?;
    Ok(())
}

fn deactivate_writer_best_effort<W: Write>(writer: &mut W, mouse_enabled: bool) {
    let _ = deactivate_writer(writer, mouse_enabled);
}
