//! The model port: one-shot completion, prompt in, raw text out (ADR 0016; an
//! engine module since ADR 0018).
//!
//! This file is the trait and its error, and nothing that runs anything. The
//! one implementation — an agent CLI spawned with an argv per agent — is
//! `llmio`, the adapter, on the pattern of `forge`/`forgeio`. The split is what
//! lets the layering test check this file as domain: grouping and the pipeline
//! consume only `LlmBackend` from here, and the subprocess machinery stays
//! behind the adapter's door.
//!
//! The grouping stage needs exactly one capability from a model: one-shot text
//! completion — prompt in, raw text out. The contract is deliberately that
//! narrow: no streaming, no chat state, no conversation to manage.
//!
//! It stays that narrow now that the model reads for itself (ADR 0022). Tools
//! run inside the CLI the adapter spawns, so what crosses this seam is still a
//! prompt and a string. What changed is a flag in the adapter's argv, not the
//! trait.

use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{command} exited with {code:?}: {stderr}")]
    Failed {
        command: String,
        code: Option<i32>,
        stderr: String,
    },

    #[error("{command} produced no output")]
    Empty { command: String },

    #[error("{command} exceeded the {timeout:?} deadline and was killed")]
    Timeout { command: String, timeout: Duration },

    #[error("{command} was cancelled and killed")]
    Cancelled { command: String },

    #[error("io error talking to {command}: {source}")]
    Io {
        command: String,
        #[source]
        source: std::io::Error,
    },
}

/// One-shot completion: prompt in, raw text out.
pub trait LlmBackend: Send + Sync {
    /// What to call this agent on screen, for a reviewer waiting on it.
    ///
    /// A product name, not a command line: "Claude Code", not `claude -p
    /// --output-format text --allowed-tools Bash(...),...`. The reviewer is
    /// waiting to learn *which agent* is thinking, and the argv answers a
    /// different question at four times the width — it overran the splash line
    /// the moment the allowlist grew.
    ///
    /// The command as it will actually run is still reported where it is the
    /// answer: `LlmError` carries it, because a spawn failure is debugged with
    /// the whole argv and nothing less.
    fn name(&self) -> &str;

    /// Everything about this backend that could change the grouping, and
    /// nothing that could not. The grouping cache key hashes this (ADR 0009).
    ///
    /// Separate from `name` because the two answer different questions. `name`
    /// is what to show a reviewer, so it is a product name. This is what
    /// determines the answer, so it is the argv — minus the parts that say
    /// where this machine keeps things. Hashing a path put the absolute
    /// location of `dfr` into the key, so a debug build, a release build and
    /// two checkouts of one commit each re-ran a four-hundred-second call for
    /// an identical class partition, and the worktree-shared cache
    /// `plan::grouping_cache_dir` promises was defeated.
    ///
    /// Defaults to `name`, which is right for any backend whose identity has no
    /// environment in it.
    fn identity(&self) -> &str {
        self.name()
    }

    fn complete(&self, prompt: &str) -> Result<String, LlmError>;
}
