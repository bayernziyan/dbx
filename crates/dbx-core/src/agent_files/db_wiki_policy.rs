use std::path::Path;

use super::policy::{DirectoryAllowlistPolicy, FileScopeAccess};

pub struct DbWikiDirectoryPolicy;

const READ_EXTENSIONS: &[&str] =
    &["md", "txt", "sql", "json", "yaml", "yml", "xml", "csv", "tsv", "xls", "xlsx", "docx"];
const WRITE_EXTENSIONS: &[&str] = &["md", "txt", "sql", "json", "yaml", "yml", "xml", "csv", "tsv"];

impl DirectoryAllowlistPolicy for DbWikiDirectoryPolicy {
    fn id(&self) -> &'static str {
        "db-wiki"
    }

    fn matches(&self, canonical_root: &Path) -> bool {
        canonical_root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("db-wiki"))
    }

    fn validate_root(&self, _canonical_root: &Path) -> Result<(), String> {
        Ok(())
    }

    fn access_mode(&self) -> FileScopeAccess {
        FileScopeAccess::ReadWrite
    }

    fn allows_extension(&self, extension: &str, write: bool) -> bool {
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        let allowed = if write { WRITE_EXTENSIONS } else { READ_EXTENSIONS };
        allowed.contains(&extension.as_str())
    }

    fn after_write(&self, canonical_root: &Path, _relative_path: &Path) -> Result<Vec<String>, String> {
        super::file_write::sync_db_wiki_manifest(canonical_root)?;
        Ok(vec!["sync-manifest".to_string(), "reindex-on-next-search".to_string()])
    }
}
