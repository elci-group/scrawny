use scrawny_git::{GitCliRepository, SplitWorktree};
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
fn creates_real_commits_without_touching_the_current_branch() {
    let dir = init_repo();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);

    let repo = GitCliRepository::discover(dir.path()).unwrap();
    let head_before = String::from_utf8(
        Command::new("git").arg("-C").arg(dir.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
    )
    .unwrap();
    let branch_before = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["branch", "--show-current"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();

    let worktree = SplitWorktree::create(repo.root(), "HEAD", "scrawny/split-test").unwrap();
    worktree.write_file(Path::new("lib.rs"), "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    let sha = worktree.commit_all("behavioural: main").unwrap();
    assert!(worktree.is_clean().unwrap());
    assert!(!sha.is_empty());

    // The current branch/checkout in the main repo must be completely
    // untouched by any of this.
    let head_after = String::from_utf8(
        Command::new("git").arg("-C").arg(dir.path()).args(["rev-parse", "HEAD"]).output().unwrap().stdout,
    )
    .unwrap();
    let branch_after = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["branch", "--show-current"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(head_before, head_after);
    assert_eq!(branch_before, branch_after);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("lib.rs")).unwrap(),
        "fn main() {}\n",
        "main repo's working tree must be untouched"
    );

    // But the new branch IS a real, reachable ref from the main repo.
    let log = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["log", "--oneline", "scrawny/split-test"])
        .output()
        .unwrap();
    assert!(log.status.success());
    assert!(String::from_utf8_lossy(&log.stdout).contains("behavioural: main"));

    let worktree_path = worktree.path().to_path_buf();
    worktree.remove().unwrap();
    assert!(!worktree_path.exists(), "worktree directory should be gone after remove()");
}

#[test]
fn deleting_a_file_in_the_worktree_is_captured_by_commit_all() {
    let dir = init_repo();
    std::fs::write(dir.path().join("keep.rs"), "fn keep() {}\n").unwrap();
    std::fs::write(dir.path().join("gone.rs"), "fn gone() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);

    let repo = GitCliRepository::discover(dir.path()).unwrap();
    let worktree = SplitWorktree::create(repo.root(), "HEAD", "scrawny/split-delete-test").unwrap();
    worktree.delete_file(Path::new("gone.rs")).unwrap();
    worktree.commit_all("chore: remove gone.rs").unwrap();
    assert!(worktree.is_clean().unwrap());
    assert!(!worktree.path().join("gone.rs").exists());
    assert!(worktree.path().join("keep.rs").exists());

    worktree.remove().unwrap();
}
