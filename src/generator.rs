use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::{CommandSpec, GitTool, Profile};
use crate::template::Variables;

const SCIP_CLANG_VERSION: &str = "v0.4.0";

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub output_dir: PathBuf,
    pub work_dir: PathBuf,
    pub tools_dir: PathBuf,
    pub jobs: usize,
    pub index_jobs: usize,
    pub scip_clang: Option<PathBuf>,
    pub dry_run: bool,
    pub skip_build: bool,
}

pub struct Generator {
    profile: Profile,
    options: GenerateOptions,
    repo_dir: PathBuf,
    scip_clang: PathBuf,
    path_entries: Vec<PathBuf>,
    variables: Variables,
}

impl Generator {
    pub fn new(profile: Profile, options: GenerateOptions) -> Result<Self> {
        let output_dir = absolute(&options.output_dir)?;
        let work_dir = absolute(&options.work_dir)?;
        let tools_dir = absolute(&options.tools_dir)?;
        let repo_dir = work_dir.join(profile.checkout_directory());

        let mut options = options;
        options.output_dir = output_dir;
        options.work_dir = work_dir;
        options.tools_dir = tools_dir;

        let mut variables = Variables::default();
        variables.insert("name", &profile.name);
        variables.insert("repo", &profile.name);
        variables.insert("repo_dir", path_string(&repo_dir));
        variables.insert("work_dir", path_string(&options.work_dir));
        variables.insert("output_dir", path_string(&options.output_dir));
        variables.insert("tools_dir", path_string(&options.tools_dir));
        variables.insert("jobs", options.jobs.to_string());
        variables.insert("index_jobs", options.index_jobs.to_string());
        for (key, value) in &profile.variables {
            variables.insert(key, value);
        }

        Ok(Self {
            profile,
            options,
            repo_dir,
            scip_clang: PathBuf::new(),
            path_entries: Vec::new(),
            variables,
        })
    }

    pub fn run(mut self) -> Result<PathBuf> {
        self.prepare_directories()?;
        let checkout_changed = self.clone_repository()?;
        let output = self.output_path()?;
        let cache_key = self.cache_key()?;
        if self.cached_index_is_current(&output, &cache_key, checkout_changed)? {
            println!("==> Reusing unchanged SCIP index {}", output.display());
            if !self.options.dry_run {
                fs::write(self.cache_path(), &cache_key)?;
            }
            return Ok(output);
        }
        self.ensure_tools()?;
        if !self.options.skip_build {
            self.run_commands()?;
        } else {
            println!("==> Skipping configured build commands");
        }
        let output = self.run_indexer()?;
        if !self.options.dry_run {
            fs::write(self.cache_path(), cache_key)?;
        }
        Ok(output)
    }

    fn prepare_directories(&self) -> Result<()> {
        if self.options.dry_run {
            return Ok(());
        }
        fs::create_dir_all(&self.options.output_dir)?;
        fs::create_dir_all(&self.options.work_dir)?;
        fs::create_dir_all(&self.options.tools_dir)?;
        fs::create_dir_all(self.options.tools_dir.join("ccache"))?;
        Ok(())
    }

    fn cache_key(&self) -> Result<String> {
        let commit = git_head(&self.repo_dir)?;
        let profile = serde_json::to_string(&self.profile)?;
        Ok(format!("scip-clang={SCIP_CLANG_VERSION}\ncommit={commit}\nprofile={profile}\n"))
    }

    fn cache_path(&self) -> PathBuf {
        self.options.output_dir.join(format!(".{}.scip-cache", self.profile.name))
    }

    fn cached_index_is_current(&self, output: &Path, key: &str, checkout_changed: bool) -> Result<bool> {
        if self.options.dry_run || !output.is_file() || fs::metadata(output)?.len() < 1024 {
            return Ok(false);
        }
        match fs::read_to_string(self.cache_path()) {
            Ok(stored) => Ok(stored == key),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(!checkout_changed),
            Err(error) => Err(error.into()),
        }
    }

    fn ensure_tools(&mut self) -> Result<()> {
        for tool in self.profile.tools.clone() {
            self.ensure_git_tool(&tool)?;
        }
        for entry in &self.profile.path_prepend {
            self.path_entries
                .push(PathBuf::from(self.variables.render(entry)?));
        }

        self.scip_clang = if let Some(path) = &self.options.scip_clang {
            absolute(path)?
        } else if let Some(path) = find_in_path("scip-clang") {
            path
        } else {
            let path = self.options.tools_dir.join("scip-clang");
            self.install_scip_clang(&path)?;
            path
        };
        if !self.options.dry_run && !self.scip_clang.is_file() {
            bail!(
                "scip-clang executable does not exist: {}",
                self.scip_clang.display()
            );
        }
        self.variables
            .insert("scip_clang", path_string(&self.scip_clang));
        Ok(())
    }

