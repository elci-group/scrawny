pub mod classify;
pub mod extract;
pub mod score;

use scrawny_cluster::{cluster_changes, Cluster};
use scrawny_config::Config;
use scrawny_core::{Change, Concern, Diff};
use serde::{Deserialize, Serialize};

pub use score::{ReviewLoad, SCORE_FORMULA_VERSION};
pub use scrawny_cluster::SymbolTable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
    pub changes: Vec<Change>,
    pub review_load: ReviewLoad,
    pub cohesion: f64,
    pub concerns: Vec<ConcernSummary>,
    pub clusters: Vec<Cluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcernSummary {
    pub concern: Concern,
    pub lines: usize,
    pub count: usize,
}

/// Analyse without any resolved symbol data — cohesion falls back to the
/// concern-fraction proxy and `clusters` is one singleton per change. Prefer
/// `analyse_with_symbols` whenever file content is available to resolve.
pub fn analyse(diff: &Diff, config: &Config) -> AnalysisResult {
    analyse_with_symbols(diff, config, &SymbolTable::new())
}

/// Analyse using resolved Rust symbol data where available (§9/§12). Cohesion
/// is cluster-based whenever `symbols` resolved at least one file; otherwise
/// it degrades to the Phase I concern-fraction proxy rather than reporting a
/// falsely low score for changes we simply have no AST insight into.
pub fn analyse_with_symbols(diff: &Diff, config: &Config, symbols: &SymbolTable) -> AnalysisResult {
    let changes = extract::extract_changes(diff, config, symbols);
    let clusters = cluster_changes(&changes, symbols);
    let review_load = score::compute_review_load(&changes);
    let cohesion = if symbols.is_empty() {
        score::compute_concern_cohesion(&changes)
    } else {
        score::compute_cluster_cohesion(&changes, &clusters)
    };
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
        clusters,
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

    fn range_hunk(added: &[&str], new_start: usize) -> Hunk {
        Hunk {
            old_range: Range::new(new_start, new_start + added.len().saturating_sub(1)),
            new_range: Range::new(new_start, new_start + added.len().saturating_sub(1)),
            lines: added.iter().map(|a| DiffLine::Added(a.to_string())).collect(),
        }
    }

    #[test]
    fn unrelated_functions_get_lower_cohesion_with_symbol_data_than_without() {
        let content = "fn alpha() {\n    println!(\"a\");\n}\n\nfn beta() {\n    println!(\"b\");\n}\n";
        let path = PathBuf::from("src/lib.rs");
        let symbols = scrawny_parser::parse_rust_symbols(&path, content).unwrap();
        let mut table = SymbolTable::new();
        table.insert(path.clone(), symbols);

        let diff = Diff {
            files: vec![FileDiff {
                path,
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![
                    range_hunk(&["    println!(\"a changed\");"], 2),
                    range_hunk(&["    println!(\"b changed\");"], 6),
                ],
            }],
        };

        let phase1 = analyse(&diff, &Config::default());
        let phase2 = analyse_with_symbols(&diff, &Config::default(), &table);

        // Same-concern (both Behavioural) proxy sees one blob; real symbol
        // clustering correctly sees two unrelated functions.
        assert!((phase1.cohesion - 1.0).abs() < f64::EPSILON);
        assert_eq!(phase2.clusters.len(), 2);
        assert!(phase2.cohesion < phase1.cohesion);
        assert!((phase2.cohesion - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn dependent_functions_stay_one_cluster_with_symbol_data() {
        let content =
            "fn helper() {\n    println!(\"h\");\n}\n\nfn main() {\n    helper();\n}\n";
        let path = PathBuf::from("src/lib.rs");
        let symbols = scrawny_parser::parse_rust_symbols(&path, content).unwrap();
        let mut table = SymbolTable::new();
        table.insert(path.clone(), symbols);

        let diff = Diff {
            files: vec![FileDiff {
                path,
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![
                    range_hunk(&["    println!(\"h changed\");"], 2),
                    range_hunk(&["    helper();"], 6),
                ],
            }],
        };

        let result = analyse_with_symbols(&diff, &Config::default(), &table);
        assert_eq!(result.clusters.len(), 1);
        assert!((result.cohesion - 1.0).abs() < f64::EPSILON);
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
