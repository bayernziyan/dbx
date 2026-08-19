use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use serde_json::json;

pub fn parse_docx(path: &Path, limit: usize) -> Result<serde_json::Value, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| format!("FORMAT_DOCX_INVALID: {error}"))?;
    if archive.len() > 2_000 {
        return Err("FORMAT_DOCX_RESOURCE_LIMIT: too many ZIP entries".to_string());
    }
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|_| "FORMAT_DOCX_INVALID: word/document.xml is missing".to_string())?;
    if document.size() > 16 * 1024 * 1024 {
        return Err("FORMAT_DOCX_RESOURCE_LIMIT: document.xml is too large".to_string());
    }
    let mut xml = String::new();
    document.read_to_string(&mut xml).map_err(|error| format!("FORMAT_DOCX_INVALID: {error}"))?;
    let mut reader = XmlReader::from_str(&xml);
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut truncated = false;
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                let decoded = text.unescape().map_err(|error| format!("FORMAT_DOCX_INVALID: {error}"))?;
                current.push_str(&decoded);
            }
            Ok(Event::End(end)) if end.name().as_ref() == b"w:p" => {
                if !current.trim().is_empty() {
                    if paragraphs.len() >= limit {
                        truncated = true;
                        break;
                    }
                    paragraphs.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("FORMAT_DOCX_INVALID: {error}")),
        }
    }
    Ok(json!({ "format": "docx", "paragraphs": paragraphs, "truncated": truncated }))
}
