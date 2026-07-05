use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub repository: Repository,
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    #[serde(default)]
    pub tools: Vec<GitTool>,
    #[serde(default)]
    pub path_prepend: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    pub index: IndexSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub url: String,
    #[serde(default)]
    pub directory: Option<String>,
    #[serde(default = "default_depth")]
    pub depth: u32,
    #[serde(default)]
    pub revision: Option<String>,
}

fn default_depth() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitTool {
    pub name: String,
    pub repository: String,
    pub destination: String,
    #[serde(default = "default_depth")]
    pub depth: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub name: String,
    pub run: String,
    #[serde(default = "default_repo_cwd")]
    pub cwd: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

fn default_repo_cwd() -> String {
    "{repo_dir}".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSpec {
    pub compilation_database: String,
    #[serde(default = "default_output")]
    pub output: String,
    #[serde(default)]
    pub arguments: Vec<String>,
}

fn default_output() -> String {
    "{output_dir}/{name}.scip".to_owned()
}

impl Profile {
    pub fn from_path(path: &Path) -> Result<Self> {
        let value = resolve_document(path, &mut Vec::new())?;
        let profile: Self = serde_yaml::from_value(value)
            .with_context(|| format!("failed to parse profile {}", path.display()))?;
        profile.validate(path)?;
        Ok(profile)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        if self.version != 1 {
            bail!(
                "profile {} uses unsupported schema version {}; expected 1",
                path.display(),
                self.version
            );
        }
        validate_name(&self.name, "profile name")?;
        if self.repository.url.trim().is_empty() {
            bail!("profile {} has an empty repository URL", path.display());
        }
        if self.repository.depth == 0 {
            bail!(
                "profile {} has a zero repository clone depth",
                path.display()
            );
        }
        if let Some(directory) = &self.repository.directory {
            validate_relative_path(directory, "repository directory")?;
        }
        for tool in &self.tools {
            validate_name(&tool.name, "tool name")?;
            validate_relative_path(&tool.destination, "tool destination")?;
            if tool.repository.trim().is_empty() || tool.depth == 0 {
                bail!(
                    "tool {} has an invalid repository or clone depth",
                    tool.name
                );
            }
        }
        for key in self.variables.keys() {
            validate_name(key, "variable name")?;
            if RESERVED_VARIABLES.contains(&key.as_str()) {
                bail!("profile variable '{key}' is reserved by scip-cli");
            }
        }
        if self.index.compilation_database.trim().is_empty() {
            bail!("profile {} has no compilation database", path.display());
        }
        if Path::new(&self.index.output)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            bail!("profile output may not contain '..': {}", self.index.output);
        }
        Ok(())
    }
}

const RESERVED_VARIABLES: &[&str] = &[
    "name",
    "repo",
    "repo_dir",
    "work_dir",
    "output_dir",
    "tools_dir",
    "jobs",
    "index_jobs",
    "scip_clang",
];

fn resolve_document(path: &Path, stack: &mut Vec<PathBuf>) -> Result<Value> {
    let identity = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if stack.contains(&identity) {
        let chain = stack
            .iter()
            .chain(std::iter::once(&identity))
            .map(|entry| entry.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        bail!("profile inheritance cycle: {chain}");
    }
    stack.push(identity);

    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile {}", path.display()))?;
    let mut child: Value = serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse YAML in {}", path.display()))?;
    let extends_key = Value::String("extends".to_owned());
    let extends = child
        .as_mapping_mut()
        .and_then(|mapping| mapping.remove(&extends_key));

    let resolved = if let Some(extends) = extends {
        let relative = extends
            .as_str()
            .with_context(|| format!("'extends' must be a path string in {}", path.display()))?;
        let base_path = path.parent().unwrap_or(Path::new(".")).join(relative);
        let mut base = resolve_document(&base_path, stack).with_context(|| {
            format!(
                "failed to resolve base profile '{}' for {}",
                base_path.display(),
                path.display()
            )
        })?;
        deep_merge(&mut base, child);
        base
    } else {
        child
    };
    stack.pop();
    Ok(resolved)
}

fn deep_merge(base: &mut Value, child: Value) {
    match (base, child) {
        (Value::Mapping(base_map), Value::Mapping(child_map)) => {
            merge_mappings(base_map, child_map);
        }
        (base_slot, child_value) => *base_slot = child_value,
    }
}

fn merge_mappings(base: &mut Mapping, child: Mapping) {
    for (key, child_value) in child {
        match base.get_mut(&key) {
            Some(base_value) => deep_merge(base_value, child_value),
            None => {
                base.insert(key, child_value);
            }
        }
    }
}

pub fn load_profiles(config_dir: &Path) -> Result<Vec<(PathBuf, Profile)>> {
    let entries = fs::read_dir(config_dir)
        .with_context(|| format!("failed to read config directory {}", config_dir.display()))?;
    let mut profiles = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("yaml")
            || path.extension().and_then(|value| value.to_str()) == Some("yml")
        {
            profiles.push((path.clone(), Profile::from_path(&path)?));
        }
    }
    profiles.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(profiles)
}

pub fn find_profile(config_dir: &Path, name: &str) -> Result<(PathBuf, Profile)> {
    validate_name(name, "repository name")?;
    let profiles = load_profiles(config_dir)?;
    profiles
        .into_iter()
        .find(|(_, profile)| profile.name == name)
        .with_context(|| {
            format!(
                "no profile named '{name}' in {}; run `scip-cli list`",
                config_dir.display()
            )
        })
}

fn validate_name(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("invalid {label} '{value}'; use letters, digits, '-' or '_'");
    }
    Ok(())
}

fn validate_relative_path(value: &str, label: &str) -> Result<()> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("{label} must be a safe relative path: {value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_checkout_path() {
        assert!(validate_relative_path("../outside", "test").is_err());
    }

    #[test]
    fn accepts_profile_names() {
        assert!(validate_name("v8-main_1", "test").is_ok());
        assert!(validate_name("v8/main", "test").is_err());
    }

    #[test]
    fn resolves_inherited_maps_and_replaces_lists() {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().join("base.yaml");
        let child = directory.path().join("child.yaml");
        fs::write(
            &base,
            r#"
version: 1
variables:
  build_dir: build
  base_only: present
commands:
  - name: Base
    run: base
"#,
        )
        .unwrap();
        fs::write(
            &child,
            r#"
extends: base.yaml
name: demo
description: inherited
repository:
  url: https://example.com/demo.git
variables:
  build_dir: custom
commands:
  - name: Child
    run: child
index:
  compilation_database: "{repo_dir}/custom/compile_commands.json"
"#,
        )
        .unwrap();

        let profile = Profile::from_path(&child).unwrap();
        assert_eq!(profile.variables["build_dir"], "custom");
        assert_eq!(profile.variables["base_only"], "present");
        assert_eq!(profile.commands.len(), 1);
        assert_eq!(profile.commands[0].name, "Child");
    }

    #[test]
    fn rejects_inheritance_cycles() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.yaml");
        let second = directory.path().join("second.yaml");
        fs::write(&first, "extends: second.yaml\n").unwrap();
        fs::write(&second, "extends: first.yaml\n").unwrap();
        assert!(Profile::from_path(&first).is_err());
    }
}
