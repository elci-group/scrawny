use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
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
    git(dir.path(), &["config", "user.name", "Scrawny Test"]);
    dir
}

#[test]
fn analyse_reports_a_coherent_behavioural_change() {
    let dir = init_repo();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(
        dir.path().join("lib.rs"),
        "fn main() {\n    do_work();\n}\n\nfn do_work() {}\n",
    )
    .unwrap();

    Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "analyse"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Review load"))
        .stdout(predicate::str::contains("No automatic changes made."));
}

#[test]
fn analyse_json_output_is_valid_and_matches_schema() {
    let dir = init_repo();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(dir.path().join("lib.rs"), "fn main() {\n    do_work();\n}\n").unwrap();

    let output = Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "--format", "json", "analyse"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["version"], "1");
    assert!(json["metrics"]["review_load"]["total"].is_number());
    assert!(json["clusters"].as_array().unwrap().is_empty());
    assert!(json["recommendations"].as_array().unwrap().is_empty());
}

#[test]
fn check_fails_when_review_load_exceeds_policy() {
    let dir = init_repo();
    std::fs::write(dir.path().join(".scrawny.toml"), "[policy]\nmax_review_load = 1\n").unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(
        dir.path().join("lib.rs"),
        "fn main() {\n    do_work();\n    do_more();\n}\n\nfn do_work() {}\nfn do_more() {}\n",
    )
    .unwrap();

    Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "check"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("SCRAWNY FAILURE"));
}

#[test]
fn check_passes_on_a_small_coherent_change() {
    let dir = init_repo();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(dir.path().join("lib.rs"), "fn main() {\n    do_work();\n}\n").unwrap();

    Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SCRAWNY OK"));
}

#[test]
fn suggest_is_honest_about_not_having_decomposition_yet() {
    let dir = init_repo();
    std::fs::write(dir.path().join("lib.rs"), "fn main() {}\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "initial"]);
    std::fs::write(dir.path().join("lib.rs"), "fn main() {\n    do_work();\n}\n").unwrap();

    Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "suggest"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Phase II"));
}

#[test]
fn diff_stdin_works_without_a_git_repository() {
    let diff_text = "diff --git a/a.rs b/a.rs\n\
        index 1111111..2222222 100644\n\
        --- a/a.rs\n\
        +++ b/a.rs\n\
        @@ -1,1 +1,1 @@\n\
        -old\n\
        +new\n";

    let mut cmd = Command::cargo_bin("scrawny").unwrap();
    cmd.args(["diff", "--stdin"]).write_stdin(diff_text);
    cmd.assert().success().stdout(predicate::str::contains("Review load"));
}

#[test]
fn check_stdin_honours_the_policy_at_path() {
    // Mirrors how kaptaind's integration analysis invokes Scrawny: it has no
    // interest in walking a git repo itself, it already has `git diff
    // target...source` text and just wants a policy verdict against that
    // repo's own `.scrawny.toml`.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".scrawny.toml"), "[policy]\nmax_review_load = 1\n").unwrap();

    let diff_text = "diff --git a/a.rs b/a.rs\n\
        index 1111111..2222222 100644\n\
        --- a/a.rs\n\
        +++ b/a.rs\n\
        @@ -1,1 +1,1 @@\n\
        -old\n\
        +new\n";

    let mut cmd = Command::cargo_bin("scrawny").unwrap();
    cmd.args(["--path", dir.path().to_str().unwrap(), "--format", "json", "check", "--stdin"])
        .write_stdin(diff_text);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["passed"], false);
}

#[test]
fn errors_cleanly_outside_a_repository_without_stdin() {
    let dir = tempfile::tempdir().unwrap();
    Command::cargo_bin("scrawny")
        .unwrap()
        .args(["--path", dir.path().to_str().unwrap(), "analyse"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}
