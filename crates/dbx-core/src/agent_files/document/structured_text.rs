use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::json;

pub fn parse_structured_text(path: &Path, extension: &str, limit: usize) -> Result<serde_json::Value, String> {
    let (text, byte_truncated) = super::read_text_limited(path, 1024 * 1024)?;
    let summary = match extension {
        "json" => {
            let value: serde_json::Value =
                serde_json::from_str(&text).map_err(|error| format!("FORMAT_JSON_INVALID: {error}"))?;
            json!({ "rootType": json_type(&value), "preview": value })
        }
        "yaml" | "yml" => {
            let value: serde_yaml_ng::Value =
                serde_yaml_ng::from_str(&text).map_err(|error| format!("FORMAT_YAML_INVALID: {error}"))?;
            let value = serde_json::to_value(value).map_err(|error| format!("FORMAT_YAML_INVALID: {error}"))?;
            json!({ "rootType": json_type(&value), "preview": value })
        }
        "xml" => {
            validate_xml(&text)?;
            let lines = text.lines().take(limit).collect::<Vec<_>>();
            json!({ "lines": lines, "lineCount": text.lines().count() })
        }
        _ => return Err(format!("FORMAT_NOT_SUPPORTED: .{extension}")),
    };
    Ok(json!({ "format": extension, "content": summary, "truncated": byte_truncated }))
}

fn validate_xml(text: &str) -> Result<(), String> {
    let mut reader = Reader::from_str(text);
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(error) => return Err(format!("FORMAT_XML_INVALID: {error}")),
        }
    }
}

fn json_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
