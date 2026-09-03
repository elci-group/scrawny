use crate::classify::classify_hunk;
use scrawny_cluster::SymbolTable;
use scrawny_config::Config;
use scrawny_core::{Change, ChangeId, ChangeKind, Diff, DiffLine, FileStatus, Hunk, Range};
use scrawny_parser::{tag_hunk_lines, Symbol};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Turn a structural `Diff` into classified `Change` records — the point
/// past which the rest of the pipeline never looks at raw diff text again.
///
/// One hunk becomes one `Change`, *unless* symbol data is available for the
/// file and the hunk's new-file lines actually span more than one symbol —
/// git's default 3-line diff context routinely merges two unrelated nearby
/// edits into a single hunk, and without splitting those back apart,
/// clustering would never see the seam. See `split_by_symbol`.
pub fn extract_changes(diff: &Diff, config: &Config, symbols: &SymbolTable) -> Vec<Change> {
    let mut changes = Vec::new();

    for file in &diff.files {
        let kind = change_kind(&file.status);
        let file_symbols = symbols.get(&file.path).map(Vec::as_slice);

        for (index, hunk) in file.hunks.iter().enumerate() {
            let groups = file_symbols.map(|syms| split_by_symbol(hunk, syms));

            match groups {
                None => {
                    let classification = classify_hunk(&file.path, &file.status, hunk, config);
                    changes.push(Change {
                        id: change_id(&file.path, index, 0, hunk.old_range, hunk.new_range),
                        file: file.path.clone(),
                        old_range: hunk.old_range,
                        new_range: hunk.new_range,
                        kind,
                        lines_added: hunk.lines_added(),
                        lines_removed: hunk.lines_removed(),
                        classification,
                        touched_symbols: Vec::new(),
                        lines: hunk.lines.clone(),
                    });
                }
                Some(groups) if groups.len() <= 1 => {
                    // Symbol data exists, and every changed line in this hunk
                    // attributes to (at most) one symbol — no seam to split
                    // on, but we now know precisely which symbol it is,
                    // instead of whatever the hunk's context-inflated
                    // new_range happens to overlap.
                    let touched_symbols = groups
                        .into_iter()
                        .next()
                        .and_then(|g| g.key)
                        .into_iter()
                        .collect();
                    let classification = classify_hunk(&file.path, &file.status, hunk, config);
                    changes.push(Change {
                        id: change_id(&file.path, index, 0, hunk.old_range, hunk.new_range),
                        file: file.path.clone(),
                        old_range: hunk.old_range,
                        new_range: hunk.new_range,
                        kind,
                        lines_added: hunk.lines_added(),
                        lines_removed: hunk.lines_removed(),
                        classification,
                        touched_symbols,
                        lines: hunk.lines.clone(),
                    });
                }
                Some(groups) => {
                    for (sub_index, group) in groups.into_iter().enumerate() {
                        let synthetic = Hunk {
                            old_range: group.old_range,
                            new_range: group.new_range,
                            lines: group.lines,
                        };
                        let classification =
                            classify_hunk(&file.path, &file.status, &synthetic, config);
                        changes.push(Change {
                            id: change_id(
                                &file.path,
                                index,
                                sub_index + 1,
                                synthetic.old_range,
                                synthetic.new_range,
                            ),
                            file: file.path.clone(),
                            old_range: synthetic.old_range,
                            new_range: synthetic.new_range,
                            kind,
                            lines_added: synthetic.lines_added(),
                            lines_removed: synthetic.lines_removed(),
                            classification,
                            touched_symbols: group.key.into_iter().collect(),
                            lines: synthetic.lines,
                        });
                    }
                }
            }
        }

        // A renamed/moved file with no hunks (pure rename) still needs a
        // representative Change so it isn't lost from the analysis.
        if file.hunks.is_empty() && matches!(file.status, FileStatus::Renamed) {
            let placeholder = Hunk {
                old_range: Range::new(0, 0),
                new_range: Range::new(0, 0),
                lines: Vec::new(),
            };
            let classification = classify_hunk(&file.path, &file.status, &placeholder, config);
            changes.push(Change {
                id: change_id(&file.path, 0, 0, placeholder.old_range, placeholder.new_range),
                file: file.path.clone(),
                old_range: placeholder.old_range,
                new_range: placeholder.new_range,
                kind,
                lines_added: 0,
                lines_removed: 0,
                classification,
                touched_symbols: Vec::new(),
                lines: Vec::new(),
            });
        }
    }

    changes
}

