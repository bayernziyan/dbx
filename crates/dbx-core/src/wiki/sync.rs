use std::path::Path;

pub fn sync_manifest(root: &Path) -> Result<(), String> {
    crate::agent_files::file_write::sync_db_wiki_manifest(root)
}
