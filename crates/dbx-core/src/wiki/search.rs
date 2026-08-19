use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiSearchHit {
    pub relative_path: String,
    pub locator: String,
    pub text: String,
    pub score: f32,
}
