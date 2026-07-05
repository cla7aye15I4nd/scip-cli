use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use protobuf::Message;
use scip::types::{
    Document, Index, Occurrence, PositionEncoding, SymbolInformation, SyntaxKind, occurrence,
    symbol_information,
};
use serde::Serialize;
use url::Url;

use crate::function_docs::{
    FUNCTION_INDEX_VERSION, FunctionIndex, FunctionRecord, source_hash, stable_doc_key,
};

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
#[serde(rename_all = "camelCase")]
struct FileData<'a> {
    path: &'a str,
    language: &'a str,
    text: &'a str,
    symbols: Vec<&'a str>,
    /// [start line, start UTF-16 column, end line, end UTF-16 column,
    ///  local symbol id, target file, target line, target UTF-16 column, roles,
    ///  SCIP syntax kind]
    occurrences: Vec<[i64; 10]>,
    function_doc_keys: BTreeMap<String, String>,
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

    let mut warnings = Vec::new();
    index.documents = deduplicate_documents(index.documents, &mut warnings);
    for document in &index.documents {
        validate_relative_path(&document.relative_path)?;
    }

    let source_root = options.source_root.or_else(|| infer_source_root(&index));
    let sources: Vec<String> = index
        .documents
        .iter()
        .map(|document| read_source(document, source_root.as_deref(), &mut warnings))
        .collect::<Result<_>>()?;

    let definitions = collect_definitions(&index.documents, &sources);
    let function_index = collect_function_index(&repo_slug, &options.commit, &index, &sources);
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
        let data = make_file_data(
            file_id,
            document,
            &sources[file_id],
            &definitions,
            &function_index,
        );
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
    write_json(&generated.join("functions.json"), &function_index)?;
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

/// SCIP producers can emit the same document once per translation unit. Routes are
/// path-based, so allowing duplicate paths into the manifest makes all but the first
/// copy unreachable and can point definitions at a different copy than the UI opens.
fn deduplicate_documents(
    mut documents: Vec<Document>,
    warnings: &mut Vec<String>,
) -> Vec<Document> {
    documents.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut unique = Vec::with_capacity(documents.len());
    let mut duplicates = 0usize;

    for document in documents {
        let Some(existing) = unique.last_mut() else {
            unique.push(document);
            continue;
        };
        if existing.relative_path != document.relative_path {
            unique.push(document);
            continue;
        }

        duplicates += 1;
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

        existing
            .occurrences
            .sort_by_cached_key(|item| item.write_to_bytes().unwrap_or_default());
        existing.occurrences.dedup();
        existing
            .symbols
            .sort_by_cached_key(|item| item.write_to_bytes().unwrap_or_default());
        existing.symbols.dedup();
    }

    if duplicates > 0 {
        warnings.push(format!(
            "consolidated {duplicates} duplicate SCIP document entr{} by relative path",
            if duplicates == 1 { "y" } else { "ies" }
        ));
    }
    unique
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
    project.commits.retain(|item| item.commit != commit);
    // Generation order, rather than lexical SHA order, defines the current revision.
    // Historical revisions remain addressable by URL but are not promoted in the UI.
    project.commits.insert(0, entry);
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

fn collect_function_index(
    repo: &str,
    commit: &str,
    index: &Index,
    sources: &[String],
) -> FunctionIndex {
    let mut information = HashMap::<&str, &SymbolInformation>::new();
    for symbol in index
        .documents
        .iter()
        .flat_map(|document| document.symbols.iter())
        .chain(index.external_symbols.iter())
    {
        information.entry(symbol.symbol.as_str()).or_insert(symbol);
    }

    let mut functions = BTreeMap::<String, FunctionRecord>::new();
    for (file_id, document) in index.documents.iter().enumerate() {
        let source = &sources[file_id];
        let lines: Vec<&str> = source.split('\n').collect();
        let encoding = document.position_encoding.enum_value_or_default();
        for occurrence in &document.occurrences {
            if occurrence.symbol.is_empty() || occurrence.symbol_roles & DEFINITION_ROLE == 0 {
                continue;
            }
            let info = information.get(occurrence.symbol.as_str()).copied();
            let Some(raw_definition) = occurrence_range(occurrence) else {
                continue;
            };
            if raw_definition.start_line >= lines.len() || raw_definition.end_line >= lines.len() {
                continue;
            }
            if !is_function_symbol(occurrence, document, info, &lines, raw_definition) {
                continue;
            }
            let Some(raw_enclosing) = occurrence_enclosing_range(occurrence)
                .filter(|range| range.start_line < lines.len() && range.end_line < lines.len())
                .or_else(|| infer_function_range(&lines, raw_definition, &document.language))
            else {
                continue;
            };
            let definition_range = browser_range(raw_definition, &lines, encoding);
            let enclosing_range = browser_range(raw_enclosing, &lines, encoding);
            let snippet = source_lines(source, raw_enclosing);
            let doc_key = stable_doc_key(repo, commit, &document.relative_path, &occurrence.symbol);
            let signature = info
                .and_then(|value| value.signature_documentation.as_ref())
                .map(|value| value.text.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| lines[raw_definition.start_line].trim())
                .to_owned();
            let display_name = info
                .map(|value| value.display_name.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    let line = lines[raw_definition.start_line];
                    let start = raw_definition.start_character.min(line.len());
                    let end = raw_definition.end_character.min(line.len()).max(start);
                    line.get(start..end).unwrap_or(&occurrence.symbol)
                })
                .to_owned();
            let mut related_symbols = document
                .occurrences
                .iter()
                .filter(|candidate| {
                    !candidate.symbol.is_empty()
                        && candidate.symbol != occurrence.symbol
                        && occurrence_range(candidate)
                            .is_some_and(|range| range_within(range, raw_enclosing))
                })
                .map(|candidate| candidate.symbol.clone())
                .collect::<Vec<_>>();
            if let Some(info) = info {
                related_symbols.extend(
                    info.relationships
                        .iter()
                        .map(|relationship| relationship.symbol.clone()),
                );
            }
            related_symbols.sort();
            related_symbols.dedup();
            functions
                .entry(doc_key.clone())
                .or_insert_with(|| FunctionRecord {
                    doc_key,
                    file_id,
                    file: document.relative_path.clone(),
                    language: document.language.clone(),
                    scip_symbol: occurrence.symbol.clone(),
                    kind: info
                        .map(|value| format!("{:?}", value.kind.enum_value_or_default()))
                        .unwrap_or_else(|| "Function".to_owned()),
                    display_name,
                    signature,
                    definition_range,
                    enclosing_range,
                    existing_documentation: info
                        .map(|value| value.documentation.clone())
                        .unwrap_or_default(),
                    related_symbols,
                    diagnostics: occurrence
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.message.clone())
                        .collect(),
                    source_hash: source_hash(&snippet),
                });
        }
    }

    FunctionIndex {
        version: FUNCTION_INDEX_VERSION,
        repo: repo.to_owned(),
        commit: commit.to_owned(),
        functions: functions.into_values().collect(),
    }
}

