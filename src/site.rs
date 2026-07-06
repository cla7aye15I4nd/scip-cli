use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use protobuf::Message;
use scip::types::{Document, Index, Occurrence, PositionEncoding, SymbolInformation, occurrence};
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

fn write_site_shell(web_root: &Path) -> Result<()> {
    fs::create_dir_all(web_root.join("assets"))?;
    let legacy_catalog = web_root.join("generated/catalog.js");
    if legacy_catalog.is_file() {
        fs::remove_file(&legacy_catalog)
            .with_context(|| format!("failed to remove {}", legacy_catalog.display()))?;
    }
    fs::write(
        web_root.join("index.html"),
        include_str!("../assets/index.html"),
    )?;
    fs::write(
        web_root.join("404.html"),
        include_str!("../assets/index.html"),
    )?;
    fs::write(web_root.join("_redirects"), "/* /index.html 200\n")?;
    fs::write(
        web_root.join("assets/app.js"),
        include_str!("../assets/app.js"),
    )?;
    fs::write(
        web_root.join("assets/style.css"),
        include_str!("../assets/style.css"),
    )?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
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

struct Definitions<'a> {
    global: HashMap<&'a str, Definition>,
    local: Vec<HashMap<&'a str, Definition>>,
}

impl<'a> Definitions<'a> {
    fn new(file_count: usize) -> Self {
        Self {
            global: HashMap::new(),
            local: (0..file_count).map(|_| HashMap::new()).collect(),
        }
    }

