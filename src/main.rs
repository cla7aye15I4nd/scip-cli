mod config;
mod generator;
mod template;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::config::{find_profile, load_profiles};
use crate::generator::{GenerateOptions, Generator};
use crate::template::Variables;
use scip_cli::{BuildOptions, build as build_site};

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
    /// Generate SCIP indexes and add every configured repository to one website.
    GenerateAll {
        /// Shared website root containing index.html and generated project data.
        #[arg(long, default_value = "web")]
        web_root: PathBuf,

        /// Directory where reusable <repository>.scip files are written.
        #[arg(short, long, default_value = ".scip-cli/indexes")]
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

        /// Reuse existing compilation databases without running build commands.
        #[arg(long)]
        skip_build: bool,
    },
    /// Convert an existing SCIP index into the static source browser.
    Html {
        /// SCIP protobuf index to convert.
        #[arg(value_name = "INDEX.SCIP")]
        input: PathBuf,

        /// Repository root containing indexed source files.
        #[arg(short = 'r', long)]
        source_root: Option<PathBuf>,

        /// Canonical repository URL used to identify this project.
        #[arg(long)]
        repo_url: String,

        /// Indexed commit SHA (or another stable, URL-safe revision).
        #[arg(long)]
        commit: String,

        /// Shared web application directory.
        #[arg(long, default_value = "web")]
        web_root: PathBuf,

        /// Website title shown in the navigation bar.
        #[arg(long, default_value = "SCIP source browser")]
        title: String,
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
    let configured_dir = cli.config_dir;
    match cli.command {
        Commands::List => {
            let config_dir = resolve_config_dir(configured_dir.as_deref())?;
            for (_, profile) in load_profiles(&config_dir)? {
                if profile.description.is_empty() {
                    println!("{}", profile.name);
                } else {
                    println!("{:<16} {}", profile.name, profile.description);
                }
            }
        }
        Commands::Show { repository } => {
            let config_dir = resolve_config_dir(configured_dir.as_deref())?;
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
            let config_dir = resolve_config_dir(configured_dir.as_deref())?;
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
        Commands::GenerateAll {
            web_root,
            output_dir,
            work_dir,
            tools_dir,
            jobs,
            index_jobs,
            scip_clang,
            skip_build,
        } => {
            let config_dir = resolve_config_dir(configured_dir.as_deref())?;
            generate_all(
                &config_dir,
                web_root,
                GenerateOptions {
                    output_dir,
                    work_dir,
                    tools_dir,
                    jobs,
                    index_jobs,
                    scip_clang,
                    dry_run: false,
                    skip_build,
                },
            )?;
        }
        Commands::Html {
            input,
            source_root,
            repo_url,
            commit,
            web_root,
            title,
        } => {
            let report = build_site(BuildOptions {
                input,
                source_root,
                web_root,
                repo_url,
                commit,
                title,
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
        }
    }
    Ok(())
}

fn generate_all(config_dir: &Path, web_root: PathBuf, options: GenerateOptions) -> Result<()> {
    if options.jobs == 0 || options.index_jobs == 0 {
        anyhow::bail!("--jobs and --index-jobs must be greater than zero");
    }
    let profiles = load_profiles(config_dir)?;
    if profiles.is_empty() {
        anyhow::bail!("no repository profiles found in {}", config_dir.display());
    }

    println!(
        "==> Generating {} configured projects into {}",
        profiles.len(),
        web_root.display()
    );
    let mut failures = Vec::new();
    let mut generated = 0;
    for (path, profile) in profiles {
        println!(
            "\n==> [{}] Loaded profile from {}",
            profile.name,
            path.display()
        );
        let name = profile.name.clone();
        if let Err(error) = generate_project(profile, &web_root, options.clone()) {
            eprintln!("error: [{name}] {error:#}");
            failures.push(name);
        } else {
            generated += 1;
        }
    }

    println!(
        "\n==> Generated {generated} of {} projects into {}",
        generated + failures.len(),
        web_root.display()
    );
    if !failures.is_empty() {
        anyhow::bail!("failed projects: {}", failures.join(", "));
    }
    Ok(())
}

fn generate_project(
    profile: crate::config::Profile,
    web_root: &Path,
    options: GenerateOptions,
) -> Result<()> {
    let repo_url = render_repository_url(&profile)?;
    let repo_dir = absolute_from_cwd(&options.work_dir)?.join(profile.checkout_directory());
    let title = if profile.description.is_empty() {
        profile.name.clone()
    } else {
        profile.description.clone()
    };
    let index = Generator::new(profile, options)?.run()?;
    let commit = git_commit(&repo_dir)?;
    println!("==> Building website data for commit {commit}");
    build_site(BuildOptions {
        input: index,
        source_root: Some(repo_dir),
        web_root: web_root.to_owned(),
        repo_url,
        commit,
        title,
    })?;
    Ok(())
}

fn render_repository_url(profile: &crate::config::Profile) -> Result<String> {
    let mut variables = Variables::default();
    variables.insert("name", &profile.name);
    variables.insert("repo", &profile.name);
    for (key, value) in &profile.variables {
        variables.insert(key, value);
    }
    variables.render(&profile.repository.url)
}

fn absolute_from_cwd(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn git_commit(repo_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", &repo_dir.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("failed to inspect Git checkout {}", repo_dir.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "git rev-parse failed in {}: {}",
            repo_dir.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
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
