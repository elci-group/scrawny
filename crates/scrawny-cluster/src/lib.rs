//! Phase II semantic clustering: group `Change`s into `Cluster`s using real
//! AST symbol data (scrawny-parser), not textual proximity or shared concern
//! labels alone.
//!
//! Two edges are recognised, both conservative and file-scoped by design:
//!
//! - **same symbol**: two changes were attributed to the same symbol name in
//!   the same file (a function split across two hunks, say — see
//!   `Change::touched_symbols`, computed at extraction time from the
//!   *actual* added/removed lines, not the hunk's context-inflated range).
//! - **dependency**: one touched symbol's body textually mentions another
//!   touched symbol's name, in the *same file*. Cross-file reference
//!   resolution would need real import/type resolution; scoping to one file
//!   keeps every edge honestly evidenced instead of guessing.
//!
//! A change with no resolvable symbol data (non-Rust file, unparseable
//! source, content simply unavailable, or edited lines outside every known
//! symbol) becomes its own singleton cluster — never silently folded into a
//! neighbour on the strength of nothing but adjacency.

use scrawny_core::{Change, ChangeId};
use scrawny_parser::Symbol;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub type SymbolTable = HashMap<PathBuf, Vec<Symbol>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub id: usize,
    pub change_ids: Vec<ChangeId>,
    pub files: Vec<PathBuf>,
    pub symbols: Vec<String>,
    pub lines: usize,
    /// Human-readable justification for every merge this cluster represents
    /// — the "why" behind grouping these changes together (§31).
    pub evidence: Vec<String>,
}

/// Look up a symbol's declared `references` by (file, name) — the only
/// thing clustering still needs from the raw `SymbolTable`; which symbols a
/// change touches comes from `Change::touched_symbols` itself now.
fn references_of<'a>(symbols: &'a SymbolTable, file: &Path, name: &str) -> &'a [String] {
    symbols
        .get(file)
        .and_then(|syms| syms.iter().find(|s| s.name == name))
        .map(|s| s.references.as_slice())
        .unwrap_or(&[])
}

pub fn cluster_changes(changes: &[Change], symbols: &SymbolTable) -> Vec<Cluster> {
    let n = changes.len();
    let mut dsu = DisjointSet::new(n);
    let mut evidence: Vec<(usize, usize, String)> = Vec::new();

    for i in 0..n {
        for j in (i + 1)..n {
            if dsu.same(i, j) {
                continue;
            }
            if changes[i].file != changes[j].file {
                continue;
            }
            if let Some(reason) = relate(&changes[i], &changes[j], symbols) {
                dsu.union(i, j);
                evidence.push((i, j, reason));
            }
        }
    }

    assemble(changes, &dsu, evidence)
}

/// `a` and `b` are already known to be in the same file.
fn relate(a: &Change, b: &Change, symbols: &SymbolTable) -> Option<String> {
    // Same symbol: two hunks (or sub-hunk groups) landing in one definition.
    for x in &a.touched_symbols {
        if b.touched_symbols.contains(x) {
            return Some(format!("shared symbol `{x}`"));
        }
    }
    // Dependency: one side's body mentions the other side's name.
    for x in &a.touched_symbols {
        for y in &b.touched_symbols {
            if references_of(symbols, &a.file, x).iter().any(|r| r == y) {
                return Some(format!("`{x}` references `{y}`"));
            }
            if references_of(symbols, &b.file, y).iter().any(|r| r == x) {
                return Some(format!("`{y}` references `{x}`"));
            }
        }
    }
    None
}

