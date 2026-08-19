use std::path::Path;

use serde_json::json;

pub fn parse_delimited(path: &Path, tsv: bool, limit: usize) -> Result<serde_json::Value, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("FILE_TOO_LARGE: delimited files are limited to 8 MiB per parse call".to_string());
    }
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(if tsv { b'\t' } else { b',' })
        .flexible(true)
        .from_reader(bytes.as_slice());
    let headers = reader
        .headers()
        .map_err(|error| format!("FORMAT_DELIMITED_INVALID: {error}"))?
        .iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut truncated = false;
    for record in reader.records() {
        let record = record.map_err(|error| format!("FORMAT_DELIMITED_INVALID: {error}"))?;
        if rows.len() >= limit {
            truncated = true;
            break;
        }
        rows.push(record.iter().map(ToOwned::to_owned).collect::<Vec<_>>());
    }
    Ok(json!({ "format": if tsv { "tsv" } else { "csv" }, "headers": headers, "rows": rows, "truncated": truncated }))
}
