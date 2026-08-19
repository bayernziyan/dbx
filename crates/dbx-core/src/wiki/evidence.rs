use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePack {
    pub schema_version: u32,
    pub scope_id: String,
    pub question: String,
    #[serde(default)]
    pub facts: Vec<WikiFact>,
    #[serde(default)]
    pub citations: Vec<WikiCitation>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub missing_evidence: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiFact {
    pub evidence_id: String,
    pub subject: String,
    pub value: String,
    pub status: String,
    #[serde(default)]
    pub citation_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiCitation {
    pub citation_id: String,
    pub relative_path: String,
    pub locator: String,
    pub content_hash: String,
}
