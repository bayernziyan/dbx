use std::path::Path;

use super::policy::{DirectoryAllowlistPolicy, FileScopeAccess};

pub struct ReadOnlyDirectoryPolicy;

const READ_EXTENSIONS: &[&str] =
    &["md", "txt", "sql", "json", "yaml", "yml", "xml", "csv", "tsv", "xls", "xlsx", "docx"];

impl DirectoryAllowlistPolicy for ReadOnlyDirectoryPolicy {
    fn id(&self) -> &'static str {
        "generic-read-only"
    }

    fn matches(&self, _canonical_root: &Path) -> bool {
        true
    }

    fn validate_root(&self, _canonical_root: &Path) -> Result<(), String> {
        Ok(())
    }

    fn access_mode(&self) -> FileScopeAccess {
        FileScopeAccess::ReadOnly
    }

    fn allows_extension(&self, extension: &str, write: bool) -> bool {
        !write && READ_EXTENSIONS.contains(&extension.trim_start_matches('.').to_ascii_lowercase().as_str())
    }

    fn after_write(&self, _canonical_root: &Path, _relative_path: &Path) -> Result<Vec<String>, String> {
        Err("FILE_SCOPE_READ_ONLY: this directory is not matched by a write allowlist policy".to_string())
    }
}
