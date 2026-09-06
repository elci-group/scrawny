use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "scrawny", version, about = "Detect and explain review-hostile changes.")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Output format: text, json, jsonl
    #[arg(long, global = true, default_value = "text")]
    pub format: String,

    /// Repository path to analyse
    #[arg(long, global = true, default_value = ".")]
    pub path: PathBuf,
}

#[derive(Subcommand)]
pub enum Command {
    /// Analyse current changes (default when no subcommand is given).
    Analyse {
        /// Analyse only staged (index) changes instead of all uncommitted changes.
        #[arg(long)]
        staged: bool,
    },
    /// Explain why a change is considered review-hostile, with evidence.
    Explain {
        #[arg(long)]
        staged: bool,
    },
    /// Generate a decomposition proposal (ranked clusters, no mutation).
    Suggest {
        #[arg(long)]
        staged: bool,
    },
    /// Construct and validate a real series of commits from the detected
    /// clusters, entirely inside a disposable temporary worktree. Never
    /// touches the current branch or checkout; nothing is merged, reset, or
    /// pushed. `--apply`-style automation onto the real branch is
    /// deliberately not implemented (Phase V territory).
    Split {
        #[arg(long)]
        staged: bool,
        /// Remove the temporary worktree after presenting the plan. The
        /// branch it built (and its commits) are kept either way — this
        /// only controls whether the scratch working directory sticks
        /// around for inspection.
        #[arg(long)]
        discard_worktree: bool,
    },
    /// CI-compatible pass/fail check against policy thresholds.
    Check {
        #[arg(long)]
        staged: bool,
        /// Check a supplied diff (e.g. a ref-range comparison) instead of the
        /// working repository. `--path` still selects which `.scrawny.toml`
        /// policy applies.
        #[arg(long)]
        stdin: bool,
        /// Read the diff from a file instead of stdin.
        file: Option<PathBuf>,
    },
    /// Analyse a supplied diff instead of the working repository.
    Diff {
        /// Read the diff from stdin.
        #[arg(long)]
        stdin: bool,
        /// Read the diff from a file.
        file: Option<PathBuf>,
    },
    /// Clone the latest scrawny source and install it with `baby --user`.
    Update,
}
