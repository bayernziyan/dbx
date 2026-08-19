use std::path::Path;

use serde_json::json;

pub fn parse_text(path: &Path, limit: usize) -> Result<serde_json::Value, String> {
    let (text, byte_truncated) = super::read_text_limited(path, 512 * 1024)?;
    let all_lines = text.lines().count();
    let lines = text.lines().take(limit).collect::<Vec<_>>();
    Ok(json!({
        "format": path.extension().and_then(|value| value.to_str()).unwrap_or("text"),
        "lines": lines,
        "lineCount": all_lines,
        "truncated": byte_truncated || all_lines > limit,
        "continuationLine": (all_lines > limit).then_some(limit + 1),
    }))
}
