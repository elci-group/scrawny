use clap::{Parser, Subcommand};
use scrawny_analysis::AnalysisResult;
use scrawny_config::Config;
use scrawny_core::Diff;
use scrawny_git::{diff_from_text, DiffSpec, GitCliRepository, Repository};
use scrawny_output::{build_report, render, Format};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

#[derive(Parser)]
#[command(name = "scrawny", version, about = "Detect and explain review-hostile changes.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Output format: text, json, jsonl
    #[arg(long, global = true, default_value = "text")]
    format: String,

    /// Repository path to analyse
    #[arg(long, global = true, default_value = ".")]
    path: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Analyse current changes (default when no subcommand is given).
    Analyse {
        /// Analyse only staged (index) changes instead of all uncommitted changes.
        #[arg(long)]
        staged: bool,
    },
    /// Explain why a change is considered review-hostile, with evidence.
    Explain {
        #[arg(long)]
        staged: bool,
    },
    /// Generate a decomposition proposal (not yet available — Phase II).
    Suggest {
        #[arg(long)]
        staged: bool,
    },
    /// CI-compatible pass/fail check against policy thresholds.
    Check {
        #[arg(long)]
        staged: bool,
        /// Check a supplied diff (e.g. a ref-range comparison) instead of the
        /// working repository. `--path` still selects which `.scrawny.toml`
        /// policy applies.
        #[arg(long)]
        stdin: bool,
        /// Read the diff from a file instead of stdin.
        file: Option<PathBuf>,
    },
    /// Analyse a supplied diff instead of the working repository.
    Diff {
        /// Read the diff from stdin.
        #[arg(long)]
        stdin: bool,
        /// Read the diff from a file.
        file: Option<PathBuf>,
    },
}

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
            let result = scrawny_analysis::analyse(&diff, &config);
            print_report(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Explain { staged } => {
            let (root, description, diff) = load_repo_diff(path, staged)?;
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let result = scrawny_analysis::analyse(&diff, &config);
            print_explain(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Suggest { staged } => {
            let (root, description, diff) = load_repo_diff(path, staged)?;
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let result = scrawny_analysis::analyse(&diff, &config);
            print_suggest(&root, &description, &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check { staged, stdin, file } => {
            let (root, diff) = if stdin || file.is_some() {
                (path.to_path_buf(), load_standalone_diff(stdin, file.as_deref())?)
            } else {
                let (root, _description, diff) = load_repo_diff(path, staged)?;
                (root, diff)
            };
            let config = Config::load(&root).map_err(anyhow::Error::from)?;
            let result = scrawny_analysis::analyse(&diff, &config);
            let passed = print_check(&result, &config, format)?;
            Ok(if passed { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
        Command::Diff { stdin, file } => {
            let diff = load_standalone_diff(stdin, file.as_deref())?;
            let config = Config::load(path).map_err(anyhow::Error::from)?;
            let result = scrawny_analysis::analyse(&diff, &config);
            print_report(path, "supplied diff", &result, format)?;
            Ok(ExitCode::SUCCESS)
        }
    }
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
    let note = "Decomposition proposals require semantic clustering (dependency graph, \
        cohesion-validated partitions), which is Phase II work not yet implemented. \
        Run `scrawny explain` for the concern breakdown available today.";

    match format {
        Format::Json | Format::Jsonl => {
            let report = build_report(&root.display().to_string(), description, result);
            #[derive(serde::Serialize)]
            struct SuggestReport<'a> {
                #[serde(flatten)]
                report: &'a scrawny_output::Report,
                decomposition_available: bool,
                note: &'a str,
            }
            let out = SuggestReport { report: &report, decomposition_available: false, note };
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
            println!("{note}");
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