    fn ensure_git_tool(&self, tool: &GitTool) -> Result<()> {
        let repository = self.variables.render(&tool.repository)?;
        let destination = self
            .options
            .tools_dir
            .join(self.variables.render(&tool.destination)?);
        if destination.join(".git").is_dir() {
            println!("==> Using tool {} at {}", tool.name, destination.display());
            return Ok(());
        }
        if destination.exists() {
            bail!(
                "tool destination exists but is not a Git checkout: {}",
                destination.display()
            );
        }
        println!("==> Installing tool {}", tool.name);
        self.run_program(
            "git",
            &[
                "clone".into(),
                "--depth".into(),
                tool.depth.to_string(),
                repository,
                path_string(&destination),
            ],
            None,
        )
    }

    fn install_scip_clang(&self, destination: &Path) -> Result<()> {
        if destination.is_file() {
            return Ok(());
        }
        if env::consts::OS != "linux" || env::consts::ARCH != "x86_64" {
            bail!("automatic scip-clang installation supports x86_64 Linux; use --scip-clang");
        }
        let url = format!(
            "https://github.com/sourcegraph/scip-clang/releases/download/{SCIP_CLANG_VERSION}/scip-clang-x86_64-linux"
        );
        println!("==> Installing scip-clang {SCIP_CLANG_VERSION}");
        self.run_program(
            "curl",
            &[
                "--fail".into(),
                "--location".into(),
                "--retry".into(),
                "3".into(),
                "--output".into(),
                path_string(destination),
                url,
            ],
            None,
        )?;
        if !self.options.dry_run {
            let mut permissions = fs::metadata(destination)?.permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(destination, permissions)?;
        }
        Ok(())
    }

    fn clone_repository(&self) -> Result<bool> {
        let repository = self.variables.render(&self.profile.repository.url)?;
        let existing_checkout = self.repo_dir.join(".git").is_dir();
        let previous_head = existing_checkout.then(|| git_head(&self.repo_dir)).transpose()?;
        if existing_checkout {
            println!("==> Using checkout {}", self.repo_dir.display());
        } else {
            if self.repo_dir.exists() {
                bail!(
                    "checkout path exists but is not a Git repository: {}",
                    self.repo_dir.display()
                );
            }
            println!("==> Cloning {repository}");
            self.run_program(
                "git",
                &[
                    "clone".into(),
                    "--depth".into(),
                    self.profile.repository.depth.to_string(),
                    repository,
                    path_string(&self.repo_dir),
                ],
                None,
            )?;
        }
        if let Some(revision) = &self.profile.repository.revision {
            println!("==> Fetching and checking out {revision}");
            self.run_program(
                "git",
                &[
                    "fetch".into(),
                    "--depth".into(),
                    self.profile.repository.depth.to_string(),
                    "origin".into(),
                    revision.clone(),
                ],
                Some(&self.repo_dir),
            )?;
            self.run_program(
                "git",
                &["checkout".into(), "--detach".into(), "FETCH_HEAD".into()],
                Some(&self.repo_dir),
            )?;
        } else if existing_checkout {
            println!("==> Fetching the latest upstream revision");
            self.run_program(
                "git",
                &[
                    "fetch".into(),
                    "--depth".into(),
                    self.profile.repository.depth.to_string(),
                    "origin".into(),
                    "HEAD".into(),
                ],
                Some(&self.repo_dir),
            )?;
            self.run_program(
                "git",
                &["checkout".into(), "--detach".into(), "FETCH_HEAD".into()],
                Some(&self.repo_dir),
            )?;
        }
        let current_head = if self.options.dry_run { previous_head.clone() } else { Some(git_head(&self.repo_dir)?) };
        Ok(!existing_checkout || previous_head != current_head)
    }

    fn run_commands(&self) -> Result<()> {
        for command in &self.profile.commands {
            self.run_configured_command(command)?;
        }
        Ok(())
    }

