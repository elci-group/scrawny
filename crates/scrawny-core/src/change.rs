use crate::diff::Range;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable identifier for a `Change` within one analysis run.
///
/// Deterministic within a run (see the project directive, O7): derived from
/// file path + hunk index, not from insertion order alone, so re-running
/// analysis over the same repository state reproduces identical ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChangeId(pub u64);

/// The syntactic nature of an edit, independent of *why* it was made.
///
/// This is Phase I's structural view (§8). The later, semantic
/// classification (mechanical / structural / behavioural / ...) lives in
/// `Concern` and is attached separately, because a single `ChangeKind` can
/// carry any `Concern` — a `Modified` region can be mechanical (reformatted)
/// or behavioural (new logic), and conflating the two would throw away the
/// distinction the whole project exists to preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Renamed,
    Moved,
    Unknown,
}

/// A semantically distinct purpose a change can serve.
///
/// Every concern classification MUST carry a confidence in `[0.0, 1.0]`
/// (§10: "Never pretend certainty"). `Unknown` is a first-class outcome, not
/// a fallback error path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Concern {
    Behavioural,
    Structural,
    Testing,
    Documentation,
    Mechanical,
    Configuration,
    Dependency,
    Unknown,
}

impl Concern {
    pub fn as_str(&self) -> &'static str {
        match self {
            Concern::Behavioural => "behavioural",
            Concern::Structural => "structural",
            Concern::Testing => "testing",
            Concern::Documentation => "documentation",
            Concern::Mechanical => "mechanical",
            Concern::Configuration => "configuration",
            Concern::Dependency => "dependency",
            Concern::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Classification {
    pub concern: Concern,
    pub confidence: f64,
}

impl Classification {
    pub fn new(concern: Concern, confidence: f64) -> Self {
        debug_assert!((0.0..=1.0).contains(&confidence));
        Self {
            concern,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// One structured, classified region of a diff.
///
/// This is the atomic unit the rest of the pipeline (clustering, scoring,
/// decomposition) operates on — never raw diff text again after extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub id: ChangeId,
    pub file: PathBuf,
    pub old_range: Range,
    pub new_range: Range,
    pub kind: ChangeKind,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub classification: Classification,
}

impl Change {
    pub fn lines_changed(&self) -> usize {
        self.lines_added + self.lines_removed
    }
}
