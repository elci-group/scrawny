use scrawny_core::{Change, Concern};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Version tag for the scoring formula (§11: "should remain configurable and
/// versioned"). Bump when the weights or shape below change so historical
/// reports stay interpretable.
pub const SCORE_FORMULA_VERSION: &str = "1";

/// Review-load breakdown. Deliberately multi-dimensional (§11) — never
/// collapse to `lines_changed` alone. Phase I approximates
/// `dependency_crossing` and `abstraction_distance` as zero: both need the
/// AST-level dependency graph that arrives in Phase II, and pretending
/// otherwise would misrepresent confidence in the score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewLoad {
    pub total: u32,
    pub size: f64,
    pub concern_multiplicity: f64,
    pub file_dispersion: f64,
    pub mechanical_noise: f64,
    pub behavioural_density: f64,
}

impl ReviewLoad {
    pub fn label(&self) -> &'static str {
        match self.total {
            0..=29 => "LOW",
            30..=59 => "MODERATE",
            60..=79 => "HIGH",
            _ => "SEVERE",
        }
    }
}

pub fn compute_review_load(changes: &[Change]) -> ReviewLoad {
    if changes.is_empty() {
        return ReviewLoad {
            total: 0,
            size: 0.0,
            concern_multiplicity: 0.0,
            file_dispersion: 0.0,
            mechanical_noise: 0.0,
            behavioural_density: 0.0,
        };
    }

    let total_lines: usize = changes.iter().map(Change::lines_changed).sum();
    let files: std::collections::BTreeSet<_> = changes.iter().map(|c| c.file.clone()).collect();
    let concerns: std::collections::BTreeSet<_> =
        changes.iter().map(|c| c.classification.concern.as_str()).collect();

    let mechanical_lines: usize = changes
        .iter()
        .filter(|c| c.classification.concern == Concern::Mechanical)
        .map(Change::lines_changed)
        .sum();
    let behavioural_lines: usize = changes
        .iter()
        .filter(|c| c.classification.concern == Concern::Behavioural)
        .map(Change::lines_changed)
        .sum();

    let size = (total_lines as f64 / 10.0).min(40.0);
    let concern_multiplicity = ((concerns.len().saturating_sub(1)) as f64 * 8.0).min(25.0);
    let file_dispersion = ((files.len().saturating_sub(1)) as f64 * 3.0).min(15.0);
    let mechanical_noise = if total_lines == 0 {
        0.0
    } else {
        (mechanical_lines as f64 / total_lines as f64 * 15.0).min(15.0)
    };
    let behavioural_density = if total_lines == 0 {
        0.0
    } else {
        (behavioural_lines as f64 / total_lines as f64 * 20.0).min(20.0)
    };

    let total = (size + concern_multiplicity + file_dispersion + mechanical_noise + behavioural_density)
        .round()
        .clamp(0.0, 100.0) as u32;

    ReviewLoad {
        total,
        size,
        concern_multiplicity,
        file_dispersion,
        mechanical_noise,
        behavioural_density,
    }
}

/// Fraction of changed lines belonging to the single largest concern.
/// A one-concern change scores 1.0; an evenly five-way-split change scores
/// 0.2. This is the Phase I stand-in for full cluster cohesion (§12), which
/// needs the dependency graph from Phase II to measure properly.
pub fn compute_cohesion(changes: &[Change]) -> f64 {
    if changes.is_empty() {
        return 1.0;
    }

    let total_lines: usize = changes.iter().map(Change::lines_changed).sum();
    if total_lines == 0 {
        return 1.0;
    }

    let mut by_concern: BTreeMap<&'static str, usize> = BTreeMap::new();
    for change in changes {
        *by_concern
            .entry(change.classification.concern.as_str())
            .or_insert(0) += change.lines_changed();
    }

    let largest = by_concern.values().copied().max().unwrap_or(0);
    largest as f64 / total_lines as f64
}

pub fn concern_summary(changes: &[Change]) -> Vec<(Concern, usize, usize)> {
    let mut by_concern: BTreeMap<&'static str, (Concern, usize, usize)> = BTreeMap::new();
    for change in changes {
        let entry = by_concern
            .entry(change.classification.concern.as_str())
            .or_insert((change.classification.concern, 0, 0));
        entry.1 += change.lines_changed();
        entry.2 += 1;
    }
    let mut out: Vec<_> = by_concern.into_values().collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.1));
    out
}