    fn run_configured_command(&self, spec: &CommandSpec) -> Result<()> {
        let cwd = PathBuf::from(self.variables.render(&spec.cwd)?);
        let script = self.variables.render(&spec.run)?;
        println!("==> {}", spec.name);
        println!("    $ {script}");
        if self.options.dry_run {
            return Ok(());
        }
        if !cwd.is_dir() {
            bail!(
                "command working directory does not exist: {}",
                cwd.display()
            );
        }
        let mut command = Command::new("bash");
        command.arg("-c").arg(&script).current_dir(&cwd);
        self.apply_environment(&mut command)?;
        for (key, value) in &spec.env {
            command.env(key, self.variables.render(value)?);
        }
        run_status(command).with_context(|| format!("command '{}' failed", spec.name))
    }

    fn run_indexer(&self) -> Result<PathBuf> {
        let rendered_compdb = PathBuf::from(
            self.variables
                .render(&self.profile.index.compilation_database)?,
        );
        let compdb = if rendered_compdb.is_absolute() {
            rendered_compdb
        } else {
            self.repo_dir.join(rendered_compdb)
        };
        let output = self.output_path()?;
        if !output.starts_with(&self.options.output_dir) {
            bail!(
                "profile output must remain under --output-dir: {}",
                output.display()
            );
        }
        if !self.options.dry_run && !compdb.is_file() {
            bail!("compilation database does not exist: {}", compdb.display());
        }
        if let Some(parent) = output.parent()
            && !self.options.dry_run
        {
            fs::create_dir_all(parent)?;
        }

        let mut args = vec![
            format!("--compdb-path={}", compdb.display()),
            format!("--index-output-path={}", output.display()),
            format!("--jobs={}", self.options.index_jobs),
        ];
        for argument in &self.profile.index.arguments {
            args.push(self.variables.render(argument)?);
        }

        println!("==> Generating {}", output.display());
        self.run_program(
            self.scip_clang.to_string_lossy().as_ref(),
            &args,
            Some(&self.repo_dir),
        )?;
        if !self.options.dry_run {
            let metadata = fs::metadata(&output)
                .with_context(|| format!("indexer did not create {}", output.display()))?;
            if metadata.len() < 1024 {
                bail!(
                    "generated index is suspiciously small ({} bytes): {}",
                    metadata.len(),
                    output.display()
                );
            }
            println!("==> Wrote {} ({} bytes)", output.display(), metadata.len());
        }
        Ok(output)
    }

    fn output_path(&self) -> Result<PathBuf> {
        let rendered = PathBuf::from(self.variables.render(&self.profile.index.output)?);
        let output = if rendered.is_absolute() { rendered } else { self.options.output_dir.join(rendered) };
        if !output.starts_with(&self.options.output_dir) {
            bail!("profile output must remain under --output-dir: {}", output.display());
        }
        Ok(output)
    }

    fn run_program(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<()> {
        let rendered = std::iter::once(program.to_owned())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        println!("    $ {rendered}");
        if self.options.dry_run {
            return Ok(());
        }
        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        self.apply_environment(&mut command)?;
        run_status(command)
    }

    fn apply_environment(&self, command: &mut Command) -> Result<()> {
        let mut configured_entries = self.path_entries.clone();
        let ccache_wrappers = PathBuf::from("/usr/lib/ccache");
        if ccache_wrappers.is_dir() {
            configured_entries.insert(0, ccache_wrappers);
        }
        if !configured_entries.is_empty() {
            let current = env::var_os("PATH").unwrap_or_default();
            let mut entries = configured_entries;
            entries.extend(env::split_paths(&current));
            let path = env::join_paths(entries)?;
            command.env("PATH", path);
        }
        if find_in_path("ccache").is_some() {
            let cache_dir = env::var_os("CCACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| self.options.tools_dir.join("ccache"));
            command
                .env("CCACHE_DIR", cache_dir)
                .env("CCACHE_BASEDIR", &self.repo_dir)
                .env("CCACHE_NOHASHDIR", "true")
                .env("CCACHE_COMPILERCHECK", "content");
        }
        command
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        Ok(())
    }
}

fn git_head(repo_dir: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", &path_string(repo_dir), "rev-parse", "HEAD"])
        .output()
        .with_context(|| format!("failed to inspect Git checkout {}", repo_dir.display()))?;
    if !output.status.success() {
        bail!("git rev-parse failed in {}", repo_dir.display());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn run_status(mut command: Command) -> Result<()> {
    let status = command.status().context("failed to start process")?;
    if !status.success() {
        bail!("process exited with {status}");
    }
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(name);
        candidate.is_file().then_some(candidate)
    })
}
