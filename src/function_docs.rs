use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FUNCTION_INDEX_VERSION: u32 = 1;
pub const DOCUMENT_SCHEMA_VERSION: u32 = 2;
pub const PROMPT_VERSION: &str = "function-doc-v3";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionIndex {
    pub version: u32,
    pub repo: String,
    pub commit: String,
    pub functions: Vec<FunctionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionRecord {
    pub doc_key: String,
    pub file_id: usize,
    pub file: String,
    pub language: String,
    pub scip_symbol: String,
    pub kind: String,
    pub display_name: String,
    pub signature: String,
    pub definition_range: [usize; 4],
    pub enclosing_range: [usize; 4],
    pub existing_documentation: Vec<String>,
    pub related_symbols: Vec<String>,
    pub diagnostics: Vec<String>,
    pub source_hash: String,
}

pub fn stable_doc_key(repo: &str, commit: &str, file: &str, symbol: &str) -> String {
    let identity = if symbol.starts_with("local ") {
        format!("{repo}\0{commit}\0{file}\0{symbol}")
    } else {
        format!("{repo}\0{commit}\0{symbol}")
    };
    sha256_hex(identity.as_bytes())
}

pub fn source_hash(source: &str) -> String {
    sha256_hex(source.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_symbols_include_the_file_in_the_document_key() {
        assert_ne!(
            stable_doc_key("repo", "commit", "a.cc", "local 1"),
            stable_doc_key("repo", "commit", "b.cc", "local 1")
        );
        assert_eq!(
            stable_doc_key("repo", "commit", "a.cc", "global()."),
            stable_doc_key("repo", "commit", "b.cc", "global().")
        );
    }
}
