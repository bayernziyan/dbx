use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSession {
    pub session_id: String,
    pub goal: String,
    pub iteration: u32,
    #[serde(default)]
    pub evidence: Vec<EvidenceReference>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReference {
    pub id: String,
    pub scope_id: String,
    pub relative_path: String,
    pub content_hash: String,
    pub locator: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenQuestion {
    pub question: String,
    pub status: String,
}
