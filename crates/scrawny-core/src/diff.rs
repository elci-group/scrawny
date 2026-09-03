use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// An inclusive-exclusive line range, 1-indexed at `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLine {
    Context(String),
    Added(String),
    Removed(String),
}

/// A single git-diff hunk: one contiguous `@@ ... @@` region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub old_range: Range,
    pub new_range: Range,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn lines_added(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Added(_)))
            .count()
    }

    pub fn lines_removed(&self) -> usize {
        self.lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Removed(_)))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileStatus {
    Added,
    Removed,
    Modified,
    Renamed,
    Copied,
    Unknown,
}

/// One file's worth of diff: identity plus its hunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub status: FileStatus,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn lines_added(&self) -> usize {
        self.hunks.iter().map(Hunk::lines_added).sum()
    }

    pub fn lines_removed(&self) -> usize {
        self.hunks.iter().map(Hunk::lines_removed).sum()
    }
}

/// The full diff under analysis: one or more files, already normalised.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

impl Diff {
    pub fn lines_added(&self) -> usize {
        self.files.iter().map(FileDiff::lines_added).sum()
    }

    pub fn lines_removed(&self) -> usize {
        self.files.iter().map(FileDiff::lines_removed).sum()
    }
}
