use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiManifest {
    pub manifest_version: u32,
    pub wiki_schema_version: u32,
    pub entry: String,
    #[serde(default)]
    pub documents: Vec<WikiManifestDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiManifestDocument {
    pub path: String,
    pub kind: String,
    pub sha256: String,
}

pub fn load_manifest(root: &Path) -> Result<Option<WikiManifest>, String> {
    let path = root.join(".dbx-wiki").join("manifest.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("MANIFEST_READ_FAILED: {}: {error}", path.display())),
    };
    serde_json::from_str(&text).map(Some).map_err(|error| format!("MANIFEST_INVALID: {error}"))
}
