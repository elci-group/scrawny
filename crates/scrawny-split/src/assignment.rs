//! Maps a `Change` (or the symbol it was attributed to) back to the
//! `Cluster` it belongs to — built once from Phase II's own clustering
//! output, never re-derived by guessing.

use scrawny_cluster::Cluster;
use scrawny_core::{Change, ChangeId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct ClusterAssignment {
    change_to_cluster: HashMap<ChangeId, usize>,
    /// (file, symbol name) -> cluster. Unambiguous by construction: every
    /// Change sharing a (file, symbol) touched-symbol pair was merged into
    /// the same cluster by scrawny-cluster's "shared symbol" edge.
    name_to_cluster: HashMap<(PathBuf, String), usize>,
}

impl ClusterAssignment {
    pub fn build(clusters: &[Cluster], changes: &[Change]) -> Self {
        let mut change_to_cluster = HashMap::new();
        for cluster in clusters {
            for id in &cluster.change_ids {
                change_to_cluster.insert(*id, cluster.id);
            }
        }

        let mut name_to_cluster = HashMap::new();
        for change in changes {
            if let Some(name) = change.touched_symbols.first() {
                if let Some(&cluster_id) = change_to_cluster.get(&change.id) {
                    name_to_cluster.insert((change.file.clone(), name.clone()), cluster_id);
                }
            }
        }

        Self { change_to_cluster, name_to_cluster }
    }

    pub fn cluster_for_change(&self, id: ChangeId) -> Option<usize> {
        self.change_to_cluster.get(&id).copied()
    }

    pub fn cluster_for_symbol(&self, file: &Path, name: &str) -> Option<usize> {
        self.name_to_cluster.get(&(file.to_path_buf(), name.to_string())).copied()
    }
}