/// One sub-region of a hunk attributed to a single symbol (or `None` for
/// lines outside every known symbol — comments, blank lines, `use`
/// statements). Never drops a line: every line in the source hunk
/// (context included) ends up in exactly one group, in original order.
struct SymbolGroup {
    key: Option<String>,
    old_range: Range,
    new_range: Range,
    lines: Vec<DiffLine>,
}

/// Group `hunk`'s tagged lines (see `scrawny_parser::tag_hunk_lines`) by
/// symbol, for extraction/clustering purposes. Order-within-group is
/// preserved but groups themselves are *not* a substitute for the flat
/// tagged sequence when true original order matters (see attribution.rs's
/// module docs) — this function is only ever used here, for building
/// `Change` records, never for reconstruction.
fn split_by_symbol(hunk: &Hunk, symbols: &[Symbol]) -> Vec<SymbolGroup> {
    let mut builders: Vec<GroupBuilder> = Vec::new();
    let mut new_cursor = hunk.new_range.start;
    let mut old_cursor = hunk.old_range.start;

    for tagged in tag_hunk_lines(hunk, symbols) {
        let builder = find_or_insert(&mut builders, &tagged.key);
        match &tagged.line {
            DiffLine::Added(_) => {
                builder.new_lines.push(new_cursor);
                new_cursor += 1;
            }
            DiffLine::Removed(_) => {
                builder.old_lines.push(old_cursor);
                old_cursor += 1;
            }
            DiffLine::Context(_) => {
                new_cursor += 1;
                old_cursor += 1;
            }
        }
        builder.lines.push(tagged.line);
    }

    builders
        .into_iter()
        // A group with only context lines (no real edit) isn't a change —
        // this happens now that context lines are retained per-group for
        // reconstruction: a "None" bucket can pick up surrounding context
        // even when nothing outside a symbol actually changed.
        .filter(|b| !b.new_lines.is_empty() || !b.old_lines.is_empty())
        .map(|b| SymbolGroup {
            key: b.key,
            new_range: range_of(&b.new_lines, hunk.new_range),
            old_range: range_of(&b.old_lines, hunk.old_range),
            lines: b.lines,
        })
        .collect()
}

fn find_or_insert<'a>(
    builders: &'a mut Vec<GroupBuilder>,
    key: &Option<String>,
) -> &'a mut GroupBuilder {
    if let Some(pos) = builders.iter().position(|b| &b.key == key) {
        &mut builders[pos]
    } else {
        builders.push(GroupBuilder {
            key: key.clone(),
            new_lines: Vec::new(),
            old_lines: Vec::new(),
            lines: Vec::new(),
        });
        builders.last_mut().unwrap()
    }
}

struct GroupBuilder {
    key: Option<String>,
    new_lines: Vec<usize>,
    old_lines: Vec<usize>,
    lines: Vec<DiffLine>,
}

fn range_of(lines: &[usize], fallback: Range) -> Range {
    match (lines.iter().min(), lines.iter().max()) {
        (Some(&start), Some(&end)) => Range::new(start, end),
        _ => fallback,
    }
}

fn change_kind(status: &FileStatus) -> ChangeKind {
    match status {
        FileStatus::Added => ChangeKind::Added,
        FileStatus::Removed => ChangeKind::Removed,
        FileStatus::Modified => ChangeKind::Modified,
        FileStatus::Renamed => ChangeKind::Renamed,
        FileStatus::Copied => ChangeKind::Added,
        FileStatus::Unknown => ChangeKind::Unknown,
    }
}

fn change_id(
    path: &std::path::Path,
    hunk_index: usize,
    sub_index: usize,
    old_range: Range,
    new_range: Range,
) -> ChangeId {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hunk_index.hash(&mut hasher);
    sub_index.hash(&mut hasher);
    old_range.start.hash(&mut hasher);
    new_range.start.hash(&mut hasher);
    ChangeId(hasher.finish())
}
