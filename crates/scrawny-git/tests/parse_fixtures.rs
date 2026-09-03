use scrawny_core::{DiffLine, FileStatus};
use scrawny_git::parse_unified_diff;

const MODIFIED_DIFF: &str = r#"diff --git a/src/lib.rs b/src/lib.rs
index 1234567..89abcde 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@
 fn main() {
-    println!("old");
+    println!("new");
+    println!("second line");
 }
"#;

#[test]
fn parses_a_modified_file_hunk() {
    let diff = parse_unified_diff(MODIFIED_DIFF).unwrap();
    assert_eq!(diff.files.len(), 1);
    let file = &diff.files[0];
    assert_eq!(file.path.to_str().unwrap(), "src/lib.rs");
    assert_eq!(file.status, FileStatus::Modified);
    assert_eq!(file.hunks.len(), 1);

    let hunk = &file.hunks[0];
    assert_eq!(hunk.lines_added(), 2);
    assert_eq!(hunk.lines_removed(), 1);
    assert_eq!(hunk.old_range.start, 1);
    assert_eq!(hunk.new_range.start, 1);
}

const ADDED_FILE_DIFF: &str = r#"diff --git a/src/new_module.rs b/src/new_module.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/src/new_module.rs
@@ -0,0 +1,2 @@
+pub fn hello() {}
+pub fn world() {}
"#;

#[test]
fn parses_a_new_file() {
    let diff = parse_unified_diff(ADDED_FILE_DIFF).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].status, FileStatus::Added);
    assert_eq!(diff.files[0].hunks[0].lines_added(), 2);
    assert_eq!(diff.files[0].hunks[0].lines_removed(), 0);
}

const RENAMED_DIFF: &str = r#"diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 100%
rename from src/old_name.rs
rename to src/new_name.rs
"#;

#[test]
fn parses_a_pure_rename() {
    let diff = parse_unified_diff(RENAMED_DIFF).unwrap();
    assert_eq!(diff.files.len(), 1);
    let file = &diff.files[0];
    assert_eq!(file.status, FileStatus::Renamed);
    assert_eq!(file.path.to_str().unwrap(), "src/new_name.rs");
    assert_eq!(file.old_path.as_ref().unwrap().to_str().unwrap(), "src/old_name.rs");
    assert!(file.hunks.is_empty());
}

const BINARY_DIFF: &str = r#"diff --git a/assets/logo.png b/assets/logo.png
index 1111111..2222222 100644
Binary files a/assets/logo.png and b/assets/logo.png differ
"#;

#[test]
fn parses_a_binary_file_change() {
    let diff = parse_unified_diff(BINARY_DIFF).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert!(diff.files[0].is_binary);
    assert!(diff.files[0].hunks.is_empty());
}

const MULTI_FILE_DIFF: &str = r#"diff --git a/a.rs b/a.rs
index 1111111..2222222 100644
--- a/a.rs
+++ b/a.rs
@@ -1,1 +1,1 @@
-old a
+new a
diff --git a/b.rs b/b.rs
index 3333333..4444444 100644
--- a/b.rs
+++ b/b.rs
@@ -1,1 +1,1 @@
-old b
+new b
"#;

#[test]
fn parses_multiple_files_in_one_diff() {
    let diff = parse_unified_diff(MULTI_FILE_DIFF).unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(diff.files[0].path.to_str().unwrap(), "a.rs");
    assert_eq!(diff.files[1].path.to_str().unwrap(), "b.rs");
}

#[test]
fn line_content_is_preserved_without_the_marker_prefix() {
    let diff = parse_unified_diff(MODIFIED_DIFF).unwrap();
    let lines = &diff.files[0].hunks[0].lines;
    assert!(lines.contains(&DiffLine::Removed("    println!(\"old\");".to_string())));
    assert!(lines.contains(&DiffLine::Added("    println!(\"new\");".to_string())));
}

#[test]
fn empty_input_yields_empty_diff() {
    let diff = parse_unified_diff("").unwrap();
    assert!(diff.files.is_empty());
}
