//! `e` — the terminal goes to the reader's editor, and comes back.
//!
//! **"Editor" means something else everywhere else in this crate.** The
//! finding composer is a `tui-textarea` held in `Mode::Editing { editor, .. }`,
//! and it never leaves the screen. This module is about the OTHER one: a
//! program outside this process, which owns the terminal while it runs.
//!
//! Two reasons this is not `engine::subprocess::run`, which every other child
//! in the workspace goes through:
//!
//! 1. **It pipes all three streams.** That is right for an agent, whose bytes
//!    are the answer. An editor's bytes ARE the terminal, and a child that
//!    cannot see the keyboard cannot be typed in.
//! 2. **It carries a deadline and a watchdog.** Right again for a call that
//!    must not hang a pipeline. Here the reader is the deadline: an editor
//!    killed at twenty minutes is an editor that eats an afternoon's note.
//!
//! So this is a plain blocking spawn with inherited stdio, and the whole of
//! what it adds is the pair either side of it. `crates/tui` is an adapter
//! (`crates/engine/tests/layering.rs` says so in as many words), so naming
//! `std::process` here breaks no rule.

use std::path::Path;
use std::process::Command;

use differential_engine::config::EditorCommand;

/// What happened, in the words the footer uses.
///
/// Every arm is a sentence for the reader rather than an error for a log: the
/// reviewer is still running, and the only thing they can do with any of this
/// is read it.
pub enum Outcome {
    /// The editor ran and exited cleanly.
    Edited,
    /// It ran and exited non-zero. The reader is told, and nothing else
    /// changes — an editor that refused to save is not this tool's to judge.
    ExitedWith(String),
    /// It never started. The message is the operating system's, passed
    /// through, on the rule ADR 0029 set for `gh`: the tool the reader
    /// installed reports its own absence better than a guess about it.
    NotStarted(String),
    /// The path is not in the working tree. A review of a committed range can
    /// name a file this checkout does not have, and a deleted file names one
    /// nothing has.
    NotOnDisk,
}

impl Outcome {
    /// The footer line, given the path the reader asked for and whether the
    /// command said where the line goes.
    pub fn message(&self, path: &str, carries_line: bool) -> String {
        match self {
            // Two facts, and the second is the one nobody would guess. The
            // document is a pure function of the range and is never patched
            // (spec/persistence.md), so the diff on screen is the diff that
            // was generated — whatever was just written to the file.
            Outcome::Edited if carries_line => {
                format!("edited {path} · the diff is not reloaded")
            }
            Outcome::Edited => {
                format!("edited {path} · the command has no {{line}} · the diff is not reloaded")
            }
            Outcome::ExitedWith(what) => format!("{what} · the diff is not reloaded"),
            Outcome::NotStarted(why) => why.clone(),
            Outcome::NotOnDisk => format!("{path} is not in the working tree"),
        }
    }
}

/// Hand the terminal over, run the editor, take it back.
///
/// `root` is the checkout's top level; `path` is repo-relative, as every path
/// in the document is. The child runs in `root` so a command spelling a
/// relative path of its own still resolves the way the reader expects.
///
/// The suspend and the resume are paired here and nowhere else, so there is
/// one place where the terminal can be left handed away. Both halves run even
/// when the spawn fails, which is the case a `?` in the middle would get
/// wrong: a reviewer that never came back from an editor that never started
/// is the worst outcome available.
pub fn open(
    terminal: &mut crate::Session,
    root: &Path,
    cmd: &EditorCommand,
    path: &str,
    line: u32,
) -> anyhow::Result<Outcome> {
    let file = root.join(path);
    if !file.exists() {
        return Ok(Outcome::NotOnDisk);
    }
    let argv = cmd.argv(&file, line);

    terminal.suspend()?;
    // Stdio is inherited — the default for `Command` — which is the whole
    // point: the child gets this process's terminal, keyboard included.
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(root)
        .status();
    let resumed = terminal.resume();

    // The resume's failure outranks the child's outcome: a reviewer that
    // cannot draw has nothing to report the outcome ON.
    resumed?;
    Ok(match status {
        Ok(s) if s.success() => Outcome::Edited,
        Ok(s) => Outcome::ExitedWith(match s.code() {
            Some(c) => format!("{} exited {c}", cmd.program()),
            // A signal. `code()` is None then, and "exited none" would say
            // less than nothing.
            None => format!("{} was killed", cmd.program()),
        }),
        Err(e) => Outcome::NotStarted(format!("{}: {e}", cmd.program())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The footer is the only place this feature can be honest about what it
    /// did NOT do, so the two facts it carries are pinned.
    #[test]
    fn the_footer_says_the_diff_did_not_move() {
        let m = Outcome::Edited.message("src/x.rs", true);
        assert_eq!(m, "edited src/x.rs · the diff is not reloaded");

        // A command with no `{line}` opened the file at the top. Saying so is
        // the difference between a feature that looks broken and one the
        // reader can fix in their config.
        let m = Outcome::Edited.message("src/x.rs", false);
        assert!(m.contains("{line}"), "{m}");
        assert!(m.contains("the diff is not reloaded"), "{m}");

        // A path the checkout does not hold names itself, because "not found"
        // without the path is a message the reader cannot act on.
        assert_eq!(
            Outcome::NotOnDisk.message("src/gone.rs", true),
            "src/gone.rs is not in the working tree"
        );

        // The spawn error is the operating system's own and is not dressed up.
        assert_eq!(
            Outcome::NotStarted("nvim: not found".into()).message("src/x.rs", true),
            "nvim: not found"
        );
    }
}
