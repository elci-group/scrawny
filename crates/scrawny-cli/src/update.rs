//! Self-update: clone the latest source and install it with Baby.

use anyhow::{anyhow, Context, Result};
use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Clone the latest `scrawny` source and install it via `baby --user`.
pub fn run() -> Result<()> {
    let repo_url = env!("CARGO_PKG_REPOSITORY");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let checkout_dir =
        env::temp_dir().join(format!("scrawny-update-{}-{nonce}", std::process::id()));

    if checkout_dir.exists() {
        std::fs::remove_dir_all(&checkout_dir)
            .with_context(|| format!("failed to clean up {}", checkout_dir.display()))?;
    }

    println!("Cloning latest scrawny from {repo_url}…");
    let clone_status = Command::new("git")
        .args(["clone", "--depth", "1", repo_url])
        .arg(&checkout_dir)
        .status()
        .context("failed to run git clone")?;
    if !clone_status.success() {
        return Err(anyhow!("git clone of {repo_url} failed"));
    }

    println!("Installing with 'baby --user'…");
    let install_result = Command::new("baby")
        .arg("--user")
        .current_dir(&checkout_dir)
        .status();

    let _ = std::fs::remove_dir_all(&checkout_dir);

    let install_status =
        install_result.context("failed to run 'baby --user' (is baby installed and on PATH?)")?;
    if !install_status.success() {
        return Err(anyhow!("'baby --user' install failed"));
    }

    println!("scrawny updated to the latest version");
    Ok(())
}
