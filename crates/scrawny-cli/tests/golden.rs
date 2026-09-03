//! Golden fixture tests (§26): real before/after Rust files under
//! `fixtures/`, exercised end-to-end through the CLI, asserting on the
//! structural shape of the analysis rather than exact fixture text — so the
//! fixtures stay readable and reusable independent of this test.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

fn git(dir: &Path, args: &[&str]) {
    let status = StdCommand::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git command runnable");
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Scrawny Golden"]);
    dir
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

fn run_fixture(fixture_name: &str) -> serde_json::Value {
    let dir = init_repo();
    let before = std::fs::read_to_string(fixture(&format!("{fixture_name}/before.rs"))).unwrap();
    let after = std::fs::read_to_string(fixture(&format!("{fixture_name}/after.rs"))).unwrap();

    std::fs::write(dir.path().join("lib.rs"), &before).unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(dir.path().join("lib.rs"), &after).unwrap();

    let output = Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "--format", "json", "analyse"])
        .output()
        .unwrap();
    assert!(output.status.success(), "scrawny analyse failed: {output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn golden_mixed_concern_rust_splits_into_multiple_low_cohesion_clusters() {
    let json = run_fixture("mixed_concern_rust");
    let clusters = json["clusters"].as_array().unwrap();
    let cohesion = json["metrics"]["cohesion"].as_f64().unwrap();

    // Counter::increment and format_report are unrelated: no shared symbol,
    // no reference between them. Real clustering must keep them apart even
    // though they're both plain Behavioural edits to the same file.
    assert!(
        clusters.len() >= 2,
        "expected the mixed fixture to split into multiple clusters, got {clusters:#?}"
    );
    assert!(cohesion < 0.7, "expected low cohesion for unrelated edits, got {cohesion}");
}

#[test]
fn golden_large_coherent_change_stays_one_high_cohesion_cluster() {
    let json = run_fixture("large_coherent_change");
    let clusters = json["clusters"].as_array().unwrap();
    let cohesion = json["metrics"]["cohesion"].as_f64().unwrap();

    // `process` calls `validate`; editing both is one coherent unit of work
    // even though it touches two functions and >200 chars of diff.
    assert_eq!(clusters.len(), 1, "expected one connected cluster, got {clusters:#?}");
    assert!(cohesion > 0.9, "expected high cohesion for a coherent dependent change, got {cohesion}");
}
