mod delimited;
mod docx;
mod excel;
mod structured_text;
mod text;

use std::path::Path;

use serde_json::Value;

pub const DEFAULT_PARSE_LIMIT: usize = 200;
pub const MAX_PARSE_LIMIT: usize = 2_000;

pub fn parse_document(path: &Path, limit: usize) -> Result<Value, String> {
    let limit = limit.clamp(1, MAX_PARSE_LIMIT);
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
    match extension.as_str() {
        "md" | "txt" | "sql" => text::parse_text(path, limit),
        "json" | "yaml" | "yml" | "xml" => structured_text::parse_structured_text(path, &extension, limit),
        "csv" | "tsv" => delimited::parse_delimited(path, extension == "tsv", limit),
        "xls" | "xlsx" => excel::parse_excel(path, limit),
        "docx" => docx::parse_docx(path, limit),
        "doc" => {
            Err("FORMAT_CONVERSION_REQUIRED: legacy .doc is not supported; convert it to docx or text".to_string())
        }
        _ => Err(format!("FORMAT_NOT_SUPPORTED: .{extension}")),
    }
}

pub(crate) fn read_text_limited(path: &Path, max_bytes: usize) -> Result<(String, bool), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
    let truncated = bytes.len() > max_bytes;
    let slice = &bytes[..bytes.len().min(max_bytes)];
    let slice = slice.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(slice);
    let text = std::str::from_utf8(slice)
        .map_err(|_| format!("FILE_ENCODING_UNSUPPORTED: {} is not UTF-8 text", path.display()))?;
    Ok((text.to_string(), truncated))
}
