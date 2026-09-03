//! Per-line symbol attribution for a diff hunk — the single source of truth
//! both extraction-time grouping (scrawny-analysis, which turns a hunk
//! spanning multiple symbols into multiple `Change`s) and Phase III commit
//! reconstruction (scrawny-split, which needs the *true original order* of
//! every line, context included, to safely reassemble a file for an
//! arbitrary subset of applied clusters) build on.
//!
//! Reconstruction in particular cannot go through grouped/reordered output:
//! if the same symbol is touched twice with an unrelated symbol's edit
//! sandwiched between (rare, but possible within one hunk's context
//! window), grouping by symbol produces a non-contiguous subsequence for
//! that symbol. Grouped output is fine for clustering (order doesn't matter
//! there) but would silently misplace lines if used to rebuild a file.
//! `tag_hunk_lines` keeps every line in its real position; only the tag
//! changes meaning depending on what the caller does with it.

use crate::Symbol;
use scrawny_core::{DiffLine, Hunk};

/// One line of a hunk together with the symbol it was attributed to (the
/// innermost symbol whose range contains the line's new-file position;
/// `None` for lines outside every known symbol — comments, blank lines,
/// `use` statements, or anything ahead of the first item).
#[derive(Debug, Clone)]
pub struct TaggedLine {
    pub line: DiffLine,
    pub key: Option<String>,
}

/// Tag every line in `hunk` (context included) in original order. A removed
/// line is attributed to the symbol at the new-file cursor position it was
/// removed *at* — the nearest defensible attribution without old-file
/// symbol data.
pub fn tag_hunk_lines(hunk: &Hunk, symbols: &[Symbol]) -> Vec<TaggedLine> {
    let mut new_cursor = hunk.new_range.start;
    let mut out = Vec::with_capacity(hunk.lines.len());

    for line in &hunk.lines {
        let key = innermost_symbol_at(new_cursor, symbols).map(|s| s.name.clone());
        out.push(TaggedLine { line: line.clone(), key });
        match line {
            DiffLine::Added(_) | DiffLine::Context(_) => new_cursor += 1,
            DiffLine::Removed(_) => {}
        }
    }

    out
}

pub fn innermost_symbol_at(line_no: usize, symbols: &[Symbol]) -> Option<&Symbol> {
    symbols
        .iter()
        .filter(|s| s.range.start <= line_no && line_no <= s.range.end)
        .min_by_key(|s| s.range.end.saturating_sub(s.range.start))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SymbolKind;
    use scrawny_core::Range;

    fn symbol(name: &str, start: usize, end: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            range: Range::new(start, end),
            references: Vec::new(),
        }
    }

    #[test]
    fn tags_preserve_original_line_order_including_context() {
        let hunk = Hunk {
            old_range: Range::new(1, 3),
            new_range: Range::new(1, 3),
            lines: vec![
                DiffLine::Context("fn alpha() {".to_string()),
                DiffLine::Removed("    old();".to_string()),
                DiffLine::Added("    new();".to_string()),
                DiffLine::Context("}".to_string()),
            ],
        };
        let symbols = vec![symbol("alpha", 1, 3)];
        let tags = tag_hunk_lines(&hunk, &symbols);

        assert_eq!(tags.len(), 4);
        assert!(tags.iter().all(|t| t.key.as_deref() == Some("alpha")));
        // Original order preserved exactly, including context lines.
        assert!(matches!(tags[0].line, DiffLine::Context(_)));
        assert!(matches!(tags[1].line, DiffLine::Removed(_)));
        assert!(matches!(tags[2].line, DiffLine::Added(_)));
        assert!(matches!(tags[3].line, DiffLine::Context(_)));
    }

    #[test]
    fn interleaved_symbols_keep_true_order_not_grouped_order() {
        // alpha, beta, alpha again — a case grouping-by-symbol would have to
        // reorder to make contiguous; tagging must not reorder anything.
        let hunk = Hunk {
            old_range: Range::new(1, 5),
            new_range: Range::new(1, 5),
            lines: vec![
                DiffLine::Added("a1".to_string()),
                DiffLine::Added("b1".to_string()),
                DiffLine::Added("a2".to_string()),
            ],
        };
        let symbols = vec![symbol("alpha", 1, 1), symbol("beta", 2, 2), symbol("alpha2", 3, 3)];
        // Reuse "alpha" name isn't the point here; use distinct ranges per line.
        let tags = tag_hunk_lines(&hunk, &symbols);
        let keys: Vec<_> = tags.iter().map(|t| t.key.clone()).collect();
        assert_eq!(
            keys,
            vec![Some("alpha".to_string()), Some("beta".to_string()), Some("alpha2".to_string())]
        );
    }
}