fn is_function_symbol(
    occurrence: &Occurrence,
    document: &Document,
    info: Option<&SymbolInformation>,
    lines: &[&str],
    definition: Range,
) -> bool {
    let kind = info.map(|value| value.kind.enum_value_or_default());
    matches!(
        kind,
        Some(
            symbol_information::Kind::Function
                | symbol_information::Kind::Method
                | symbol_information::Kind::Constructor
                | symbol_information::Kind::Getter
                | symbol_information::Kind::Setter
                | symbol_information::Kind::AbstractMethod
        )
    ) || document.occurrences.iter().any(|candidate| {
        candidate.syntax_kind.enum_value_or_default() == SyntaxKind::IdentifierFunctionDefinition
            && occurrence_range(candidate) == occurrence_range(occurrence)
    }) || (kind == Some(symbol_information::Kind::UnspecifiedKind) || kind.is_none())
        && (occurrence.symbol.ends_with(").")
            || looks_like_function_definition(occurrence, lines, definition))
}

fn infer_function_range(lines: &[&str], definition: Range, language: &str) -> Option<Range> {
    let start = definition.start_line;
    if language.eq_ignore_ascii_case("python") {
        let base_indent = lines
            .get(start)?
            .chars()
            .take_while(|character| character.is_whitespace())
            .count();
        for (line, text) in lines
            .iter()
            .enumerate()
            .take(lines.len().min(start + 2000))
            .skip(start + 1)
        {
            if text.trim().is_empty() || text.trim_start().starts_with('#') {
                continue;
            }
            let indent = text
                .chars()
                .take_while(|character| character.is_whitespace())
                .count();
            if indent <= base_indent {
                return Some(Range {
                    start_line: start,
                    start_character: 0,
                    end_line: line.saturating_sub(1),
                    end_character: lines[line.saturating_sub(1)].len(),
                });
            }
        }
        return None;
    }

    let mut opened = false;
    let mut depth = 0isize;
    let mut block_comment = false;
    let mut quote = None;
    let mut escaped = false;
    for (line, text) in lines
        .iter()
        .enumerate()
        .take(lines.len().min(start + 2000))
        .skip(start)
    {
        let characters = text.as_bytes();
        let mut index = 0usize;
        while index < characters.len() {
            let character = characters[index] as char;
            let next = characters.get(index + 1).copied().map(char::from);
            if block_comment {
                if character == '*' && next == Some('/') {
                    block_comment = false;
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            if let Some(delimiter) = quote {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == delimiter {
                    quote = None;
                }
                index += 1;
                continue;
            }
            if character == '/' && next == Some('/') {
                break;
            }
            if character == '/' && next == Some('*') {
                block_comment = true;
                index += 2;
                continue;
            }
            if character == '\'' || character == '"' {
                quote = Some(character);
                index += 1;
                continue;
            }
            match character {
                '{' => {
                    opened = true;
                    depth += 1;
                }
                '}' if opened => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(Range {
                            start_line: start,
                            start_character: 0,
                            end_line: line,
                            end_character: text.len(),
                        });
                    }
                }
                ';' if !opened && line == start => return None,
                _ => {}
            }
            index += 1;
        }
    }
    None
}

