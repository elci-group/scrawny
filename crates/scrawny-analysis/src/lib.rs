pub mod classify;
pub mod extract;
pub mod score;

use scrawny_config::Config;
use scrawny_core::{Change, Concern, Diff};
use serde::{Deserialize, Serialize};

pub use score::{ReviewLoad, SCORE_FORMULA_VERSION};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub changes: Vec<Change>,
    pub review_load: ReviewLoad,
    pub cohesion: f64,
    pub concerns: Vec<ConcernSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcernSummary {
    pub concern: Concern,
    pub lines: usize,
    pub count: usize,
}

pub fn analyse(diff: &Diff, config: &Config) -> AnalysisResult {
    let changes = extract::extract_changes(diff, config);
    let review_load = score::compute_review_load(&changes);
    let cohesion = score::compute_cohesion(&changes);
    let concerns = score::concern_summary(&changes)
        .into_iter()
        .map(|(concern, lines, count)| ConcernSummary { concern, lines, count })
        .collect();

    AnalysisResult {
        lines_added: diff.lines_added(),
        lines_removed: diff.lines_removed(),
        files_changed: diff.files.len(),
        changes,
        review_load,
        cohesion,
        concerns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scrawny_core::{DiffLine, FileDiff, FileStatus, Hunk, Range};
    use std::path::PathBuf;

    fn hunk(added: &[&str], removed: &[&str]) -> Hunk {
        let mut lines = Vec::new();
        for r in removed {
            lines.push(DiffLine::Removed(r.to_string()));
        }
        for a in added {
            lines.push(DiffLine::Added(a.to_string()));
        }
        Hunk {
            old_range: Range::new(1, removed.len().max(1)),
            new_range: Range::new(1, added.len().max(1)),
            lines,
        }
    }

    #[test]
    fn single_concern_change_has_high_cohesion() {
        let diff = Diff {
            files: vec![FileDiff {
                path: PathBuf::from("src/lib.rs"),
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![hunk(&["fn new_behaviour() {}"], &[])],
            }],
        };

        let result = analyse(&diff, &Config::default());
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].classification.concern, Concern::Behavioural);
        assert!((result.cohesion - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mixed_concerns_lower_cohesion_and_raise_load() {
        let diff = Diff {
            files: vec![
                FileDiff {
                    path: PathBuf::from("src/lib.rs"),
                    old_path: None,
                    status: FileStatus::Modified,
                    is_binary: false,
                    hunks: vec![hunk(&["fn behaviour() { do_thing(); }"], &[])],
                },
                FileDiff {
                    path: PathBuf::from("README.md"),
                    old_path: None,
                    status: FileStatus::Modified,
                    is_binary: false,
                    hunks: vec![hunk(&["Updated usage docs."], &[])],
                },
                FileDiff {
                    path: PathBuf::from("tests/lib_test.rs"),
                    old_path: None,
                    status: FileStatus::Modified,
                    is_binary: false,
                    hunks: vec![hunk(&["fn test_behaviour() { assert!(true); }"], &[])],
                },
            ],
        };

        let single = analyse(
            &Diff {
                files: vec![diff.files[0].clone()],
            },
            &Config::default(),
        );
        let mixed = analyse(&diff, &Config::default());

        assert!(mixed.cohesion < single.cohesion);
        assert_eq!(mixed.concerns.len(), 3);
    }

    #[test]
    fn whitespace_only_hunk_is_mechanical() {
        let diff = Diff {
            files: vec![FileDiff {
                path: PathBuf::from("src/lib.rs"),
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![hunk(&["fn behaviour() {}"], &["fn   behaviour()   {}"])],
            }],
        };

        let result = analyse(&diff, &Config::default());
        assert_eq!(result.changes[0].classification.concern, Concern::Mechanical);
    }

    #[test]
    fn generated_lockfile_is_mechanical_low_weight() {
        let diff = Diff {
            files: vec![FileDiff {
                path: PathBuf::from("Cargo.lock"),
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![hunk(
                    &(1..=200).map(|i| format!("dep-{i} = \"1.0\"")).collect::<Vec<_>>()
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    &[],
                )],
            }],
        };

        let result = analyse(&diff, &Config::default());
        assert_eq!(result.changes[0].classification.concern, Concern::Mechanical);
        // 200 mechanical lines shouldn't dominate the score the way 200
        // behavioural lines would.
        assert!(result.review_load.behavioural_density < 1.0);
    }

    #[test]
    fn empty_diff_has_zero_load() {
        let diff = Diff::default();
        let result = analyse(&diff, &Config::default());
        assert_eq!(result.review_load.total, 0);
        assert!((result.cohesion - 1.0).abs() < f64::EPSILON);
    }
}
