use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use protobuf::Message;
use scip::types::{Document, Index, Occurrence, PositionEncoding, occurrence};
use serde::Serialize;
use url::Url;

const DEFINITION_ROLE: i32 = 1;

pub struct BuildOptions {
    pub input: PathBuf,
    pub source_root: Option<PathBuf>,
    pub web_root: PathBuf,
    pub repo_url: String,
    pub commit: String,
    pub title: String,
}

pub struct BuildReport {
    pub files: usize,
    pub occurrences: usize,
    pub output: PathBuf,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct Range {
    start_line: usize,
    start_character: usize,
    end_line: usize,
    end_character: usize,
}

#[derive(Clone, Copy)]
struct Definition {
    file: usize,
    line: usize,
    character: usize,
}

struct Definitions {
    global: HashMap<String, Definition>,
    local: Vec<HashMap<String, Definition>>,
}

impl Definitions {
    fn new(file_count: usize) -> Self {
        Self {
            global: HashMap::new(),
            local: (0..file_count).map(|_| HashMap::new()).collect(),
        }
    }

    fn insert(&mut self, file: usize, symbol: &str, definition: Definition) {
        let map = if symbol.starts_with("local ") {
            &mut self.local[file]
        } else {
            &mut self.global
        };
        map.entry(symbol.to_owned()).or_insert(definition);
    }

