mod config;
mod generator;
mod template;

use std::env;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::{find_profile, load_profiles};
use crate::generator::{GenerateOptions, Generator};

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
    /// Clone, configure, build, and index a repository.
    Generate {
        /// Repository profile name.
        repository: String,

        /// Directory where the final <repository>.scip file is written.
        #[arg(short, long, default_value = "scip-output")]
        output_dir: PathBuf,

        /// Persistent repository checkout directory.
        #[arg(long, default_value = ".scip-cli/work")]
        work_dir: PathBuf,

        /// Persistent downloaded-tools directory.
        #[arg(long, default_value = ".scip-cli/tools")]
        tools_dir: PathBuf,

        /// Parallel jobs used by configured build commands.
        #[arg(short = 'j', long, default_value_t = 8)]
        jobs: usize,

        /// Parallel scip-clang worker processes.
        #[arg(long, default_value_t = 4)]
        index_jobs: usize,

        /// Use an existing scip-clang executable.
        #[arg(long)]
        scip_clang: Option<PathBuf>,

        /// Print planned actions without changing the filesystem.
        #[arg(long)]
        dry_run: bool,

        /// Reuse an existing compilation database without running profile commands.
        #[arg(long)]
        skip_build: bool,
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
        Commands::Generate {
            repository,
            output_dir,
            work_dir,
            tools_dir,
            jobs,
            index_jobs,
            scip_clang,
            dry_run,
            skip_build,
        } => {
            if jobs == 0 || index_jobs == 0 {
                anyhow::bail!("--jobs and --index-jobs must be greater than zero");
            }
            let (path, profile) = find_profile(&config_dir, &repository)?;
            println!("==> Loaded {} from {}", profile.name, path.display());
            let output = Generator::new(
                profile,
                GenerateOptions {
                    output_dir,
                    work_dir,
                    tools_dir,
                    jobs,
                    index_jobs,
                    scip_clang,
                    dry_run,
                    skip_build,
                },
            )?
            .run()?;
            if dry_run {
                println!("==> Dry run complete; planned output: {}", output.display());
            }
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
