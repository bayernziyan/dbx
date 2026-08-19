use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiFeedback {
    pub scope_id: String,
    pub category: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
}