    fn get(&self, file: usize, symbol: &str) -> Option<&Definition> {
        if symbol.starts_with("local ") {
            self.local[file].get(symbol)
        } else {
            self.global.get(symbol)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest<'a> {
    title: &'a str,
    repo_url: &'a str,
    repo_slug: &'a str,
    commit: &'a str,
    files: Vec<ManifestFile<'a>>,
    file_count: usize,
    occurrence_count: usize,
}

#[derive(Debug, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    version: u32,
    projects: Vec<CatalogProject>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogProject {
    slug: String,
    repo_url: String,
    commits: Vec<CatalogCommit>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogCommit {
    commit: String,
    title: String,
    file_count: usize,
    occurrence_count: usize,
}

#[derive(Serialize)]
struct ManifestFile<'a> {
    id: usize,
    path: &'a str,
    language: &'a str,
}

#[derive(Serialize)]
struct FileData<'a> {
    path: &'a str,
    language: &'a str,
    text: &'a str,
    symbols: Vec<&'a str>,
    /// [start line, start UTF-16 column, end line, end UTF-16 column,
    ///  local symbol id, target file, target line, target UTF-16 column, roles,
    ///  SCIP syntax kind]
    occurrences: Vec<[i64; 10]>,
}

pub fn build(options: BuildOptions) -> Result<BuildReport> {
    validate_route_segment(&options.commit, "commit")?;
    let repo_slug = slug_repo_url(&options.repo_url)?;
    let bytes = fs::read(&options.input)
        .with_context(|| format!("failed to read {}", options.input.display()))?;
    let mut index = Index::parse_from_bytes(&bytes).context("failed to decode SCIP protobuf")?;
    if index.documents.is_empty() {
        bail!("SCIP index contains no documents");
    }

    index
        .documents
        .sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    for document in &index.documents {
        validate_relative_path(&document.relative_path)?;
    }

    let source_root = options.source_root.or_else(|| infer_source_root(&index));
    let mut warnings = Vec::new();
    let sources: Vec<String> = index
        .documents
        .iter()
        .map(|document| read_source(document, source_root.as_deref(), &mut warnings))
        .collect::<Result<_>>()?;

    let definitions = collect_definitions(&index.documents, &sources);
    let generated = options
        .web_root
        .join("generated")
        .join(&repo_slug)
        .join(&options.commit);
    fs::create_dir_all(options.web_root.join("assets"))?;
    fs::create_dir_all(generated.join("files"))?;
    fs::write(
        options.web_root.join("index.html"),
        include_str!("../assets/index.html"),
    )?;
    fs::write(
        options.web_root.join("404.html"),
        include_str!("../assets/index.html"),
    )?;
    fs::write(options.web_root.join("_redirects"), "/* /index.html 200\n")?;
    fs::write(
        options.web_root.join("assets/app.js"),
        include_str!("../assets/app.js"),
    )?;
    fs::write(
        options.web_root.join("assets/style.css"),
        include_str!("../assets/style.css"),
    )?;

    let mut occurrence_count = 0;
    for (file_id, document) in index.documents.iter().enumerate() {
        let data = make_file_data(file_id, document, &sources[file_id], &definitions);
        occurrence_count += data.occurrences.len();
        write_json(&generated.join(format!("files/{file_id}.json")), &data)?;
        write_javascript(
            &generated.join(format!("files/{file_id}.js")),
            "__SCIP_FILE__",
            &data,
        )?;
    }

    let manifest = Manifest {
        title: &options.title,
        repo_url: &options.repo_url,
        repo_slug: &repo_slug,
        commit: &options.commit,
        files: index
            .documents
            .iter()
            .enumerate()
            .map(|(id, document)| ManifestFile {
                id,
                path: &document.relative_path,
                language: &document.language,
            })
            .collect(),
        file_count: index.documents.len(),
        occurrence_count,
    };
    write_json(&generated.join("manifest.json"), &manifest)?;
    write_javascript(
        &generated.join("manifest.js"),
        "__SCIP_MANIFEST__",
        &manifest,
    )?;
    update_catalog(
        &options.web_root,
        &repo_slug,
        &options.repo_url,
        &options.commit,
        &options.title,
        index.documents.len(),
        occurrence_count,
    )?;

    Ok(BuildReport {
        files: index.documents.len(),
        occurrences: occurrence_count,
        output: options.web_root,
        warnings,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer(file, value)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_javascript(path: &Path, variable: &str, value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string(value).context("failed to encode JavaScript data")?;
    fs::write(path, format!("window.{variable}={json};"))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[allow(clippy::too_many_arguments)]
fn update_catalog(
    web_root: &Path,
    slug: &str,
    repo_url: &str,
    commit: &str,
    title: &str,
    file_count: usize,
    occurrence_count: usize,
) -> Result<()> {
    let path = web_root.join("generated/catalog.json");
    let mut catalog: Catalog = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Catalog {
            version: 1,
            projects: Vec::new(),
        },
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    catalog.version = 1;
    let project = match catalog.projects.iter_mut().find(|item| item.slug == slug) {
        Some(project) => {
            if project.repo_url != repo_url {
                bail!(
                    "repository slug collision: {repo_url:?} and {:?} both become {slug:?}",
                    project.repo_url
                );
            }
            project
        }
        None => {
            catalog.projects.push(CatalogProject {
                slug: slug.to_owned(),
                repo_url: repo_url.to_owned(),
                commits: Vec::new(),
            });
            catalog.projects.last_mut().unwrap()
        }
    };
    let entry = CatalogCommit {
        commit: commit.to_owned(),
        title: title.to_owned(),
        file_count,
        occurrence_count,
    };
    if let Some(existing) = project
        .commits
        .iter_mut()
        .find(|item| item.commit == commit)
    {
        *existing = entry;
    } else {
        project.commits.push(entry);
    }
    project.commits.sort_by(|a, b| b.commit.cmp(&a.commit));
    catalog.projects.sort_by(|a, b| a.slug.cmp(&b.slug));
    write_json(&path, &catalog)?;
    write_javascript(
        &web_root.join("generated/catalog.js"),
        "__SCIP_CATALOG__",
        &catalog,
    )
}

fn validate_route_segment(value: &str, name: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("{name} must contain only ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(())
}

fn slug_repo_url(repo_url: &str) -> Result<String> {
    let input = repo_url.trim();
    let normalized = Url::parse(input)
        .ok()
        .and_then(|url| url.host_str().map(|host| format!("{}{}", host, url.path())))
        .unwrap_or_else(|| {
            input.split_once('@').map_or_else(
                || input.to_owned(),
                |(_, host_path)| host_path.replacen(':', "/", 1),
            )
        });
    let trimmed = normalized.trim_end_matches('/').trim_end_matches(".git");
    if trimmed.is_empty() {
        bail!("--repo-url must not be empty");
    }
    let mut slug = String::with_capacity(trimmed.len());
    let mut separator = false;
    for character in trimmed.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            separator = false;
        } else if !slug.is_empty() && !separator {
            slug.push('-');
            separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        bail!("--repo-url does not contain any slug characters");
    }
    Ok(slug)
}

fn validate_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("unsafe SCIP document path: {value:?}");
    }
    Ok(())
}

fn infer_source_root(index: &Index) -> Option<PathBuf> {
    let root = &index.metadata.as_ref()?.project_root;
    let url = Url::parse(root).ok()?;
    url.to_file_path().ok()
}

fn read_source(
    document: &Document,
    source_root: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Result<String> {
    if let Some(root) = source_root {
        let path = root.join(&document.relative_path);
        match fs::read_to_string(&path) {
            Ok(text) => return Ok(text),
            Err(error) if !document.text.is_empty() => warnings.push(format!(
                "could not read {} ({error}); using text embedded in SCIP",
                path.display()
            )),
            Err(error) => bail!("failed to read source {}: {error}", path.display()),
        }
    }
    if document.text.is_empty() {
        bail!(
            "{} has no embedded text; pass --source-root",
            document.relative_path
        );
    }
    Ok(document.text.clone())
}

fn collect_definitions(documents: &[Document], sources: &[String]) -> Definitions {
    let mut result = Definitions::new(documents.len());
    for (file, document) in documents.iter().enumerate() {
        let lines: Vec<&str> = sources[file].split('\n').collect();
        for occurrence in &document.occurrences {
            if occurrence.symbol.is_empty() || occurrence.symbol_roles & DEFINITION_ROLE == 0 {
                continue;
            }
            let Some(range) = occurrence_range(occurrence) else {
                continue;
            };
            let character = utf16_column(
                lines.get(range.start_line).copied().unwrap_or(""),
                range.start_character,
                document.position_encoding.enum_value_or_default(),
            );
            result.insert(
                file,
                &occurrence.symbol,
                Definition {
                    file,
                    line: range.start_line,
                    character,
                },
            );
        }
    }
    result
}

fn make_file_data<'a>(
    file_id: usize,
    document: &'a Document,
    source: &'a str,
    definitions: &Definitions,
) -> FileData<'a> {
    let lines: Vec<&str> = source.split('\n').collect();
    let encoding = document.position_encoding.enum_value_or_default();
    let mut symbol_ids: HashMap<&str, usize> = HashMap::new();
    let mut symbols = Vec::new();
    let mut occurrences = Vec::new();

    for occurrence in &document.occurrences {
        if occurrence.symbol.is_empty() && occurrence.syntax_kind.value() == 0 {
            continue;
        }
        let Some(range) = occurrence_range(occurrence) else {
            continue;
        };
        if range.start_line >= lines.len() || range.end_line >= lines.len() {
            continue;
        }
        let symbol_id = if occurrence.symbol.is_empty() {
            -1
        } else {
            *symbol_ids.entry(&occurrence.symbol).or_insert_with(|| {
                let id = symbols.len();
                symbols.push(occurrence.symbol.as_str());
                id
            }) as i64
        };
        let start = utf16_column(lines[range.start_line], range.start_character, encoding);
        let end = utf16_column(lines[range.end_line], range.end_character, encoding);
        let target = (!occurrence.symbol.is_empty())
            .then(|| definitions.get(file_id, &occurrence.symbol))
            .flatten();
        occurrences.push([
            range.start_line as i64,
            start as i64,
            range.end_line as i64,
            end as i64,
            symbol_id,
            target.map_or(-1, |value| value.file as i64),
            target.map_or(-1, |value| value.line as i64),
            target.map_or(-1, |value| value.character as i64),
            occurrence.symbol_roles as i64,
            occurrence.syntax_kind.value() as i64,
        ]);
    }
    occurrences.sort_unstable_by_key(|item| (item[0], item[1], item[2], item[3]));

    FileData {
        path: &document.relative_path,
        language: &document.language,
        text: source,
        symbols,
        occurrences,
    }
}

fn occurrence_range(value: &Occurrence) -> Option<Range> {
    match &value.typed_range {
        Some(occurrence::Typed_range::SingleLineRange(value)) => {
            return Some(Range {
                start_line: usize::try_from(value.line).ok()?,
                start_character: usize::try_from(value.start_character).ok()?,
                end_line: usize::try_from(value.line).ok()?,
                end_character: usize::try_from(value.end_character).ok()?,
            });
        }
        Some(occurrence::Typed_range::MultiLineRange(value)) => {
            return Some(Range {
                start_line: usize::try_from(value.start_line).ok()?,
                start_character: usize::try_from(value.start_character).ok()?,
                end_line: usize::try_from(value.end_line).ok()?,
                end_character: usize::try_from(value.end_character).ok()?,
            });
        }
        _ => {}
    }
    match value.range.as_slice() {
        [line, start, end] => Some(Range {
            start_line: usize::try_from(*line).ok()?,
            start_character: usize::try_from(*start).ok()?,
            end_line: usize::try_from(*line).ok()?,
            end_character: usize::try_from(*end).ok()?,
        }),
        [start_line, start, end_line, end] => Some(Range {
            start_line: usize::try_from(*start_line).ok()?,
            start_character: usize::try_from(*start).ok()?,
            end_line: usize::try_from(*end_line).ok()?,
            end_character: usize::try_from(*end).ok()?,
        }),
        _ => None,
    }
}

fn utf16_column(line: &str, column: usize, encoding: PositionEncoding) -> usize {
    match encoding {
        PositionEncoding::UTF16CodeUnitOffsetFromLineStart => {
            column.min(line.encode_utf16().count())
        }
        PositionEncoding::UTF32CodeUnitOffsetFromLineStart => {
            line.chars().take(column).map(char::len_utf16).sum()
        }
        PositionEncoding::UTF8CodeUnitOffsetFromLineStart
        | PositionEncoding::UnspecifiedPositionEncoding => {
            let mut byte = column.min(line.len());
            while byte > 0 && !line.is_char_boundary(byte) {
                byte -= 1;
            }
            line[..byte].encode_utf16().count()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_position_encodings_to_browser_columns() {
        let line = "a🚀b";
        assert_eq!(
            utf16_column(line, 5, PositionEncoding::UTF8CodeUnitOffsetFromLineStart),
            3
        );
        assert_eq!(
            utf16_column(line, 2, PositionEncoding::UTF32CodeUnitOffsetFromLineStart),
            3
        );
        assert_eq!(
            utf16_column(line, 3, PositionEncoding::UTF16CodeUnitOffsetFromLineStart),
            3
        );
    }

    #[test]
    fn rejects_paths_outside_source_root() {
        assert!(validate_relative_path("src/main.rs").is_ok());
        assert!(validate_relative_path("../secret").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn scopes_local_symbols_to_their_document() {
        let first = Definition {
            file: 0,
            line: 1,
            character: 2,
        };
        let second = Definition {
            file: 1,
            line: 3,
            character: 4,
        };
        let mut definitions = Definitions::new(2);
        definitions.insert(0, "local 0", first);
        definitions.insert(1, "local 0", second);
        definitions.insert(0, "scip . . . Foo#", first);

        assert_eq!(definitions.get(0, "local 0").unwrap().file, 0);
        assert_eq!(definitions.get(1, "local 0").unwrap().file, 1);
        assert_eq!(definitions.get(1, "scip . . . Foo#").unwrap().file, 0);
    }

    #[test]
    fn creates_stable_repository_slugs() {
        assert_eq!(
            slug_repo_url("https://GitHub.com/harfbuzz/harfbuzz.git/").unwrap(),
            "github-com-harfbuzz-harfbuzz"
        );
        assert_eq!(
            slug_repo_url("git@github.com:owner/repo.git").unwrap(),
            "github-com-owner-repo"
        );
    }
}
