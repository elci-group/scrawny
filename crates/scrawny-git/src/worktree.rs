//! An isolated, disposable git worktree for constructing real, validated
//! commits without ever touching the caller's current branch or checkout
//! (§22 of the project directive: temporary worktree, apply, validate,
//! present — commit only happens here, on a throwaway branch, never on the
//! branch the user has checked out).
//!
//! `git worktree add -b <branch> <path> <base>` creates a second working
//! directory backed by the *same* object database as the main repository —
//! any commit made here is immediately a real, reachable ref from the main
//! checkout too (`git log <branch>`), with no fetch or import step needed.
//! Removing the worktree afterward only removes the scratch directory; the
//! branch and its commits remain.

use scrawny_core::{Result, ScrawnyError};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct SplitWorktree {
    repo_root: PathBuf,
    path: PathBuf,
    branch: String,
}

impl SplitWorktree {
    /// Create a new worktree at a fresh temp-dir path, checked out on a new
    /// branch `branch` starting from `base_rev`.
    pub fn create(repo_root: &Path, base_rev: &str, branch: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!("scrawny-split-{}", uuid_like()));
        run_in(repo_root, &["worktree", "add", "-b", branch, path_str(&path), base_rev])?;
        Ok(Self { repo_root: repo_root.to_path_buf(), path, branch: branch.to_string() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn write_file(&self, relative_path: &Path, content: &str) -> Result<()> {
        let full = self.path.join(relative_path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| ScrawnyError::Io { path: parent.to_path_buf(), source })?;
        }
        std::fs::write(&full, content).map_err(|source| ScrawnyError::Io { path: full, source })
    }

    pub fn delete_file(&self, relative_path: &Path) -> Result<()> {
        let full = self.path.join(relative_path);
        if full.exists() {
            std::fs::remove_file(&full).map_err(|source| ScrawnyError::Io { path: full, source })?;
        }
        Ok(())
    }

    /// Stage everything currently on disk and commit it. Returns the new
    /// commit's SHA.
    pub fn commit_all(&self, message: &str) -> Result<String> {
        run_in(&self.path, &["add", "-A"])?;
        run_in(&self.path, &["commit", "-q", "-m", message])?;
        let sha = run_in(&self.path, &["rev-parse", "HEAD"])?;
        Ok(sha.trim().to_string())
    }

    /// Confirm the worktree's working tree is clean (everything committed)
    /// — a hard check that every proposed file write was actually captured
    /// by a commit, not left stray.
    pub fn is_clean(&self) -> Result<bool> {
        let status = run_in(&self.path, &["status", "--porcelain"])?;
        Ok(status.trim().is_empty())
    }

    /// Remove the scratch working directory. The branch and its commits are
    /// untouched and remain reachable from the main repository.
    pub fn remove(self) -> Result<()> {
        run_in(&self.repo_root, &["worktree", "remove", "--force", path_str(&self.path)])?;
        Ok(())
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}

fn run_in(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|source| ScrawnyError::Io { path: dir.to_path_buf(), source })?;

    if !output.status.success() {
        return Err(ScrawnyError::GitFailure {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}-{}", std::process::id())
}
