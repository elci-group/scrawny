use crate::classify::classify_hunk;
use scrawny_config::Config;
use scrawny_core::{Change, ChangeId, ChangeKind, Diff, FileStatus, Hunk};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Turn a structural `Diff` into classified `Change` records — the point
/// past which the rest of the pipeline never looks at raw diff text again.
///
/// Phase I extracts one `Change` per hunk. Sub-hunk splitting (distinguishing
/// two unrelated edits sharing a hunk) is Phase II AST-aware territory.
pub fn extract_changes(diff: &Diff, config: &Config) -> Vec<Change> {
    let mut changes = Vec::new();

    for file in &diff.files {
        let kind = change_kind(&file.status);

        for (index, hunk) in file.hunks.iter().enumerate() {
            let classification = classify_hunk(&file.path, &file.status, hunk, config);
            changes.push(Change {
                id: change_id(&file.path, index, hunk),
                file: file.path.clone(),
                old_range: hunk.old_range,
                new_range: hunk.new_range,
                kind,
                lines_added: hunk.lines_added(),
                lines_removed: hunk.lines_removed(),
                classification,
            });
        }

        // A renamed/moved file with no hunks (pure rename) still needs a
        // representative Change so it isn't lost from the analysis.
        if file.hunks.is_empty() && matches!(file.status, FileStatus::Renamed) {
            let classification = classify_hunk(
                &file.path,
                &file.status,
                &Hunk {
                    old_range: scrawny_core::Range::new(0, 0),
                    new_range: scrawny_core::Range::new(0, 0),
                    lines: Vec::new(),
                },
                config,
            );
            changes.push(Change {
                id: change_id(&file.path, 0, &placeholder_hunk()),
                file: file.path.clone(),
                old_range: scrawny_core::Range::new(0, 0),
                new_range: scrawny_core::Range::new(0, 0),
                kind,
                lines_added: 0,
                lines_removed: 0,
                classification,
            });
        }
    }

    changes
}

fn placeholder_hunk() -> Hunk {
    Hunk {
        old_range: scrawny_core::Range::new(0, 0),
        new_range: scrawny_core::Range::new(0, 0),
        lines: Vec::new(),
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

fn change_id(path: &std::path::Path, hunk_index: usize, hunk: &Hunk) -> ChangeId {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hunk_index.hash(&mut hasher);
    hunk.old_range.start.hash(&mut hasher);
    hunk.new_range.start.hash(&mut hasher);
    ChangeId(hasher.finish())
}
