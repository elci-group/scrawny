//! Generate man pages for the scrawny CLI into `man/`.
//!
//! Run with `cargo run -p scrawny-cli --example gen-man`. One page is
//! written per command path: `man/scrawny.1` for the top-level command and
//! `man/scrawny-<subcommand>.1` for each subcommand.

use clap::CommandFactory;
use clap_mangen::Man;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "../src/cli.rs"]
mod cli;

fn write_page(cmd: &clap::Command, out_dir: &Path, page: &str) -> std::io::Result<()> {
    let man = Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    fs::create_dir_all(out_dir)?;
    fs::write(out_dir.join(format!("{page}.1")), buf)
}

fn generate(cmd: &clap::Command, out_dir: &Path, page: &str) -> std::io::Result<()> {
    write_page(cmd, out_dir, page)?;
    for sub in cmd.get_subcommands() {
        generate(sub, out_dir, &format!("{page}-{}", sub.get_name()))?;
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let cmd = cli::Cli::command();
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../man");
    generate(&cmd, &out_dir, cmd.get_name())
}
