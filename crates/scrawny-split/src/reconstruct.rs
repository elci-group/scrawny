//! Reconstructs a file's full content for an arbitrary subset of "applied"
//! clusters, from the original hunks plus per-line symbol attribution.
//!
//! Deliberately conservative: every decision here is driven by data already
//! computed at extraction time (or freshly re-derived the same way), never
//! guessed. The caller (`plan::build_plan`) is expected to validate the
//! final reconstructed state against the real target content before ever
//! presenting a plan — this module does not trust itself to be silently
//! correct on every input (see the module docs on `scrawny_parser::attribution`
//! for why a hunk can't always be treated as one contiguous unit).

use crate::assignment::ClusterAssignment;
use scrawny_core::{Change, DiffLine, FileDiff, Range};
use scrawny_parser::{tag_hunk_lines, Symbol};
use std::collections::HashSet;

/// Reconstruct `file`'s full new-side content given which clusters are
/// "applied" so far. `base_content` is the file's content before any of
/// this diff's hunks (`None` for a file that didn't exist yet — `Added`).
/// Returns `None` only when `file` has no hunks and isn't a rename (nothing
/// to reconstruct); callers handle `Added`/`Removed`/pure-`Renamed` as
/// atomic file operations rather than calling this at all for those.
pub fn reconstruct_modified_or_added(
    file: &FileDiff,
    base_content: Option<&str>,
    file_symbols: Option<&[Symbol]>,
    file_changes: &[&Change],
    assignment: &ClusterAssignment,
    applied: &HashSet<usize>,
) -> String {
    let base_lines: Vec<&str> = base_content.map(|c| c.lines().collect()).unwrap_or_default();
    let mut out = String::new();
    let mut old_cursor = 1usize;

    for hunk in &file.hunks {
        if hunk.old_range.start > old_cursor {
            emit_range(&mut out, &base_lines, old_cursor, hunk.old_range.start.saturating_sub(1));
        }

        let hunk_changes: Vec<&&Change> =
            file_changes.iter().filter(|c| contains(hunk.old_range, c.old_range)).collect();

        let multi_symbol_hunk = file_symbols.filter(|_| hunk_changes.len() > 1);
        if let Some(symbols) = multi_symbol_hunk {
            let tags = tag_hunk_lines(hunk, symbols);
            for tagged in tags {
                let cluster = match &tagged.key {
                    Some(name) => assignment.cluster_for_symbol(&file.path, name),
                    None => hunk_changes
                        .iter()
                        .find(|c| c.touched_symbols.is_empty())
                        .and_then(|c| assignment.cluster_for_change(c.id)),
                };
                let apply_this = cluster.is_some_and(|c| applied.contains(&c));
                emit_line(&mut out, &tagged.line, apply_this);
            }
        } else {
            let cluster = hunk_changes.first().and_then(|c| cluster_of(c, assignment));
            let apply_this = cluster.is_some_and(|c| applied.contains(&c));
            emit_decided(&mut out, &hunk.lines, apply_this);
        }

        old_cursor = if hunk.old_range == Range::new(0, 0) {
            old_cursor
        } else {
            hunk.old_range.end + 1
        };
    }

    if old_cursor <= base_lines.len() {
        emit_range(&mut out, &base_lines, old_cursor, base_lines.len());
    }

    out
}

fn cluster_of(change: &Change, assignment: &ClusterAssignment) -> Option<usize> {
    match change.touched_symbols.first() {
        Some(name) => assignment.cluster_for_symbol(&change.file, name),
        None => assignment.cluster_for_change(change.id),
    }
}

/// Does `outer` fully contain `inner`? Used to find which hunk a
/// (possibly split-out) Change originated from.
fn contains(outer: Range, inner: Range) -> bool {
    inner.start >= outer.start && inner.end <= outer.end
}

fn emit_range(out: &mut String, lines: &[&str], start_1indexed: usize, end_1indexed: usize) {
    if start_1indexed == 0 || start_1indexed > end_1indexed {
        return;
    }
    for line in &lines[(start_1indexed - 1).min(lines.len())..end_1indexed.min(lines.len())] {
        out.push_str(line);
        out.push('\n');
    }
}

fn emit_decided(out: &mut String, lines: &[DiffLine], apply_this: bool) {
    for line in lines {
        emit_line(out, line, apply_this);
    }
}

fn emit_line(out: &mut String, line: &DiffLine, apply_this: bool) {
    match line {
        DiffLine::Context(text) => {
            out.push_str(text);
            out.push('\n');
        }
        DiffLine::Added(text) => {
            if apply_this {
                out.push_str(text);
                out.push('\n');
            }
        }
        DiffLine::Removed(text) => {
            if !apply_this {
                out.push_str(text);
                out.push('\n');
            }
        }
    }
}
