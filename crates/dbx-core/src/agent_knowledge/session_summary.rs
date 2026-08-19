use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeCandidate {
    pub scope_id: String,
    #[serde(default)]
    pub facts: Vec<KnowledgeFact>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub rejected_inferences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeFact {
    pub subject: String,
    pub value: String,
    pub status: String,
    #[serde(default)]
    pub citation_ids: Vec<String>,
}
