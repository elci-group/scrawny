use scrawny_config::Config;
use scrawny_core::{Classification, Concern, DiffLine, FileStatus, Hunk};
use std::path::Path;

/// Phase I concern classification: deterministic, path- and text-heuristic
/// based. No AST, no cross-file dependency reasoning — that arrives in
/// Phase II (§10/§33). Every branch carries an honest confidence; nothing
/// here claims certainty it hasn't earned.
pub fn classify_hunk(path: &Path, status: &FileStatus, hunk: &Hunk, config: &Config) -> Classification {
    if is_generated(path, config) {
        return Classification::new(Concern::Mechanical, 0.9);
    }

    if matches!(status, FileStatus::Renamed) && is_pure_move(hunk) {
        return Classification::new(Concern::Structural, 0.85);
    }

    if is_whitespace_only(hunk) {
        return Classification::new(Concern::Mechanical, 0.95);
    }

    if is_lockfile(path) {
        return Classification::new(Concern::Dependency, 0.9);
    }

    if is_documentation(path) {
        return Classification::new(Concern::Documentation, 0.9);
    }

    if is_test(path) {
        return Classification::new(Concern::Testing, 0.8);
    }

    if is_configuration(path) {
        if manifest_touches_dependencies(hunk) {
            return Classification::new(Concern::Dependency, 0.75);
        }
        return Classification::new(Concern::Configuration, 0.8);
    }

    if is_recognised_source(path) {
        return Classification::new(Concern::Behavioural, 0.7);
    }

    Classification::new(Concern::Unknown, 0.3)
}

fn is_generated(path: &Path, config: &Config) -> bool {
    config
        .generated
        .iter()
        .any(|pattern| glob_match(pattern, path))
}

/// Minimal glob support: `**` matches any path prefix, `*` matches within a
/// segment. Sufficient for the small, explicit pattern lists in config —
/// not a general glob engine.
fn glob_match(pattern: &str, path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let pattern = pattern.replace("**/", "");
    if let Some(suffix) = pattern.strip_prefix('*') {
        return path_str.ends_with(suffix);
    }
    path_str == pattern || path_str.ends_with(&format!("/{pattern}"))
}

fn is_lockfile(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("Cargo.lock") | Some("package-lock.json") | Some("yarn.lock") | Some("pnpm-lock.yaml")
    )
}

fn is_documentation(path: &Path) -> bool {
    let ext_match = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md") | Some("mdx") | Some("rst") | Some("adoc")
    );
    let path_str = path.to_string_lossy().to_lowercase();
    ext_match || path_str.contains("docs/") || path_str.starts_with("readme")
}

fn is_test(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    path_str.contains("/tests/")
        || path_str.contains("/test/")
        || path_str.starts_with("tests/")
        || path_str.starts_with("test/")
        || path_str.contains("_test.")
        || path_str.contains("test_")
        || path_str.contains(".test.")
        || path_str.contains("__tests__")
        || path_str.contains("/spec/")
        || path_str.ends_with("_spec.rb")
}

fn is_configuration(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    let ext = path.extension().and_then(|e| e.to_str());
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    path_str.contains(".github/workflows")
        || name == "Dockerfile"
        || name == "docker-compose.yml"
        || name == "Cargo.toml"
        || name == "package.json"
        || name == ".scrawny.toml"
        || matches!(ext, Some("toml") | Some("yaml") | Some("yml") | Some("ini") | Some("json"))
}

fn manifest_touches_dependencies(hunk: &Hunk) -> bool {
    let mut in_deps_section = false;
    for line in &hunk.lines {
        let text = match line {
            DiffLine::Added(t) | DiffLine::Removed(t) | DiffLine::Context(t) => t,
        };
        let trimmed = text.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_deps_section = trimmed.to_lowercase().contains("dependencies");
        }
        if in_deps_section && matches!(line, DiffLine::Added(_) | DiffLine::Removed(_)) {
            return true;
        }
    }
    false
}

fn is_recognised_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("rs")
            | Some("py")
            | Some("js")
            | Some("jsx")
            | Some("ts")
            | Some("tsx")
            | Some("go")
            | Some("java")
            | Some("c")
            | Some("h")
            | Some("cpp")
            | Some("hpp")
            | Some("rb")
            | Some("sh")
    )
}

fn is_pure_move(hunk: &Hunk) -> bool {
    hunk.lines.is_empty()
}

fn is_whitespace_only(hunk: &Hunk) -> bool {
    let mut added: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();

    for line in &hunk.lines {
        match line {
            DiffLine::Added(t) => added.push(normalise_whitespace(t)),
            DiffLine::Removed(t) => removed.push(normalise_whitespace(t)),
            DiffLine::Context(_) => {}
        }
    }

    if added.is_empty() && removed.is_empty() {
        return false;
    }

    added == removed
}

fn normalise_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
