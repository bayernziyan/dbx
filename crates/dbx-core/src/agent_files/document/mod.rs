mod delimited;
mod docx;
mod excel;
mod structured_text;
mod text;

use std::path::Path;

use encoding_rs::{GBK, UTF_16BE, UTF_16LE};
use serde_json::Value;

pub const DEFAULT_PARSE_LIMIT: usize = 200;
pub const MAX_PARSE_LIMIT: usize = 2_000;

pub(crate) struct DecodedText {
    pub text: String,
    pub encoding: &'static str,
}

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
    let mut length = bytes.len().min(max_bytes);
    loop {
        match decode_text(&bytes[..length]) {
            Ok(decoded) => return Ok((decoded.text, truncated)),
            Err(_) if truncated && length > 0 && bytes.len().saturating_sub(length) < 4 => length -= 1,
            Err(error) => return Err(format!("{error}: {}", path.display())),
        }
    }
}

pub(crate) fn decode_text(bytes: &[u8]) -> Result<DecodedText, String> {
    if let Some(utf8) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        let text =
            std::str::from_utf8(utf8).map_err(|_| "FILE_ENCODING_UNSUPPORTED: invalid UTF-8 BOM text".to_string())?;
        return Ok(DecodedText { text: text.to_string(), encoding: "utf-8-bom" });
    }
    if let Some(utf16le) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_legacy(UTF_16LE, utf16le, "utf-16le");
    }
    if let Some(utf16be) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_legacy(UTF_16BE, utf16be, "utf-16be");
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(DecodedText { text: text.to_string(), encoding: "utf-8" });
    }
    decode_legacy(GBK, bytes, "gbk")
}

fn decode_legacy(
    encoding: &'static encoding_rs::Encoding,
    bytes: &[u8],
    label: &'static str,
) -> Result<DecodedText, String> {
    let (text, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err("FILE_ENCODING_UNSUPPORTED: supported text encodings are UTF-8, UTF-16 with BOM, and GBK; dbx_file_parse does not convert an unknown encoding".to_string());
    }
    Ok(DecodedText { text: text.into_owned(), encoding: label })
}