    fn insert(&mut self, file: usize, symbol: &'a str, definition: Definition) {
        let map = if symbol.starts_with("local ") {
            &mut self.local[file]
        } else {
            &mut self.global
        };
        map.entry(symbol).or_insert(definition);
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
#[serde(rename_all = "camelCase")]
struct FileData<'a> {
    path: &'a str,
    language: &'a str,
    text: &'a str,
    symbols: Vec<&'a str>,
    docs: BTreeMap<&'a str, String>,
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
    drop(bytes);
    if index.documents.is_empty() {
        bail!("SCIP index contains no documents");
    }

    let mut warnings = Vec::new();
    index.documents = deduplicate_documents(index.documents, &mut warnings);
    for document in &index.documents {
        validate_relative_path(&document.relative_path)?;
    }

    let source_root = options.source_root.or_else(|| infer_source_root(&index));
    let sources: Vec<String> = index
        .documents
        .iter_mut()
        .map(|document| read_source(document, source_root.as_deref(), &mut warnings))
        .collect::<Result<_>>()?;

    let definitions = collect_definitions(&index.documents, &sources);
    let project_root = options.web_root.join("generated").join(&repo_slug);
    let generated = project_root.join(&options.commit);
    let staging = project_root.join(format!(
        ".{}.staging-{}",
        options.commit,
        std::process::id()
    ));
    write_site_shell(&options.web_root)?;
    if staging.exists() {
        fs::remove_dir_all(&staging).with_context(|| {
            format!(
                "failed to clear stale staging directory {}",
                staging.display()
            )
        })?;
    }
    fs::create_dir_all(staging.join("files"))?;

    let mut occurrence_count = 0;
    for (file_id, document) in index.documents.iter().enumerate() {
        let data = make_file_data(file_id, document, &sources[file_id], &definitions);
        occurrence_count += data.occurrences.len();
        write_json(&staging.join(format!("files/{file_id}.json")), &data)?;
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
    write_json(&staging.join("manifest.json"), &manifest)?;
    publish_directory(&staging, &generated)?;
    update_catalog(
        &options.web_root,
        &repo_slug,
        &options.repo_url,
        &options.commit,
        &options.title,
        index.documents.len(),
        occurrence_count,
    )?;
    prune_old_commits(&project_root, &options.commit)?;

    Ok(BuildReport {
        files: index.documents.len(),
        occurrences: occurrence_count,
        output: options.web_root,
        warnings,
    })
}

/// SCIP producers can emit the same document once per translation unit. Routes are
/// path-based, so allowing duplicate paths into the manifest makes all but the first
/// copy unreachable and can point definitions at a different copy than the UI opens.
fn deduplicate_documents(
    mut documents: Vec<Document>,
    warnings: &mut Vec<String>,
) -> Vec<Document> {
    documents.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut unique = Vec::with_capacity(documents.len());
    let mut needs_deduplication = Vec::with_capacity(documents.len());
    let mut duplicates = 0usize;

    for document in documents {
        let Some(existing) = unique.last_mut() else {
            unique.push(document);
            needs_deduplication.push(false);
            continue;
        };
        if existing.relative_path != document.relative_path {
            unique.push(document);
            needs_deduplication.push(false);
            continue;
        }

        duplicates += 1;
        *needs_deduplication.last_mut().unwrap() = true;
        if existing.position_encoding != document.position_encoding {
            if document.occurrences.len() > existing.occurrences.len() {
                *existing = document;
            }
            warnings.push(format!(
                "{} was indexed with conflicting position encodings; kept the most complete copy",
                existing.relative_path
            ));
            continue;
        }

        if existing.language.is_empty() {
            existing.language = document.language;
        }
        if document.text.len() > existing.text.len() {
            existing.text = document.text;
        }
        existing.occurrences.extend(document.occurrences);
        existing.symbols.extend(document.symbols);
    }

    for (document, needed) in unique.iter_mut().zip(needs_deduplication) {
        if needed {
            deduplicate_messages(&mut document.occurrences);
            deduplicate_messages(&mut document.symbols);
        }
    }

    if duplicates > 0 {
        warnings.push(format!(
            "consolidated {duplicates} duplicate SCIP document entr{} by relative path",
            if duplicates == 1 { "y" } else { "ies" }
        ));
    }
    unique
}

fn deduplicate_messages<T>(values: &mut Vec<T>)
where
    T: Message,
{
    let mut seen = HashSet::<(u64, u64)>::with_capacity(values.len());
    values.retain(|value| {
        let bytes = value.write_to_bytes().unwrap_or_default();
        let mut first = DefaultHasher::new();
        bytes.hash(&mut first);
        let mut second = DefaultHasher::new();
        1_u8.hash(&mut second);
        bytes.hash(&mut second);
        seen.insert((first.finish(), second.finish()))
    });
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    serde_json::to_writer(file, value)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("JSON output path has no UTF-8 file name")?;
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    write_json(&temporary, value)?;
    fs::rename(&temporary, path).with_context(|| {
        format!(
            "failed to publish {} as {}",
            temporary.display(),
            path.display()
        )
    })
}

fn publish_directory(staging: &Path, destination: &Path) -> Result<()> {
    let backup = destination.with_file_name(format!(
        ".{}.backup-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .context("generated directory has no UTF-8 file name")?,
        std::process::id()
    ));
    if backup.exists() {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to clear stale backup {}", backup.display()))?;
    }
    let had_destination = destination.exists();
    if had_destination {
        fs::rename(destination, &backup).with_context(|| {
            format!(
                "failed to stage existing generated directory {}",
                destination.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(staging, destination) {
        if had_destination {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error).with_context(|| {
            format!(
                "failed to publish generated directory {}",
                destination.display()
            )
        });
    }
    if had_destination {
        fs::remove_dir_all(&backup)
            .with_context(|| format!("failed to remove old generated data {}", backup.display()))?;
    }
    Ok(())
}

fn prune_old_commits(project_root: &Path, current_commit: &str) -> Result<()> {
    let entries = match fs::read_dir(project_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read {}", project_root.display()));
        }
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && entry.file_name() != current_commit
            && !entry.file_name().to_string_lossy().starts_with('.')
        {
            fs::remove_dir_all(entry.path()).with_context(|| {
                format!(
                    "failed to remove old commit data {}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
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
    project.commits.clear();
    project.commits.push(entry);
    catalog.projects.sort_by(|a, b| a.slug.cmp(&b.slug));
    write_json_atomic(&path, &catalog)
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
    document: &mut Document,
    source_root: Option<&Path>,
    warnings: &mut Vec<String>,
) -> Result<String> {
    if let Some(root) = source_root {
        let path = root.join(&document.relative_path);
        match fs::read_to_string(&path) {
            Ok(text) => {
                document.text.clear();
                return Ok(text);
            }
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
    Ok(std::mem::take(&mut document.text))
}

fn collect_definitions<'a>(documents: &'a [Document], sources: &[String]) -> Definitions<'a> {
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
    definitions: &Definitions<'_>,
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
    let occurrences = consolidate_render_occurrences(occurrences);
    let docs = symbol_documentation(&document.symbols);

    FileData {
        path: &document.relative_path,
        language: &document.language,
        text: source,
        symbols,
        docs,
        occurrences,
    }
}

fn symbol_documentation(symbols: &[SymbolInformation]) -> BTreeMap<&str, String> {
    symbols
        .iter()
        .filter_map(|information| {
            if information.symbol.is_empty() {
                return None;
            }
            let markdown = information
                .documentation
                .iter()
                .map(|section| section.trim())
                .filter(|section| !section.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!markdown.is_empty()).then_some((information.symbol.as_str(), markdown))
        })
        .collect()
}

fn consolidate_render_occurrences(occurrences: Vec<[i64; 10]>) -> Vec<[i64; 10]> {
    let mut ranges = BTreeMap::<[i64; 4], [i64; 10]>::new();
    for mut candidate in occurrences {
        let range = [candidate[0], candidate[1], candidate[2], candidate[3]];
        let Some(existing) = ranges.get_mut(&range) else {
            ranges.insert(range, candidate);
            continue;
        };

        // One source range may have separate syntax-only, macro, and semantic
        // occurrences. The browser can render only one anchor, so prefer the one
        // that can navigate inside this index, then retain its syntax highlighting.
        let candidate_priority = (candidate[5] >= 0, candidate[4] >= 0, candidate[8] & 1 != 0);
        let existing_priority = (existing[5] >= 0, existing[4] >= 0, existing[8] & 1 != 0);
        if candidate_priority > existing_priority {
            if candidate[9] == 0 {
                candidate[9] = existing[9];
            }
            *existing = candidate;
        } else if existing[9] == 0 {
            existing[9] = candidate[9];
        }
    }
    ranges.into_values().collect()
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

    #[test]
    fn catalog_keeps_only_the_most_recently_generated_revision() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("generated")).unwrap();

        update_catalog(
            root.path(),
            "repo",
            "https://example.com/repo",
            "aaa",
            "Repo",
            1,
            2,
        )
        .unwrap();
        update_catalog(
            root.path(),
            "repo",
            "https://example.com/repo",
            "bbb",
            "Repo",
            3,
            4,
        )
        .unwrap();
        update_catalog(
            root.path(),
            "repo",
            "https://example.com/repo",
            "aaa",
            "Repo",
            5,
            6,
        )
        .unwrap();

        let catalog: Catalog =
            serde_json::from_slice(&fs::read(root.path().join("generated/catalog.json")).unwrap())
                .unwrap();
        assert_eq!(catalog.projects[0].commits[0].commit, "aaa");
        assert_eq!(catalog.projects[0].commits[0].file_count, 5);
        assert_eq!(catalog.projects[0].commits.len(), 1);
    }

    #[test]
    fn removes_old_commit_directories() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("old")).unwrap();
        fs::create_dir_all(root.path().join("current")).unwrap();

        prune_old_commits(root.path(), "current").unwrap();

        assert!(!root.path().join("old").exists());
        assert!(root.path().join("current").is_dir());
    }

    #[test]
    fn publishes_generated_directory_without_leaving_old_files() {
        let root = tempfile::tempdir().unwrap();
        let staging = root.path().join(".current.staging");
        let destination = root.path().join("current");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(staging.join("new.json"), "new").unwrap();
        fs::write(destination.join("obsolete.js"), "old").unwrap();

        publish_directory(&staging, &destination).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("new.json")).unwrap(),
            "new"
        );
        assert!(!destination.join("obsolete.js").exists());
        assert!(!staging.exists());
    }

    #[test]
    fn builds_json_only_static_site() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("index.scip");
        let web_root = root.path().join("web");
        let index = Index {
            documents: vec![Document {
                relative_path: "src/main.c".to_owned(),
                language: "C".to_owned(),
                text: "int main(void) { return 0; }\n".to_owned(),
                ..Document::default()
            }],
            ..Index::default()
        };
        fs::write(&input, index.write_to_bytes().unwrap()).unwrap();

        build(BuildOptions {
            input,
            source_root: None,
            web_root: web_root.clone(),
            repo_url: "https://example.com/repo.git".to_owned(),
            commit: "abc123".to_owned(),
            title: "Repo".to_owned(),
        })
        .unwrap();

        let generated = web_root.join("generated/example-com-repo/abc123");
        assert!(generated.join("manifest.json").is_file());
        assert!(generated.join("files/0.json").is_file());
        assert!(!generated.join("manifest.js").exists());
        assert!(!generated.join("files/0.js").exists());
        assert!(!web_root.join("generated/catalog.js").exists());
    }

    #[test]
    fn consolidates_duplicate_document_paths() {
        let mut first = Document {
            relative_path: "tools/tiff_tools.c".to_owned(),
            language: "C".to_owned(),
            ..Document::default()
        };
        first.occurrences.push(Occurrence {
            range: vec![0, 0, 3],
            symbol: "scip . . . first".to_owned(),
            ..Occurrence::default()
        });
        let mut second = first.clone();
        second.occurrences.push(Occurrence {
            range: vec![1, 0, 3],
            symbol: "scip . . . second".to_owned(),
            ..Occurrence::default()
        });

        let mut warnings = Vec::new();
        let documents = deduplicate_documents(vec![first, second], &mut warnings);

        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].occurrences.len(), 2);
        assert!(warnings.iter().any(|warning| warning.contains("duplicate")));
    }

    #[test]
    fn prefers_clickable_occurrence_for_the_same_range() {
        let syntax_only = [2, 4, 2, 10, -1, -1, -1, -1, 0, 17];
        let clickable = [2, 4, 2, 10, 3, 7, 12, 2, 0, 0];

        let occurrences = consolidate_render_occurrences(vec![syntax_only, clickable]);

        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0][5], 7);
        assert_eq!(occurrences[0][9], 17);
    }

    #[test]
    fn preserves_scip_symbol_documentation_for_the_browser() {
        let symbols = vec![SymbolInformation {
            symbol: "cxx . . example().".to_owned(),
            documentation: vec![
                "First paragraph.".to_owned(),
                "Second paragraph.".to_owned(),
            ],
            ..SymbolInformation::default()
        }];

        let docs = symbol_documentation(&symbols);

        assert_eq!(
            docs.get("cxx . . example().").map(String::as_str),
            Some("First paragraph.\n\nSecond paragraph.")
        );
    }
}
