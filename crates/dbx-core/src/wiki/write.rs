use std::path::Path;

pub fn content_hash(path: &Path) -> Result<String, String> {
    crate::agent_files::file_write::file_sha256(path)
}
