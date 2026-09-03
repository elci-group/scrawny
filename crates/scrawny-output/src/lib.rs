use scrawny_analysis::{AnalysisResult, ConcernSummary};
use scrawny_cluster::Cluster;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

pub const REPORT_SCHEMA_VERSION: &str = "2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Jsonl,
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(Format::Text),
            "json" => Ok(Format::Json),
            "jsonl" => Ok(Format::Jsonl),
            other => Err(format!(
                "unsupported output format: {other} (supported: text, json, jsonl)"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryInfo {
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeInfo {
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsInfo {
    pub review_load: scrawny_analysis::ReviewLoad,
    pub cohesion: f64,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub files_changed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceInfo {
    pub average: f64,
    pub low_confidence_changes: usize,
}

/// The stable, versioned machine-readable report shape (§17). `clusters` is
/// real as of Phase II — populated whenever at least one file in the diff
/// was symbol-resolved (see `scrawny_analysis::analyse_with_symbols`), one
/// singleton cluster per change otherwise. `recommendations` remains a
/// reserved, always-empty key until Phase III's decomposition proposals
/// land, so downstream consumers can depend on the schema now.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub version: String,
    pub repository: RepositoryInfo,
    pub change: ChangeInfo,
    pub metrics: MetricsInfo,
    pub concerns: Vec<ConcernSummary>,
    pub clusters: Vec<Cluster>,
    pub recommendations: Vec<serde_json::Value>,
    pub confidence: ConfidenceInfo,
}

pub fn build_report(root: &str, change_description: &str, result: &AnalysisResult) -> Report {
    let confident_changes = result.changes.len();
    let low_confidence_changes = result
        .changes
        .iter()
        .filter(|c| c.classification.confidence < 0.5)
        .count();
    let average = if confident_changes == 0 {
        1.0
    } else {
        result.changes.iter().map(|c| c.classification.confidence).sum::<f64>()
            / confident_changes as f64
    };

    Report {
        version: REPORT_SCHEMA_VERSION.to_string(),
        repository: RepositoryInfo { root: root.to_string() },
        change: ChangeInfo {
            description: change_description.to_string(),
        },
        metrics: MetricsInfo {
            review_load: result.review_load.clone(),
            cohesion: result.cohesion,
            lines_added: result.lines_added,
            lines_removed: result.lines_removed,
            files_changed: result.files_changed,
        },
        concerns: result.concerns.clone(),
        clusters: result.clusters.clone(),
        recommendations: Vec::new(),
        confidence: ConfidenceInfo {
            average,
            low_confidence_changes,
        },
    }
}

pub fn render_json(report: &Report) -> serde_json::Result<String> {
    serde_json::to_string_pretty(report)
}

pub fn render_jsonl(report: &Report) -> serde_json::Result<String> {
    serde_json::to_string(report)
}

pub fn render_text(report: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Scrawny {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(out, "Repository: {}", report.repository.root);
    let _ = writeln!(out, "Change: {}", report.change.description);
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "Review load       {}  {}/100",
        report.metrics.review_load.label(),
        report.metrics.review_load.total
    );
    let _ = writeln!(out, "Cohesion          {:.0}%", report.metrics.cohesion * 100.0);
    let _ = writeln!(
        out,
        "Changed lines     +{} -{}",
        report.metrics.lines_added, report.metrics.lines_removed
    );
    let _ = writeln!(out, "Files             {}", report.metrics.files_changed);
    let _ = writeln!(out, "Concerns          {}", report.concerns.len());
    let _ = writeln!(out, "Clusters          {}", report.clusters.len());
    let _ = writeln!(out);

    if !report.concerns.is_empty() {
        let _ = writeln!(out, "Detected concerns:");
        let _ = writeln!(out);
        for (i, c) in report.concerns.iter().enumerate() {
            let _ = writeln!(
                out,
                "  {}. {:<20} {} LOC ({} change{})",
                i + 1,
                capitalise(c.concern.as_str()),
                c.lines,
                c.count,
                if c.count == 1 { "" } else { "s" }
            );
        }
        let _ = writeln!(out);
    }

    if report.confidence.low_confidence_changes > 0 {
        let _ = writeln!(
            out,
            "{} change(s) classified with low confidence (< 0.5) — treat their concern labels as provisional.",
            report.confidence.low_confidence_changes
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "No automatic changes made.");

    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn render(report: &Report, format: Format) -> Result<String, serde_json::Error> {
    match format {
        Format::Text => Ok(render_text(report)),
        Format::Json => render_json(report),
        Format::Jsonl => render_jsonl(report),
    }
}
