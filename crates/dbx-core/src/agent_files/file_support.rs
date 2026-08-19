use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};

use crate::agent_events::{ToolCall, ToolResult};

use super::directory_scope::FileDirectoryScope;

pub const MAX_LIST_RESULTS: usize = 500;
pub const MAX_SEARCH_RESULTS: usize = 200;
pub const MAX_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub fn require_db_wiki_policy(scope: &FileDirectoryScope) -> Result<(), String> {
    if scope.policy.id() == "db-wiki" {
        Ok(())
    } else {
        Err("WIKI_SCOPE_REQUIRED: this tool requires policyId=db-wiki".to_string())
    }
}

pub fn tool_result(call: &ToolCall, content: String, is_error: bool) -> ToolResult {
    ToolResult { tool_call_id: call.id.clone(), tool_name: call.name.clone(), content, is_error, explain_data: None }
}

pub fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("INVALID_ARGUMENT: {key} is required"))
}

pub fn required_raw_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("INVALID_ARGUMENT: {key} is required"))
}

pub fn optional_str<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

pub fn collect_entries(
    scope: &FileDirectoryScope,
    current: &Path,
    depth: usize,
    glob: Option<&str>,
    output: &mut Vec<Value>,
) -> Result<(), String> {
    if output.len() > MAX_LIST_RESULTS {
        return Ok(());
    }
    if current.is_file() {
        let relative = relative_to_root(&scope.canonical_root, current)?;
        if glob.is_none_or(|pattern| wildcard_match(pattern, &relative)) {
            output.push(file_entry(scope, current)?);
        }
        return Ok(());
    }
    for entry in
        std::fs::read_dir(current).map_err(|error| format!("FILE_LIST_FAILED: {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("FILE_LIST_FAILED: {error}"))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| format!("FILE_LIST_FAILED: {error}"))?;
        if file_type.is_symlink() {
            continue;
        }
        let relative = relative_to_root(&scope.canonical_root, &path)?;
        if glob.is_none_or(|pattern| wildcard_match(pattern, &relative)) {
            output.push(file_entry(scope, &path)?);
        }
        if file_type.is_dir() && depth > 0 {
            collect_entries(scope, &path, depth - 1, glob, output)?;
        }
        if output.len() > MAX_LIST_RESULTS {
            break;
        }
    }
    Ok(())
}

pub fn collect_files(scope: &FileDirectoryScope, current: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    if output.len() >= 2_000 {
        return Ok(());
    }
    if current.is_file() {
        output.push(current.to_path_buf());
        return Ok(());
    }
    for entry in
        std::fs::read_dir(current).map_err(|error| format!("FILE_LIST_FAILED: {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("FILE_LIST_FAILED: {error}"))?;
        let file_type = entry.file_type().map_err(|error| format!("FILE_LIST_FAILED: {error}"))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_files(scope, &path, output)?;
        } else if file_type.is_file() {
            let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
            if scope.policy.allows_extension(extension, false) {
                output.push(path);
            }
        }
    }
    Ok(())
}

pub fn relative_to_root(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|_| "FILE_SCOPE_ESCAPE: path escaped root")?;
    Ok(normalize_path(relative))
}

pub fn normalize_path(path: &Path) -> String {
    path.components().map(|component| component.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

pub fn is_searchable_text(extension: &str) -> bool {
    ["md", "txt", "sql", "json", "yaml", "yml", "xml", "csv", "tsv"]
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(extension))
}

pub fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}...", value.chars().take(max.saturating_sub(3)).collect::<String>())
    }
}

fn file_entry(scope: &FileDirectoryScope, path: &Path) -> Result<Value, String> {
    let metadata = std::fs::metadata(path).map_err(|error| format!("FILE_STAT_FAILED: {}: {error}", path.display()))?;
    Ok(json!({
        "path": relative_to_root(&scope.canonical_root, path)?,
        "type": if metadata.is_dir() { "directory" } else { "file" },
        "size": metadata.is_file().then_some(metadata.len()),
    }))
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let mut regex = String::from("^");
    for character in pattern.chars() {
        match character {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            other => regex.push_str(&regex::escape(&other.to_string())),
        }
    }
    regex.push('$');
    Regex::new(&regex).is_ok_and(|regex| regex.is_match(value))
}
