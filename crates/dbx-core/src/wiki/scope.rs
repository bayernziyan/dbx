use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiScopeStatus {
    pub scope_id: String,
    pub policy_id: String,
    pub document_count: usize,
    pub manifest_present: bool,
}