fn looks_like_function_definition(
    occurrence: &Occurrence,
    lines: &[&str],
    definition: Range,
) -> bool {
    if matches!(
        occurrence.syntax_kind.enum_value_or_default(),
        SyntaxKind::IdentifierMacro | SyntaxKind::IdentifierMacroDefinition
    ) {
        return false;
    }
    let Some(enclosing) = occurrence_enclosing_range(occurrence) else {
        return false;
    };
    if enclosing.end_line < definition.end_line
        || (enclosing.start_line, enclosing.start_character)
            > (definition.start_line, definition.start_character)
    {
        return false;
    }
    let line = lines.get(definition.end_line).copied().unwrap_or("");
    let mut end = definition.end_character.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[end..].trim_start().starts_with('(')
}

fn browser_range(range: Range, lines: &[&str], encoding: PositionEncoding) -> [usize; 4] {
    [
        range.start_line,
        utf16_column(lines[range.start_line], range.start_character, encoding),
        range.end_line,
        utf16_column(lines[range.end_line], range.end_character, encoding),
    ]
}

fn range_within(inner: Range, outer: Range) -> bool {
    (inner.start_line, inner.start_character) >= (outer.start_line, outer.start_character)
        && (inner.end_line, inner.end_character) <= (outer.end_line, outer.end_character)
}

fn source_lines(source: &str, range: Range) -> String {
    let lines: Vec<&str> = source.split('\n').collect();
    if lines.is_empty() || range.start_line >= lines.len() {
        return String::new();
    }
    let end = range.end_line.min(lines.len() - 1).max(range.start_line);
    lines[range.start_line..=end].join("\n")
}

fn make_file_data<'a>(
    file_id: usize,
    document: &'a Document,
    source: &'a str,
    definitions: &Definitions,
    function_index: &FunctionIndex,
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

    FileData {
        path: &document.relative_path,
        language: &document.language,
        text: source,
        symbols,
        occurrences,
        function_doc_keys: function_index
            .functions
            .iter()
            .filter(|function| function.file_id == file_id)
            .map(|function| (function.scip_symbol.clone(), function.doc_key.clone()))
            .collect(),
    }
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

fn occurrence_enclosing_range(value: &Occurrence) -> Option<Range> {
    match &value.typed_enclosing_range {
        Some(occurrence::Typed_enclosing_range::SingleLineEnclosingRange(value)) => {
            return Some(Range {
                start_line: usize::try_from(value.line).ok()?,
                start_character: usize::try_from(value.start_character).ok()?,
                end_line: usize::try_from(value.line).ok()?,
                end_character: usize::try_from(value.end_character).ok()?,
            });
        }
        Some(occurrence::Typed_enclosing_range::MultiLineEnclosingRange(value)) => {
            return Some(Range {
                start_line: usize::try_from(value.start_line).ok()?,
                start_character: usize::try_from(value.start_character).ok()?,
                end_line: usize::try_from(value.end_line).ok()?,
                end_character: usize::try_from(value.end_character).ok()?,
            });
        }
        _ => {}
    }
    match value.enclosing_range.as_slice() {
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
    fn catalog_keeps_the_most_recently_generated_revision_first() {
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
        assert_eq!(catalog.projects[0].commits[1].commit, "bbb");
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
    fn indexes_function_bodies_when_scip_omits_enclosing_ranges() {
        let source = "static int add(int a, int b)\n{\n    return a + b;\n}\n";
        let document = Document {
            relative_path: "src/add.c".to_owned(),
            language: "C".to_owned(),
            text: source.to_owned(),
            occurrences: vec![Occurrence {
                range: vec![0, 11, 14],
                symbol: "cxx . . $ add(abc123).".to_owned(),
                symbol_roles: DEFINITION_ROLE,
                ..Occurrence::default()
            }],
            ..Document::default()
        };
        let index = Index {
            documents: vec![document],
            ..Index::default()
        };

        let functions =
            collect_function_index("example-repo", "abc123", &index, &[source.to_owned()]);

        assert_eq!(functions.functions.len(), 1);
        assert_eq!(functions.functions[0].display_name, "add");
        assert_eq!(functions.functions[0].enclosing_range, [0, 0, 3, 1]);
        assert_eq!(
            functions.functions[0].signature,
            "static int add(int a, int b)"
        );
    }
}
