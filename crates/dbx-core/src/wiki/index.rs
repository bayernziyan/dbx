use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiIndexEntry {
    pub relative_path: String,
    pub kind: String,
    pub content_hash: String,
}
