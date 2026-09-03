use scrawny_core::ScrawnyError;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CONFIG_FILE_NAME: &str = ".scrawny.toml";

/// CI policy thresholds. Exceptions are named and explicit by design (§19 of
/// the project directive) — there is deliberately no blanket "ignore
/// everything" switch, only named, explainable carve-outs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    pub max_review_load: u32,
    pub min_cohesion: f64,
    pub max_concerns: usize,
    pub exceptions: Exceptions,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            max_review_load: 70,
            min_cohesion: 0.55,
            max_concerns: 4,
            exceptions: Exceptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Exceptions {
    pub documentation: bool,
    pub generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub policy: Policy,
    /// Glob patterns for paths excluded from analysis entirely.
    pub ignore: Vec<String>,
    /// Glob patterns for files treated as generated (mechanical, low weight).
    pub generated: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            ignore: Vec::new(),
            generated: default_generated_globs(),
        }
    }
}

fn default_generated_globs() -> Vec<String> {
    vec![
        "**/Cargo.lock".into(),
        "**/package-lock.json".into(),
        "**/yarn.lock".into(),
        "**/pnpm-lock.yaml".into(),
        "**/*.generated.*".into(),
        "**/*_pb2.py".into(),
        "**/*.pb.go".into(),
    ]
}

impl Config {
    /// Load `.scrawny.toml` from `root` if present, otherwise return
    /// defaults. A present-but-invalid file is a hard error rather than a
    /// silent fallback to defaults — Scrawny must never treat a
    /// misconfiguration as "safe".
    pub fn load(root: &Path) -> Result<Self, ScrawnyError> {
        let path = root.join(CONFIG_FILE_NAME);
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path).map_err(|source| ScrawnyError::Io {
            path: path.clone(),
            source,
        })?;

        toml::from_str(&text).map_err(|e| ScrawnyError::Configuration {
            config_path: path.display().to_string(),
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert_eq!(config.policy.max_review_load, 70);
        assert!(!config.generated.is_empty());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = std::env::temp_dir().join("scrawny-config-test-missing");
        let config = Config::load(&dir).expect("defaults on missing file");
        assert_eq!(config.policy.max_concerns, 4);
    }

    #[test]
    fn parses_policy_overrides() {
        let dir = tempdir();
        std::fs::write(
            dir.join(CONFIG_FILE_NAME),
            r#"
            [policy]
            max_review_load = 50
            min_cohesion = 0.7
            max_concerns = 3

            [policy.exceptions]
            documentation = true
            "#,
        )
        .unwrap();

        let config = Config::load(&dir).unwrap();
        assert_eq!(config.policy.max_review_load, 50);
        assert_eq!(config.policy.max_concerns, 3);
        assert!(config.policy.exceptions.documentation);
        assert!(!config.policy.exceptions.generated);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scrawny-config-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
