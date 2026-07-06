use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use scip_cli::{BuildOptions, build};

/// Convert one SCIP index and its source tree into a static code browser.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// SCIP protobuf index to convert.
    #[arg(value_name = "INDEX.SCIP")]
    input: PathBuf,

    /// Repository root containing indexed source files.
    #[arg(short = 'r', long)]
    source_root: Option<PathBuf>,

    /// Canonical repository URL used to identify this project.
    #[arg(long)]
    repo_url: String,

    /// Indexed commit SHA or another stable, URL-safe revision.
    #[arg(long)]
    commit: String,

    /// Static site output directory.
    #[arg(short, long, default_value = "site")]
    output_dir: PathBuf,

    /// Website title shown in the browser.
    #[arg(long, default_value = "SCIP source browser")]
    title: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let report = build(BuildOptions {
        input: cli.input,
        source_root: cli.source_root,
        web_root: cli.output_dir,
        repo_url: cli.repo_url,
        commit: cli.commit,
        title: cli.title,
    })?;
    println!(
        "Generated {} files and {} navigable occurrences in {}",
        report.files,
        report.occurrences,
        report.output.display()
    );
    for warning in report.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
