use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWriteAudit {
    pub scope_id: String,
    pub policy_id: String,
    pub relative_path: String,
    pub previous_hash: Option<String>,
    pub content_hash: String,
    pub hooks: Vec<String>,
}
