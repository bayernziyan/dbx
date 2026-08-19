use std::path::Path;

use calamine::{open_workbook_auto, Reader};
use serde_json::json;

pub fn parse_excel(path: &Path, limit: usize) -> Result<serde_json::Value, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
    if metadata.len() > 32 * 1024 * 1024 {
        return Err("FILE_TOO_LARGE: Excel files are limited to 32 MiB".to_string());
    }
    let mut workbook = open_workbook_auto(path).map_err(|error| format!("FORMAT_EXCEL_INVALID: {error}"))?;
    let names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::new();
    for name in names.iter().take(32) {
        let range = workbook.worksheet_range(name).map_err(|error| format!("FORMAT_EXCEL_INVALID: {error}"))?;
        let total_rows = range.height();
        let rows = range
            .rows()
            .take(limit)
            .map(|row| row.iter().map(ToString::to_string).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        sheets.push(json!({
            "name": name,
            "rows": rows,
            "rowCount": total_rows,
            "columnCount": range.width(),
            "truncated": total_rows > limit,
        }));
    }
    Ok(json!({
        "format": path.extension().and_then(|value| value.to_str()).unwrap_or("excel"),
        "sheets": sheets,
        "sheetCount": names.len(),
        "truncated": names.len() > 32,
    }))
}
