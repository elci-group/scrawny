use scrawny_core::FileStatus;
use scrawny_git::{CommitSpec, DiffSpec, GitCliRepository, Repository};
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
    git(dir.path(), &["config", "user.name", "Scrawny Test"]);
    dir
}

#[test]
fn discovers_repository_root() {
    let dir = init_repo();
    let repo = GitCliRepository::discover(dir.path()).unwrap();
    let canonical_expected = std::fs::canonicalize(dir.path()).unwrap();
    let canonical_actual = std::fs::canonicalize(repo.root()).unwrap();
    assert_eq!(canonical_actual, canonical_expected);
}

#[test]
fn fails_gracefully_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let result = GitCliRepository::discover(dir.path());
    assert!(result.is_err());
}

#[test]
fn reports_working_tree_diff_after_a_commit_and_edit() {
    let dir = init_repo();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);

    std::fs::write(dir.path().join("main.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();

    let repo = GitCliRepository::discover(dir.path()).unwrap();
    let status = repo.status().unwrap();
    assert!(status.unstaged.iter().any(|p| p.to_str() == Some("main.rs")));

    let diff = repo.diff(DiffSpec::WorkingTree).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].status, FileStatus::Modified);
    assert!(diff.files[0].lines_added() >= 1);
}

#[test]
fn reports_staged_diff_separately_from_working_tree() {
    let dir = init_repo();
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);

    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();

    let repo = GitCliRepository::discover(dir.path()).unwrap();

    let staged = repo.diff(DiffSpec::Staged).unwrap();
    assert_eq!(staged.files[0].lines_added(), 1);

    let combined = repo.diff(DiffSpec::HeadToWorkingTree).unwrap();
    assert_eq!(combined.files[0].lines_added(), 2);
}

#[test]
fn lists_recent_commits() {
    let dir = init_repo();
    std::fs::write(dir.path().join("f.txt"), "1\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "first commit"]);
    std::fs::write(dir.path().join("f.txt"), "2\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "second commit"]);

    let repo = GitCliRepository::discover(dir.path()).unwrap();
    let commits = repo.commits(CommitSpec::Last(2)).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].summary, "second commit");
    assert_eq!(commits[1].summary, "first commit");
}
