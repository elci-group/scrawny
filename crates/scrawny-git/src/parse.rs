//! Unified-diff parsing for `git diff` output.
//!
//! Deliberately narrow: this parses the subset of unified-diff syntax git
//! actually emits, not the general patch format. It never shells out again
//! per-file — the caller hands over one full `git diff` stdout blob and gets
//! back a structured `Diff`.

use scrawny_core::{Diff, DiffLine, FileDiff, FileStatus, Hunk, Range, ScrawnyError};
use std::path::PathBuf;

pub fn parse_unified_diff(text: &str) -> Result<Diff, ScrawnyError> {
    let mut files = Vec::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        if !line.starts_with("diff --git ") {
            continue;
        }

        let (mut old_path, mut new_path) = parse_diff_git_header(line)?;
        let mut status = FileStatus::Modified;
        let mut is_binary = false;
        let mut old_path_renamed: Option<PathBuf> = None;
        let mut hunks = Vec::new();

        // Consume the extended header lines until we hit a hunk or the next file.
        while let Some(&next) = lines.peek() {
            if next.starts_with("@@ ") || next.starts_with("diff --git ") {
                break;
            }
            let next = lines.next().unwrap();

            if let Some(rest) = next.strip_prefix("rename from ") {
                old_path_renamed = Some(PathBuf::from(rest));
                status = FileStatus::Renamed;
            } else if let Some(rest) = next.strip_prefix("rename to ") {
                new_path = PathBuf::from(rest);
                status = FileStatus::Renamed;
            } else if let Some(rest) = next.strip_prefix("copy from ") {
                old_path_renamed = Some(PathBuf::from(rest));
                status = FileStatus::Copied;
            } else if let Some(rest) = next.strip_prefix("copy to ") {
                new_path = PathBuf::from(rest);
                status = FileStatus::Copied;
            } else if next.starts_with("new file mode") {
                status = FileStatus::Added;
            } else if next.starts_with("deleted file mode") {
                status = FileStatus::Removed;
            } else if next.starts_with("Binary files") || next.starts_with("GIT binary patch") {
                is_binary = true;
            } else if next.starts_with("--- ") {
                old_path = parse_path_side(next, "--- ").or(old_path);
            } else if next.starts_with("+++ ") {
                new_path = parse_path_side(next, "+++ ").unwrap_or(new_path);
            }
        }

        if let Some(renamed_from) = &old_path_renamed {
            old_path = Some(renamed_from.clone());
        }

        while let Some(&next) = lines.peek() {
            if !next.starts_with("@@ ") {
                break;
            }
            let header = lines.next().unwrap();
            let (old_range, new_range) = parse_hunk_header(header)?;
            let mut body = Vec::new();

            while let Some(&next) = lines.peek() {
                if next.starts_with("@@ ") || next.starts_with("diff --git ") {
                    break;
                }
                let l = lines.next().unwrap();
                if let Some(content) = l.strip_prefix('+') {
                    body.push(DiffLine::Added(content.to_string()));
                } else if let Some(content) = l.strip_prefix('-') {
                    body.push(DiffLine::Removed(content.to_string()));
                } else if let Some(content) = l.strip_prefix(' ') {
                    body.push(DiffLine::Context(content.to_string()));
                } else if l.starts_with('\\') {
                    // "\ No newline at end of file" — not a content line.
                    continue;
                } else if l.is_empty() {
                    body.push(DiffLine::Context(String::new()));
                } else {
                    // Tolerate unexpected content rather than aborting the whole
                    // parse: treat as context so line accounting stays honest.
                    body.push(DiffLine::Context(l.to_string()));
                }
            }

            hunks.push(Hunk {
                old_range,
                new_range,
                lines: body,
            });
        }

        let path = new_path.clone();
        files.push(FileDiff {
            path,
            old_path: old_path.filter(|p| p != &new_path),
            status,
            is_binary,
            hunks,
        });
    }

    Ok(Diff { files })
}

fn parse_diff_git_header(line: &str) -> Result<(Option<PathBuf>, PathBuf), ScrawnyError> {
    // "diff --git a/path/to/file b/path/to/file"
    let rest = line.strip_prefix("diff --git ").ok_or_else(|| ScrawnyError::InvalidDiff {
        reason: format!("malformed diff header: {line}"),
    })?;

    // Paths can contain spaces; git quotes them with `a/` `b/` prefixes we can
    // anchor on. Find " b/" as the split point, preferring the last occurrence
    // as a heuristic for embedded spaces in the a/ path.
    if let Some(idx) = rest.rfind(" b/") {
        let a = &rest[..idx];
        let b = &rest[idx + 3..];
        let a_path = a.strip_prefix("a/").unwrap_or(a);
        return Ok((Some(PathBuf::from(a_path)), PathBuf::from(b)));
    }

    Err(ScrawnyError::InvalidDiff {
        reason: format!("could not split diff header paths: {line}"),
    })
}

fn parse_path_side(line: &str, prefix: &str) -> Option<PathBuf> {
    let rest = line.strip_prefix(prefix)?;
    let rest = rest.split('\t').next().unwrap_or(rest);
    if rest == "/dev/null" {
        return None;
    }
    let rest = rest.strip_prefix("a/").or_else(|| rest.strip_prefix("b/")).unwrap_or(rest);
    Some(PathBuf::from(rest))
}

fn parse_hunk_header(header: &str) -> Result<(Range, Range), ScrawnyError> {
    // "@@ -old_start,old_count +new_start,new_count @@ optional context"
    let body = header
        .strip_prefix("@@ ")
        .and_then(|s| s.split(" @@").next())
        .ok_or_else(|| ScrawnyError::InvalidDiff {
            reason: format!("malformed hunk header: {header}"),
        })?;

    let mut parts = body.split_whitespace();
    let old = parts.next().ok_or_else(|| ScrawnyError::InvalidDiff {
        reason: format!("malformed hunk header: {header}"),
    })?;
    let new = parts.next().ok_or_else(|| ScrawnyError::InvalidDiff {
        reason: format!("malformed hunk header: {header}"),
    })?;

    Ok((parse_range(old, '-')?, parse_range(new, '+')?))
}

fn parse_range(spec: &str, sign: char) -> Result<Range, ScrawnyError> {
    let spec = spec.strip_prefix(sign).ok_or_else(|| ScrawnyError::InvalidDiff {
        reason: format!("malformed hunk range: {spec}"),
    })?;
    let mut parts = spec.splitn(2, ',');
    let start: usize = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ScrawnyError::InvalidDiff {
            reason: format!("malformed hunk range start: {spec}"),
        })?;
    let count: usize = match parts.next() {
        Some(c) => c.parse().map_err(|_| ScrawnyError::InvalidDiff {
            reason: format!("malformed hunk range count: {spec}"),
        })?,
        None => 1,
    };
    // Git uses a 0 start for an empty (all-added or all-removed) side.
    let end = if count == 0 { start } else { start + count - 1 };
    Ok(Range::new(start, end.max(start)))
}
