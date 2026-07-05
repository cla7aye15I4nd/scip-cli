mod config;

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::{find_profile, load_profiles};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Directory containing repository YAML profiles.
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// List available repository profiles.
    List,
    /// Print a parsed repository profile.
    Show {
        /// Repository profile name.
        repository: String,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_dir = resolve_config_dir(cli.config_dir.as_deref())?;
    match cli.command {
        Commands::List => {
            for (_, profile) in load_profiles(&config_dir)? {
                if profile.description.is_empty() {
                    println!("{}", profile.name);
                } else {
                    println!("{:<16} {}", profile.name, profile.description);
                }
            }
        }
        Commands::Show { repository } => {
            let (path, profile) = find_profile(&config_dir, &repository)?;
            println!("# {}\n{}", path.display(), serde_yaml::to_string(&profile)?);
        }
    }
    Ok(())
}

fn resolve_config_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return existing_directory(path);
    }
    if let Some(path) = env::var_os("SCIP_CLI_CONFIG_DIR") {
        return existing_directory(Path::new(&path));
    }

    let cwd = env::current_dir()?;
    let executable = env::current_exe()?;
    let executable_dir = executable.parent().unwrap_or(Path::new("."));
    let candidates = [
        cwd.join("configs"),
        cwd.join("scip-cli/configs"),
        executable_dir.join("configs"),
        executable_dir.join("../configs"),
        executable_dir.join("../../configs"),
        executable_dir.join("../../scip-cli/configs"),
    ];
    candidates
        .into_iter()
        .find(|path| path.is_dir())
        .with_context(|| "could not locate configs; pass --config-dir or set SCIP_CLI_CONFIG_DIR")
}

fn existing_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_dir() {
        anyhow::bail!("config directory does not exist: {}", path.display());
    }
    Ok(path.to_owned())
}