fn assemble(changes: &[Change], dsu: &DisjointSet, evidence: Vec<(usize, usize, String)>) -> Vec<Cluster> {
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..changes.len() {
        groups.entry(dsu.find(i)).or_default().push(i);
    }

    groups
        .into_values()
        .enumerate()
        .map(|(id, members)| {
            let member_set: BTreeSet<usize> = members.iter().copied().collect();
            let mut files = BTreeSet::new();
            let mut symbol_names = BTreeSet::new();
            let mut lines = 0usize;
            for &idx in &members {
                files.insert(changes[idx].file.clone());
                lines += changes[idx].lines_changed();
                for name in &changes[idx].touched_symbols {
                    symbol_names.insert(name.clone());
                }
            }
            let cluster_evidence = evidence
                .iter()
                .filter(|(i, j, _)| member_set.contains(i) && member_set.contains(j))
                .map(|(_, _, reason)| reason.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();

            Cluster {
                id,
                change_ids: members.iter().map(|&i| changes[i].id).collect(),
                files: files.into_iter().collect(),
                symbols: symbol_names.into_iter().collect(),
                lines,
                evidence: cluster_evidence,
            }
        })
        .collect()
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }

    fn find(&self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        root
    }

    fn same(&self, a: usize, b: usize) -> bool {
        self.find(a) == self.find(b)
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scrawny_core::{ChangeKind, Classification, Concern, Range};
    use scrawny_parser::SymbolKind;
    use std::path::PathBuf;

    /// `touched` is what extraction would have attributed to this change's
    /// actual added/removed lines — clustering no longer looks at ranges at
    /// all, so a fixed dummy range is fine here.
    fn change(id: u64, file: &str, lines: usize, touched: &[&str]) -> Change {
        Change {
            id: ChangeId(id),
            file: PathBuf::from(file),
            old_range: Range::new(1, 1),
            new_range: Range::new(1, 1),
            kind: ChangeKind::Modified,
            lines_added: lines,
            lines_removed: 0,
            classification: Classification::new(Concern::Behavioural, 0.7),
            touched_symbols: touched.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn symbol(name: &str, references: &[&str]) -> Symbol {
        Symbol {
            name: name.to_string(),
            kind: SymbolKind::Function,
            range: Range::new(1, 1),
            references: references.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn changes_with_no_symbol_data_stay_singletons() {
        let changes = vec![change(1, "a.rs", 3, &[]), change(2, "a.rs", 3, &[])];
        let clusters = cluster_changes(&changes, &SymbolTable::new());
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn changes_touching_the_same_symbol_merge() {
        let changes = vec![
            change(1, "a.rs", 1, &["do_work"]),
            change(2, "a.rs", 1, &["do_work"]),
        ];
        let mut symbols = SymbolTable::new();
        symbols.insert(PathBuf::from("a.rs"), vec![symbol("do_work", &[])]);
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].lines, 2);
        assert!(clusters[0].evidence.iter().any(|e| e.contains("do_work")));
    }

    #[test]
    fn dependent_symbols_in_the_same_file_merge() {
        let changes = vec![
            change(1, "a.rs", 1, &["main"]),
            change(2, "a.rs", 1, &["helper"]),
        ];
        let mut symbols = SymbolTable::new();
        symbols.insert(
            PathBuf::from("a.rs"),
            vec![symbol("main", &["helper"]), symbol("helper", &[])],
        );
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].evidence.iter().any(|e| e.contains("main") && e.contains("helper")));
    }

    #[test]
    fn unrelated_symbols_in_the_same_file_stay_separate() {
        let changes = vec![
            change(1, "a.rs", 1, &["alpha"]),
            change(2, "a.rs", 1, &["beta"]),
        ];
        let mut symbols = SymbolTable::new();
        symbols.insert(
            PathBuf::from("a.rs"),
            vec![symbol("alpha", &[]), symbol("beta", &[])],
        );
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn a_change_touching_an_outer_container_does_not_spuriously_link_unrelated_siblings() {
        // Regression test: two sibling methods of the same impl block are
        // unrelated. Neither change touches the container symbol itself —
        // this is what extraction now guarantees (innermost-only
        // attribution), and clustering must not go looking for container
        // overlap on its own.
        let changes = vec![
            change(1, "a.rs", 1, &["Widget::render"]),
            change(2, "a.rs", 1, &["format_report"]),
        ];
        let mut symbols = SymbolTable::new();
        symbols.insert(
            PathBuf::from("a.rs"),
            vec![
                symbol("Widget", &[]),
                symbol("Widget::render", &[]),
                symbol("format_report", &[]),
            ],
        );
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn cross_file_name_collisions_do_not_merge() {
        let changes = vec![change(1, "a.rs", 1, &["new"]), change(2, "b.rs", 1, &["new"])];
        let mut symbols = SymbolTable::new();
        symbols.insert(PathBuf::from("a.rs"), vec![symbol("new", &[])]);
        symbols.insert(PathBuf::from("b.rs"), vec![symbol("new", &[])]);
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn transitive_merges_form_one_cluster() {
        // a -> b -> c should end up as a single cluster via union-find.
        let changes = vec![
            change(1, "a.rs", 1, &["a"]),
            change(2, "a.rs", 1, &["b"]),
            change(3, "a.rs", 1, &["c"]),
        ];
        let mut symbols = SymbolTable::new();
        symbols.insert(
            PathBuf::from("a.rs"),
            vec![symbol("a", &["b"]), symbol("b", &["c"]), symbol("c", &[])],
        );
        let clusters = cluster_changes(&changes, &symbols);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].change_ids.len(), 3);
    }
}
