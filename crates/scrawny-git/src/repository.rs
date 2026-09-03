use scrawny_core::{Diff, Result, ScrawnyError};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::parse::parse_unified_diff;

/// What slice of repository history/working state to diff.
#[derive(Debug, Clone)]
pub enum DiffSpec {
    /// Unstaged changes in the working tree against the index.
    WorkingTree,
    /// Staged changes (the index) against HEAD.
    Staged,
    /// All changes in the working tree and index against HEAD (staged + unstaged).
    HeadToWorkingTree,
    /// A single commit against its parent.
    Commit(String),
    /// One ref/commit against another, e.g. `main..feature`.
    Range { from: String, to: String },
}

#[derive(Debug, Clone)]
pub enum CommitSpec {
    /// The last `n` commits reachable from HEAD.
    Last(usize),
    /// Commits in `from..to`.
    Range { from: String, to: String },
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub id: String,
    pub summary: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub staged: Vec<PathBuf>,
    pub unstaged: Vec<PathBuf>,
    pub untracked: Vec<PathBuf>,
}

/// Stable abstraction over Git so the analysis engine never shells out
/// directly. A `git2`-backed implementation can replace `GitCliRepository`
/// later without touching any caller of this trait.
pub trait Repository {
    fn status(&self) -> Result<Status>;
    fn diff(&self, spec: DiffSpec) -> Result<Diff>;
    fn commits(&self, spec: CommitSpec) -> Result<Vec<Commit>>;
}

pub struct GitCliRepository {
    root: PathBuf,
}

impl GitCliRepository {
    /// Discover the repository containing `start` by asking git for its
    /// top-level working directory.
    pub fn discover(start: &Path) -> Result<Self> {
        let output = Command::new("git")
            .arg("-C")
            .arg(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|source| ScrawnyError::Io {
                path: start.to_path_buf(),
                source,
            })?;

        if !output.status.success() {
            return Err(ScrawnyError::NotRepository {
                path: start.to_path_buf(),
            });
        }

        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Self { root: PathBuf::from(root) })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()
            .map_err(|source| ScrawnyError::Io {
                path: self.root.clone(),
                source,
            })?;

        if !output.status.success() {
            return Err(ScrawnyError::GitFailure {
                command: format!("git {}", args.join(" ")),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

impl Repository for GitCliRepository {
    fn status(&self) -> Result<Status> {
        let raw = self.run(&["status", "--porcelain=v1", "-z"])?;
        let mut status = Status::default();

        for entry in raw.split('\0').filter(|e| !e.is_empty()) {
            if entry.len() < 3 {
                continue;
            }
            let index_status = entry.as_bytes()[0] as char;
            let worktree_status = entry.as_bytes()[1] as char;
            let path = PathBuf::from(entry[3..].to_string());

            if index_status == '?' && worktree_status == '?' {
                status.untracked.push(path);
                continue;
            }
            if index_status != ' ' && index_status != '?' {
                status.staged.push(path.clone());
            }
            if worktree_status != ' ' && worktree_status != '?' {
                status.unstaged.push(path);
            }
        }

        Ok(status)
    }

    fn diff(&self, spec: DiffSpec) -> Result<Diff> {
        let raw = match &spec {
            DiffSpec::WorkingTree => self.run(&["diff"])?,
            DiffSpec::Staged => self.run(&["diff", "--staged"])?,
            DiffSpec::HeadToWorkingTree => self.run(&["diff", "HEAD"])?,
            DiffSpec::Commit(sha) => self.run(&["diff", &format!("{sha}^!")])?,
            DiffSpec::Range { from, to } => {
                self.run(&["diff", &format!("{from}..{to}")])?
            }
        };

        parse_unified_diff(&raw)
    }

    fn commits(&self, spec: CommitSpec) -> Result<Vec<Commit>> {
        const FMT: &str = "%H%x1f%s%x1f%an%x1f%at%x1e";
        let raw = match &spec {
            CommitSpec::Last(n) => {
                self.run(&["log", &format!("-{n}"), &format!("--pretty=format:{FMT}")])?
            }
            CommitSpec::Range { from, to } => self.run(&[
                "log",
                &format!("{from}..{to}"),
                &format!("--pretty=format:{FMT}"),
            ])?,
        };

        let mut commits = Vec::new();
        for record in raw.split('\u{1e}') {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            let fields: Vec<&str> = record.split('\u{1f}').collect();
            if fields.len() < 4 {
                continue;
            }
            commits.push(Commit {
                id: fields[0].to_string(),
                summary: fields[1].to_string(),
                author: fields[2].to_string(),
                timestamp: fields[3].parse().unwrap_or(0),
            });
        }

        Ok(commits)
    }
}

/// Non-repository entry point: parse an already-acquired diff blob (e.g.
/// piped in via `scrawny diff --stdin`) without touching git at all.
pub fn diff_from_text(text: &str) -> Result<Diff> {
    parse_unified_diff(text)
}
