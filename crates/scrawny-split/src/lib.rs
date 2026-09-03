//! Phase III: transactional commit-series construction (§13/§22 of the
//! project directive).
//!
//! `build_plan` turns Phase II's clusters into an ordered series of proposed
//! commits, each carrying the *full resulting content* of every file it
//! touches — never a textual patch guessed to "probably" apply. Before
//! returning a plan, it replays every commit and checks the final state
//! against the actual target content, byte for byte (§27: "must not
//! silently lose a changed line"). A plan that fails that check is an
//! error, never presented as if it were safe.
//!
//! This crate never touches git, a working tree, or any filesystem path —
//! it is pure content computation. Materialising a plan into a real
//! (disposable) worktree and constructing real commits from it is the
//! caller's job (the CLI, which has git access) — see the project
//! directive's safety model (§22): snapshot, propose, apply to a temporary
//! worktree, validate, present, and only ever commit for real after
//! explicit confirmation. This crate provides the "propose" and the first
//! half of "validate"; it never runs `--apply` and has no concept of it.

mod assignment;
mod reconstruct;

use scrawny_cluster::Cluster;
use scrawny_core::{Change, Concern, Diff, FileStatus, ScrawnyError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use assignment::ClusterAssignment;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileChange {
    Write { path: PathBuf, content: String },
    Delete { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedCommit {
    pub cluster_id: usize,
    pub message: String,
    pub evidence: Vec<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitPlan {
    pub commits: Vec<ProposedCommit>,
}

pub struct PlanInputs<'a> {
    pub diff: &'a Diff,
    /// Full original content, keyed by new path, for every `Modified` file.
    pub base_content: &'a HashMap<PathBuf, String>,
    /// Full final content, keyed by new path, for every file touched by the
    /// diff except `Removed` ones — the round-trip validation target.
    pub target_content: &'a HashMap<PathBuf, String>,
    pub symbols: &'a scrawny_cluster::SymbolTable,
}

/// Build a validated series of proposed commits, one per cluster, in
/// ascending cluster-id order. Returns `ScrawnyError::Validation` rather
/// than a plan if replaying every commit doesn't reproduce the real target
/// state exactly — see the module docs.
pub fn build_plan(
    inputs: &PlanInputs,
    clusters: &[Cluster],
    changes: &[Change],
) -> Result<SplitPlan, ScrawnyError> {
    let assignment = ClusterAssignment::build(clusters, changes);

    let mut changes_by_file: HashMap<&Path, Vec<&Change>> = HashMap::new();
    for change in changes {
        changes_by_file.entry(change.file.as_path()).or_default().push(change);
    }

    let mut atomic_files: HashMap<&Path, usize> = HashMap::new();
    for file in &inputs.diff.files {
        if matches!(file.status, FileStatus::Removed | FileStatus::Renamed | FileStatus::Copied) {
            if let Some(cluster_id) = changes_by_file
                .get(file.path.as_path())
                .and_then(|cs| cs.first())
                .and_then(|c| cluster_of(c, &assignment))
            {
                atomic_files.insert(file.path.as_path(), cluster_id);
            }
        }
    }

    let mut ordered: Vec<&Cluster> = clusters.iter().collect();
    ordered.sort_by_key(|c| c.id);

    let mut applied: HashSet<usize> = HashSet::new();
    // Seeded with the real pre-split state so the *first* cluster to touch a
    // file is compared against what the file actually looked like before
    // any cluster was applied, not an unset placeholder — otherwise every
    // file a cluster merely re-derives unchanged (because it isn't that
    // cluster's own change) would be wrongly reported as touched by
    // whichever cluster happens to run first.
    let mut current_state: HashMap<PathBuf, Option<String>> = inputs
        .diff
        .files
        .iter()
        .filter(|f| matches!(f.status, FileStatus::Modified | FileStatus::Added))
        .map(|f| (f.path.clone(), inputs.base_content.get(&f.path).cloned()))
        .collect();
    let mut commits = Vec::new();

    for cluster in ordered {
        applied.insert(cluster.id);
        let mut touched = Vec::new();

        for file in &inputs.diff.files {
            match file.status {
                FileStatus::Modified | FileStatus::Added => {
                    let Some(file_changes) = changes_by_file.get(file.path.as_path()) else {
                        continue;
                    };
                    let file_symbols = inputs.symbols.get(&file.path).map(Vec::as_slice);
                    let base = inputs.base_content.get(&file.path).map(String::as_str);
                    let content = reconstruct::reconstruct_modified_or_added(
                        file,
                        base,
                        file_symbols,
                        file_changes,
                        &assignment,
                        &applied,
                    );
                    let previous = current_state.get(&file.path).cloned().flatten();
                    if previous.as_deref() != Some(content.as_str()) {
                        touched.push(FileChange::Write { path: file.path.clone(), content: content.clone() });
                    }
                    current_state.insert(file.path.clone(), Some(content));
                }
                FileStatus::Removed => {
                    if atomic_files.get(file.path.as_path()) == Some(&cluster.id) {
                        touched.push(FileChange::Delete { path: file.path.clone() });
                        current_state.insert(file.path.clone(), None);
                    }
                }
                FileStatus::Renamed | FileStatus::Copied => {
                    if atomic_files.get(file.path.as_path()) == Some(&cluster.id) {
                        if let Some(content) = inputs.target_content.get(&file.path) {
                            touched.push(FileChange::Write {
                                path: file.path.clone(),
                                content: content.clone(),
                            });
                            current_state.insert(file.path.clone(), Some(content.clone()));
                        }
                        if file.status == FileStatus::Renamed {
                            if let Some(old_path) = &file.old_path {
                                touched.push(FileChange::Delete { path: old_path.clone() });
                            }
                        }
                    }
                }
                FileStatus::Unknown => {}
            }
        }

        if touched.is_empty() {
            continue;
        }

        let (lines_added, lines_removed) = cluster_stats(cluster, changes);
        commits.push(ProposedCommit {
            cluster_id: cluster.id,
            message: commit_message(cluster, changes),
            evidence: cluster.evidence.clone(),
            lines_added,
            lines_removed,
            files: touched,
        });
    }

    validate(inputs, &current_state)?;

    Ok(SplitPlan { commits })
}

fn cluster_of(change: &Change, assignment: &ClusterAssignment) -> Option<usize> {
    match change.touched_symbols.first() {
        Some(name) => assignment.cluster_for_symbol(&change.file, name),
        None => assignment.cluster_for_change(change.id),
    }
}

fn cluster_stats(cluster: &Cluster, changes: &[Change]) -> (usize, usize) {
    let by_id: HashMap<_, _> = changes.iter().map(|c| (c.id, c)).collect();
    cluster.change_ids.iter().filter_map(|id| by_id.get(id)).fold((0, 0), |(a, r), c| {
        (a + c.lines_added, r + c.lines_removed)
    })
}

fn commit_message(cluster: &Cluster, changes: &[Change]) -> String {
    let by_id: HashMap<_, _> = changes.iter().map(|c| (c.id, c)).collect();
    let mut by_concern: BTreeMap<&'static str, usize> = BTreeMap::new();
    for id in &cluster.change_ids {
        if let Some(c) = by_id.get(id) {
            *by_concern.entry(c.classification.concern.as_str()).or_insert(0) += c.lines_changed();
        }
    }
    let dominant = by_concern
        .into_iter()
        .max_by_key(|(_, lines)| *lines)
        .map(|(concern, _)| concern)
        .unwrap_or(Concern::Unknown.as_str());

    let subject = if !cluster.symbols.is_empty() {
        cluster.symbols.join(", ")
    } else {
        cluster.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", ")
    };

    let mut message = format!("{dominant}: {subject}");
    if !cluster.evidence.is_empty() {
        message.push_str("\n\n");
        for reason in &cluster.evidence {
            message.push_str("- ");
            message.push_str(reason);
            message.push('\n');
        }
    }
    message
}

/// Replay every commit's cumulative effect and confirm it reproduces the
/// real target state exactly. This is the hard safety gate (§22, §27): a
/// mismatch here means some reconstruction assumption didn't hold for this
/// diff, and the honest response is to refuse the plan, not present one
/// that silently drops or corrupts content.
fn validate(
    inputs: &PlanInputs,
    final_state: &HashMap<PathBuf, Option<String>>,
) -> Result<(), ScrawnyError> {
    for file in &inputs.diff.files {
        if file.status == FileStatus::Removed {
            if let Some(Some(_)) = final_state.get(&file.path) {
                return Err(ScrawnyError::Validation {
                    reason: format!(
                        "{}: expected this file to be removed, but reconstruction still has content for it",
                        file.path.display()
                    ),
                });
            }
            continue;
        }

        let Some(target) = inputs.target_content.get(&file.path) else {
            continue;
        };
        match final_state.get(&file.path) {
            Some(Some(content)) if content == target => {}
            Some(Some(_)) => {
                return Err(ScrawnyError::Validation {
                    reason: format!(
                        "{}: reconstructed content after all clusters does not match the real target content",
                        file.path.display()
                    ),
                })
            }
            _ => {
                return Err(ScrawnyError::Validation {
                    reason: format!("{}: no reconstructed content was produced", file.path.display()),
                })
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scrawny_core::{DiffLine, FileDiff, FileStatus, Hunk, Range};
    use scrawny_parser::parse_rust_symbols;
    use std::path::PathBuf;

    /// The exact scenario found live in development: two unrelated
    /// functions close enough together that git's default 3-line context
    /// merges both edits into a single hunk.
    fn mixed_concern_setup() -> (Diff, HashMap<PathBuf, String>, HashMap<PathBuf, String>, scrawny_cluster::SymbolTable)
    {
        let before = "fn alpha() {\n    println!(\"a\");\n}\n\nfn beta() {\n    println!(\"b\");\n}\n";
        let after =
            "fn alpha() {\n    println!(\"a changed\");\n}\n\nfn beta() {\n    println!(\"b changed\");\n}\n";
        let path = PathBuf::from("lib.rs");

        let hunk = Hunk {
            old_range: Range::new(1, 7),
            new_range: Range::new(1, 7),
            lines: vec![
                DiffLine::Context("fn alpha() {".to_string()),
                DiffLine::Removed("    println!(\"a\");".to_string()),
                DiffLine::Added("    println!(\"a changed\");".to_string()),
                DiffLine::Context("}".to_string()),
                DiffLine::Context(String::new()),
                DiffLine::Context("fn beta() {".to_string()),
                DiffLine::Removed("    println!(\"b\");".to_string()),
                DiffLine::Added("    println!(\"b changed\");".to_string()),
                DiffLine::Context("}".to_string()),
            ],
        };

        let diff = Diff {
            files: vec![FileDiff {
                path: path.clone(),
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![hunk],
            }],
        };

        let mut base_content = HashMap::new();
        base_content.insert(path.clone(), before.to_string());
        let mut target_content = HashMap::new();
        target_content.insert(path.clone(), after.to_string());

        let symbols = parse_rust_symbols(&path, after).unwrap();
        let mut table = scrawny_cluster::SymbolTable::new();
        table.insert(path, symbols);

        (diff, base_content, target_content, table)
    }

    #[test]
    fn splits_two_unrelated_functions_sharing_one_hunk_into_two_valid_commits() {
        let (diff, base_content, target_content, symbols) = mixed_concern_setup();
        let config = scrawny_config::Config::default();
        let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
        let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2, "alpha and beta must not merge");

        let inputs = PlanInputs {
            diff: &diff,
            base_content: &base_content,
            target_content: &target_content,
            symbols: &symbols,
        };
        let plan = build_plan(&inputs, &clusters, &changes).expect("plan must validate");

        assert_eq!(plan.commits.len(), 2);

        // The first commit changes exactly one of the two functions, not both.
        let FileChange::Write { content: after_first, .. } = &plan.commits[0].files[0] else {
            panic!("expected a write");
        };
        let has_a = after_first.contains("a changed");
        let has_b = after_first.contains("b changed");
        assert!(has_a ^ has_b, "first commit should touch exactly one function, got: {after_first}");

        // The second commit's file content must equal the real target exactly.
        let FileChange::Write { content: after_second, .. } = &plan.commits[1].files[0] else {
            panic!("expected a write");
        };
        assert_eq!(after_second, target_content.get(&PathBuf::from("lib.rs")).unwrap());
    }

    #[test]
    fn validation_rejects_a_plan_that_would_not_reach_the_real_target() {
        let (diff, base_content, mut target_content, symbols) = mixed_concern_setup();
        // Corrupt the target so it can never match what reconstruction
        // actually produces — validation must refuse this, not shrug.
        target_content.insert(PathBuf::from("lib.rs"), "not the real content".to_string());

        let config = scrawny_config::Config::default();
        let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
        let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);

        let inputs = PlanInputs {
            diff: &diff,
            base_content: &base_content,
            target_content: &target_content,
            symbols: &symbols,
        };
        let result = build_plan(&inputs, &clusters, &changes);
        assert!(result.is_err(), "a plan that can't reach the real target must be rejected");
    }

    #[test]
    fn a_single_cluster_still_produces_one_valid_commit() {
        let path = PathBuf::from("lib.rs");
        let before = "fn only() {\n    old();\n}\n";
        let after = "fn only() {\n    new();\n}\n";
        let hunk = Hunk {
            old_range: Range::new(1, 3),
            new_range: Range::new(1, 3),
            lines: vec![
                DiffLine::Context("fn only() {".to_string()),
                DiffLine::Removed("    old();".to_string()),
                DiffLine::Added("    new();".to_string()),
                DiffLine::Context("}".to_string()),
            ],
        };
        let diff = Diff {
            files: vec![FileDiff {
                path: path.clone(),
                old_path: None,
                status: FileStatus::Modified,
                is_binary: false,
                hunks: vec![hunk],
            }],
        };
        let mut base_content = HashMap::new();
        base_content.insert(path.clone(), before.to_string());
        let mut target_content = HashMap::new();
        target_content.insert(path.clone(), after.to_string());
        let mut symbols = scrawny_cluster::SymbolTable::new();
        symbols.insert(path.clone(), parse_rust_symbols(&path, after).unwrap());

        let config = scrawny_config::Config::default();
        let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
        let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 1);

        let inputs =
            PlanInputs { diff: &diff, base_content: &base_content, target_content: &target_content, symbols: &symbols };
        let plan = build_plan(&inputs, &clusters, &changes).unwrap();
        assert_eq!(plan.commits.len(), 1);
        let FileChange::Write { content, .. } = &plan.commits[0].files[0] else {
            panic!("expected a write");
        };
        assert_eq!(content, after);
    }

    #[test]
    fn an_added_file_reconstructs_with_no_base_content() {
        let path = PathBuf::from("new_mod.rs");
        let after = "fn one() {}\nfn two() {}\n";
        let hunk = Hunk {
            old_range: Range::new(0, 0),
            new_range: Range::new(1, 2),
            lines: vec![
                DiffLine::Added("fn one() {}".to_string()),
                DiffLine::Added("fn two() {}".to_string()),
            ],
        };
        let diff = Diff {
            files: vec![FileDiff {
                path: path.clone(),
                old_path: None,
                status: FileStatus::Added,
                is_binary: false,
                hunks: vec![hunk],
            }],
        };
        let base_content = HashMap::new();
        let mut target_content = HashMap::new();
        target_content.insert(path.clone(), after.to_string());
        let mut symbols = scrawny_cluster::SymbolTable::new();
        symbols.insert(path.clone(), parse_rust_symbols(&path, after).unwrap());

        let config = scrawny_config::Config::default();
        let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
        let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2, "one() and two() are unrelated new functions");

        let inputs =
            PlanInputs { diff: &diff, base_content: &base_content, target_content: &target_content, symbols: &symbols };
        let plan = build_plan(&inputs, &clusters, &changes).expect("plan must validate");
        assert_eq!(plan.commits.len(), 2);
    }

    #[test]
    fn a_removed_file_is_deleted_atomically_in_one_commit() {
        let removed_path = PathBuf::from("old_mod.rs");
        let modified_path = PathBuf::from("lib.rs");
        let before_modified = "fn keep() {\n    old();\n}\n";
        let after_modified = "fn keep() {\n    new();\n}\n";

        let remove_hunk = Hunk {
            old_range: Range::new(1, 1),
            new_range: Range::new(0, 0),
            lines: vec![DiffLine::Removed("fn gone() {}".to_string())],
        };
        let modify_hunk = Hunk {
            old_range: Range::new(1, 3),
            new_range: Range::new(1, 3),
            lines: vec![
                DiffLine::Context("fn keep() {".to_string()),
                DiffLine::Removed("    old();".to_string()),
                DiffLine::Added("    new();".to_string()),
                DiffLine::Context("}".to_string()),
            ],
        };

        let diff = Diff {
            files: vec![
                FileDiff {
                    path: removed_path.clone(),
                    old_path: None,
                    status: FileStatus::Removed,
                    is_binary: false,
                    hunks: vec![remove_hunk],
                },
                FileDiff {
                    path: modified_path.clone(),
                    old_path: None,
                    status: FileStatus::Modified,
                    is_binary: false,
                    hunks: vec![modify_hunk],
                },
            ],
        };

        let mut base_content = HashMap::new();
        base_content.insert(modified_path.clone(), before_modified.to_string());
        let mut target_content = HashMap::new();
        target_content.insert(modified_path.clone(), after_modified.to_string());
        let mut symbols = scrawny_cluster::SymbolTable::new();
        symbols.insert(modified_path.clone(), parse_rust_symbols(&modified_path, after_modified).unwrap());

        let config = scrawny_config::Config::default();
        let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
        let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2);

        let inputs =
            PlanInputs { diff: &diff, base_content: &base_content, target_content: &target_content, symbols: &symbols };
        let plan = build_plan(&inputs, &clusters, &changes).expect("plan must validate");
        assert_eq!(plan.commits.len(), 2);

        let delete_commit = plan
            .commits
            .iter()
            .find(|c| c.files.iter().any(|f| matches!(f, FileChange::Delete { .. })))
            .expect("one commit should delete old_mod.rs");
        assert_eq!(delete_commit.files.len(), 1);
    }
}
