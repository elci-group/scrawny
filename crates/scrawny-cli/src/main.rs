mod cli;
mod update;

use clap::Parser;
use cli::{Cli, Command};
use scrawny_analysis::{AnalysisResult, SymbolTable};
use scrawny_config::Config;
use scrawny_core::{Diff, FileStatus};
use scrawny_git::{diff_from_text, DiffSpec, GitCliRepository, Repository, SplitWorktree};
use scrawny_output::{build_report, render, Format};
use scrawny_split::{FileChange, PlanInputs};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let format = match Format::from_str(&cli.format) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let command = cli.command.unwrap_or(Command::Analyse { staged: false });

    match run(command, &cli.path, format) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(command: Command, path: &Path, format: Format) -> anyhow::Result<ExitCode> {
    match command {
        Command::Analyse { staged } => {
            let (root, description, diff) = load_repo_diff(path, staged)?;
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let symbols = build_symbol_table(&root, &diff);
            let result = scrawny_analysis::analyse_with_symbols(&diff, &config, &symbols);
            print_report(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Explain { staged } => {
            let (root, description, diff) = load_repo_diff(path, staged)?;
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let symbols = build_symbol_table(&root, &diff);
            let result = scrawny_analysis::analyse_with_symbols(&diff, &config, &symbols);
            print_explain(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Suggest { staged } => {
            let (root, description, diff) = load_repo_diff(path, staged)?;
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let symbols = build_symbol_table(&root, &diff);
            let result = scrawny_analysis::analyse_with_symbols(&diff, &config, &symbols);
            print_suggest(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Split { staged, discard_worktree } => {
            cmd_split(path, staged, discard_worktree, format)
        }
        Command::Check { staged, stdin, file } => {
            let (root, diff) = if stdin || file.is_some() {
                (path.to_path_buf(), load_standalone_diff(stdin, file.as_deref())?)
            } else {
                let (root, _description, diff) = load_repo_diff(path, staged)?;
                (root, diff)
            };
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let symbols = build_symbol_table(&root, &diff);
            let result = scrawny_analysis::analyse_with_symbols(&diff, &config, &symbols);
            let passed = print_check(&result, &config, format)?;
            Ok(if passed { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Command::Diff { stdin, file } => {
            let diff = load_standalone_diff(stdin, file.as_deref())?;
            let config = Config::load(path).map_err(anyhow::Error::from)?;
            let symbols = build_symbol_table(path, &diff);
            let result = scrawny_analysis::analyse_with_symbols(&diff, &config, &symbols);
            print_report(path, "supplied diff", &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Update => {
            update::run()?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Resolve symbol data for every Rust file in `diff` from `root`'s current
/// working tree. Best-effort by design: a file that can't be read (common
/// for a stdin-supplied diff whose repo isn't checked out to the matching
/// ref) is silently skipped — that's the expected degrade-to-Phase-I path,
/// not an error. A file that *can* be read but fails to parse is surfaced
/// as a warning, since that's genuinely unexpected.
fn build_symbol_table(root: &Path, diff: &Diff) -> SymbolTable {
    let mut table = SymbolTable::new();
    for file in &diff.files {
        if file.is_binary || file.status == FileStatus::Removed {
            continue;
        }
        if file.path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(root.join(&file.path)) else {
            continue;
        };
        match scrawny_parser::parse_rust_symbols(&file.path, &content) {
            Ok(symbols) => {
                table.insert(file.path.clone(), symbols);
            }
            Err(error) => {
                eprintln!(
                    "warning: {error}; continuing without symbol data for {}",
                    file.path.display()
                );
            }
        }
    }
    table
}

fn cmd_split(path: &Path, staged: bool, discard_worktree: bool, format: Format) -> anyhow::Result<ExitCode> {
    let repo = GitCliRepository::discover(path).map_err(anyhow::Error::from)?;
    let root = repo.root().to_path_buf();
    let spec = if staged { DiffSpec::Staged } else { DiffSpec::HeadToWorkingTree };
    let raw_diff = repo.diff(spec).map_err(anyhow::Error::from)?;
    let config = Config::load(&root).map_err(anyhow::Error::from)?;

    let binary_count = raw_diff.files.iter().filter(|f| f.is_binary).count();
    let diff = Diff { files: raw_diff.files.into_iter().filter(|f| !f.is_binary).collect() };

    let symbols = build_symbol_table(&root, &diff);
    let changes = scrawny_analysis::extract::extract_changes(&diff, &config, &symbols);
    let clusters = scrawny_cluster::cluster_changes(&changes, &symbols);

    if binary_count > 0 {
        eprintln!("note: {binary_count} binary file(s) excluded from decomposition (unaffected by this split)");
    }

    if clusters.len() <= 1 {
        println!("Everything in this change is one connected unit — nothing to split.");
        return Ok(ExitCode::SUCCESS);
    }

    let (base_content, target_content) = read_content_for_split(&repo, &root, &diff, staged)?;
    let inputs = PlanInputs { diff: &diff, base_content: &base_content, target_content: &target_content, symbols: &symbols };

    let plan = match scrawny_split::build_plan(&inputs, &clusters, &changes) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("error: could not safely construct a decomposition for this change: {error}");
            eprintln!("No commits were made. Your repository is unchanged.");
            return Ok(ExitCode::FAILURE);
        }
    };

    let branch = format!("scrawny/split-{}", short_id());
    let worktree = SplitWorktree::create(&root, "HEAD", &branch).map_err(anyhow::Error::from)?;

    let mut shas = Vec::with_capacity(plan.commits.len());
    for commit in &plan.commits {
        for file_change in &commit.files {
            match file_change {
                FileChange::Write { path, content } => worktree.write_file(path, content).map_err(anyhow::Error::from)?,
                FileChange::Delete { path } => worktree.delete_file(path).map_err(anyhow::Error::from)?,
            }
        }
        let sha = worktree.commit_all(&commit.message).map_err(anyhow::Error::from)?;
        shas.push(sha);
    }

    let clean = worktree.is_clean().map_err(anyhow::Error::from)?;
    if !clean {
        eprintln!("error: worktree was not clean after committing every proposed change — refusing to present this plan");
        worktree.remove().ok();
        return Ok(ExitCode::FAILURE);
    }

    print_split_report(&root, &branch, worktree.path(), &plan, &shas, format)?;

    if discard_worktree {
        worktree.remove().map_err(anyhow::Error::from)?;
    }

    Ok(ExitCode::SUCCESS)
}

/// Base ("before") content for every `Modified` file, and target ("after")
/// content for every non-`Removed` file — read from the index when
/// `staged`, otherwise from the working tree, matching whichever diff spec
/// produced `diff` in the first place.
fn read_content_for_split(
    repo: &GitCliRepository,
    root: &Path,
    diff: &Diff,
    staged: bool,
) -> anyhow::Result<(HashMap<PathBuf, String>, HashMap<PathBuf, String>)> {
    let mut base_content = HashMap::new();
    let mut target_content = HashMap::new();

    for file in &diff.files {
        if file.status == FileStatus::Modified {
            if let Some(content) = repo.show_file("HEAD", &file.path).map_err(anyhow::Error::from)? {
                base_content.insert(file.path.clone(), content);
            }
        }
        if file.status != FileStatus::Removed {
            let content = if staged {
                repo.show_file("", &file.path).map_err(anyhow::Error::from)?
            } else {
                std::fs::read_to_string(root.join(&file.path)).ok()
            };
            if let Some(content) = content {
                target_content.insert(file.path.clone(), content);
            }
        }
    }

    Ok((base_content, target_content))
}

fn short_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{nanos:x}")
}

fn print_split_report(
    root: &Path,
    branch: &str,
    worktree_path: &Path,
    plan: &scrawny_split::SplitPlan,
    shas: &[String],
    format: Format,
) -> anyhow::Result<()> {
    match format {
        Format::Json | Format::Jsonl => {
            #[derive(serde::Serialize)]
            struct CommitReport<'a> {
                sha: &'a str,
                message: &'a str,
                evidence: &'a [String],
                lines_added: usize,
                lines_removed: usize,
                files: usize,
            }
            #[derive(serde::Serialize)]
            struct SplitReport<'a> {
                repository: String,
                branch: &'a str,
                worktree: String,
                commits: Vec<CommitReport<'a>>,
            }
            let report = SplitReport {
                repository: root.display().to_string(),
                branch,
                worktree: worktree_path.display().to_string(),
                commits: plan
                    .commits
                    .iter()
                    .zip(shas)
                    .map(|(c, sha)| CommitReport {
                        sha,
                        message: &c.message,
                        evidence: &c.evidence,
                        lines_added: c.lines_added,
                        lines_removed: c.lines_removed,
                        files: c.files.len(),
                    })
                    .collect(),
            };
            let text = if format == Format::Json {
                serde_json::to_string_pretty(&report)?
            } else {
                serde_json::to_string(&report)?
            };
            println!("{text}");
        }
        Format::Text => {
            println!("Constructed and validated {} commit(s) on branch {branch}:", plan.commits.len());
            println!();
            for (commit, sha) in plan.commits.iter().zip(shas) {
                let short_sha = &sha[..sha.len().min(10)];
                let subject = commit.message.lines().next().unwrap_or(&commit.message);
                println!(
                    "  {short_sha}  {subject}  (+{} -{}, {} file(s))",
                    commit.lines_added,
                    commit.lines_removed,
                    commit.files.len()
                );
                for reason in &commit.evidence {
                    println!("            {reason}");
                }
            }
            println!();
            println!("Worktree: {}", worktree_path.display());
            println!("Branch:   {branch}  (reachable from this repo — `git log {branch}`)");
            println!();
            println!("Your current branch and working tree are untouched. Nothing was pushed.");
        }
    }
    Ok(())
}

fn load_repo_diff(path: &Path, staged: bool) -> anyhow::Result<(PathBuf, String, Diff)> {
    let repo = GitCliRepository::discover(path).map_err(anyhow::Error::from)?;
    let root = repo.root().to_path_buf();
    let spec = if staged { DiffSpec::Staged } else { DiffSpec::HeadToWorkingTree };
    let description = if staged { "staged changes" } else { "working tree" };
    let diff = repo.diff(spec).map_err(anyhow::Error::from)?;
    Ok((root, description.to_string(), diff))
}

fn load_standalone_diff(stdin: bool, file: Option<&std::path::Path>) -> anyhow::Result<Diff> {
    let text = if stdin {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else if let Some(file) = file {
        std::fs::read_to_string(file)?
    } else {
        return Err(anyhow::anyhow!(
            "no diff supplied: pass --stdin or a file path, e.g. `git diff | scrawny diff --stdin`"
        ));
    };

    diff_from_text(&text).map_err(anyhow::Error::from)
}

fn print_report(
    root: &std::path::Path,
    description: &str,
    result: &AnalysisResult,
    format: Format,
) -> anyhow::Result<()> {
    let report = build_report(&root.display().to_string(), description, result);
    println!("{}", render(&report, format)?);
    Ok(())
}

fn print_explain(
    root: &std::path::Path,
    description: &str,
    result: &AnalysisResult,
    format: Format,
) -> anyhow::Result<()> {
    match format {
        Format::Json | Format::Jsonl => {
            let report = build_report(&root.display().to_string(), description, result);
            #[derive(serde::Serialize)]
            struct ExplainReport<'a> {
                #[serde(flatten)]
                report: &'a scrawny_output::Report,
                changes: &'a [scrawny_core::Change],
            }
            let explain = ExplainReport { report: &report, changes: &result.changes };
            let text = if format == Format::Json {
                serde_json::to_string_pretty(&explain)?
            } else {
                serde_json::to_string(&explain)?
            };
            println!("{text}");
        }
        Format::Text => {
            let report = build_report(&root.display().to_string(), description, result);
            println!("{}", render(&report, Format::Text)?);

            if !result.changes.is_empty() {
                println!("Evidence:");
                println!();
                for summary in &result.concerns {
                    let matching: Vec<_> = result
                        .changes
                        .iter()
                        .filter(|c| c.classification.concern == summary.concern)
                        .collect();
                    println!(
                        "  {} ({} LOC, confidence avg {:.2}):",
                        capitalise(summary.concern.as_str()),
                        summary.lines,
                        matching.iter().map(|c| c.classification.confidence).sum::<f64>()
                            / matching.len().max(1) as f64
                    );
                    for change in matching.iter().take(5) {
                        println!(
                            "    {}:{}-{} (+{} -{})",
                            change.file.display(),
                            change.new_range.start,
                            change.new_range.end,
                            change.lines_added,
                            change.lines_removed
                        );
                    }
                    if matching.len() > 5 {
                        println!("    ... and {} more", matching.len() - 5);
                    }
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn print_suggest(
    root: &std::path::Path,
    description: &str,
    result: &AnalysisResult,
    format: Format,
) -> anyhow::Result<()> {
    // A decomposition proposal is only meaningful when clustering actually
    // found more than one independent unit of work. One cluster covering
    // everything means the change is already a single coherent thing — per
    // §32, the goal is easier review, not maximal fragmentation for its own
    // sake, so we don't manufacture a split that isn't there.
    let decomposable = result.clusters.len() > 1;
    let total_lines = (result.lines_added + result.lines_removed).max(1);
    let mut ranked: Vec<_> = result.clusters.iter().collect();
    ranked.sort_by_key(|c| std::cmp::Reverse(c.lines));

    match format {
        Format::Json | Format::Jsonl => {
            let report = build_report(&root.display().to_string(), description, result);
            #[derive(serde::Serialize)]
            struct SuggestReport<'a> {
                #[serde(flatten)]
                report: &'a scrawny_output::Report,
                decomposition_available: bool,
            }
            let out = SuggestReport { report: &report, decomposition_available: decomposable };
            let text = if format == Format::Json {
                serde_json::to_string_pretty(&out)?
            } else {
                serde_json::to_string(&out)?
            };
            println!("{text}");
        }
        Format::Text => {
            let report = build_report(&root.display().to_string(), description, result);
            println!("{}", render(&report, Format::Text)?);

            if !decomposable {
                if result.clusters.len() == 1 {
                    println!(
                        "Everything in this change is one connected unit ({} LOC across {} file(s)) — no decomposition to propose.",
                        result.clusters[0].lines,
                        result.clusters[0].files.len()
                    );
                } else {
                    println!("Nothing to decompose.");
                }
                return Ok(());
            }

            println!("Recommended decomposition:");
            println!();
            let labels = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
            for (i, cluster) in ranked.iter().enumerate() {
                let label = labels.chars().nth(i).unwrap_or('?');
                let files = cluster
                    .files
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let what = if cluster.symbols.is_empty() {
                    files
                } else {
                    format!("{files}: {}", cluster.symbols.join(", "))
                };
                println!("  {label}  {what} — {} LOC", cluster.lines);
                for reason in &cluster.evidence {
                    println!("       {reason}");
                }
            }
            println!();

            let largest_cluster_lines = ranked.first().map(|c| c.lines).unwrap_or(0);
            let reduction = 100.0 - (largest_cluster_lines as f64 / total_lines as f64 * 100.0);
            println!("Expected review-load reduction: ~{reduction:.0}% (largest single reviewable unit vs. the whole change)");
        }
    }
    Ok(())
}

fn print_check(result: &AnalysisResult, config: &Config, format: Format) -> anyhow::Result<bool> {
    let policy = &config.policy;
    let exempt_documentation = policy.exceptions.documentation;
    let exempt_generated = policy.exceptions.generated;

    let counted_concerns: usize = result
        .concerns
        .iter()
        .filter(|c| {
            !(exempt_documentation && c.concern == scrawny_core::Concern::Documentation)
                && !(exempt_generated && c.concern == scrawny_core::Concern::Mechanical)
        })
        .count();

    let mut reasons = Vec::new();
    if result.review_load.total > policy.max_review_load {
        reasons.push(format!(
            "review load {}/100 exceeds policy maximum {}",
            result.review_load.total, policy.max_review_load
        ));
    }
    if result.cohesion < policy.min_cohesion {
        reasons.push(format!(
            "cohesion {:.0}% is below policy minimum {:.0}%",
            result.cohesion * 100.0,
            policy.min_cohesion * 100.0
        ));
    }
    if counted_concerns > policy.max_concerns {
        reasons.push(format!(
            "{counted_concerns} independent concerns detected, policy maximum is {}",
            policy.max_concerns
        ));
    }

    let passed = reasons.is_empty();

    match format {
        Format::Json | Format::Jsonl => {
            #[derive(serde::Serialize)]
            struct CheckReport<'a> {
                passed: bool,
                review_load: u32,
                max_review_load: u32,
                cohesion: f64,
                min_cohesion: f64,
                concerns: usize,
                max_concerns: usize,
                reasons: &'a [String],
            }
            let out = CheckReport {
                passed,
                review_load: result.review_load.total,
                max_review_load: policy.max_review_load,
                cohesion: result.cohesion,
                min_cohesion: policy.min_cohesion,
                concerns: counted_concerns,
                max_concerns: policy.max_concerns,
                reasons: &reasons,
            };
            let text = if format == Format::Json {
                serde_json::to_string_pretty(&out)?
            } else {
                serde_json::to_string(&out)?
            };
            println!("{text}");
        }
        Format::Text => {
            if passed {
                println!("SCRAWNY OK");
                println!();
                println!(
                    "Review load: {}/100 (max {})",
                    result.review_load.total, policy.max_review_load
                );
            } else {
                println!("SCRAWNY FAILURE");
                println!();
                println!("Reason:");
                for reason in &reasons {
                    println!("  {reason}");
                }
                println!();
                println!("Suggested action:");
                println!("  Run `scrawny explain` for details.");
            }
        }
    }

    Ok(passed)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
