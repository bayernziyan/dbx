use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownSection {
    pub heading: String,
    pub level: usize,
    pub body: String,
}

pub fn sections(markdown: &str) -> Vec<MarkdownSection> {
    let mut result = Vec::new();
    let mut heading = String::new();
    let mut level = 0usize;
    let mut body = Vec::new();
    for line in markdown.lines() {
        let hashes = line.chars().take_while(|character| *character == '#').count();
        if hashes > 0 && hashes <= 6 && line.chars().nth(hashes) == Some(' ') {
            if !heading.is_empty() || !body.is_empty() {
                result.push(MarkdownSection { heading, level, body: body.join("\n") });
            }
            heading = line[hashes + 1..].trim().to_string();
            level = hashes;
            body = Vec::new();
        } else {
            body.push(line.to_string());
        }
    }
    if !heading.is_empty() || !body.is_empty() {
        result.push(MarkdownSection { heading, level, body: body.join("\n") });
    }
    result
}
