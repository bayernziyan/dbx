use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::agent_events::{ToolCall, ToolDefinition, ToolResult};

use super::audit::FileWriteAudit;
use super::directory_scope::{canonicalize_scope_root, FileDirectoryScope};
use super::document::{parse_document, DEFAULT_PARSE_LIMIT};
use super::file_support::{
    collect_entries, collect_files, is_searchable_text, normalize_path, optional_str, relative_to_root,
    require_db_wiki_policy, required_raw_str, required_str, tool_result, truncate_chars, MAX_LIST_RESULTS,
    MAX_SEARCH_RESULTS, MAX_TEXT_FILE_BYTES,
};
use super::file_write::{bytes_sha256, file_sha256, write_text_file, WriteRequest};
use super::policy_registry::PolicyRegistry;

pub struct AgentFileService {
    policies: PolicyRegistry,
    scopes: RwLock<HashMap<String, FileDirectoryScope>>,
    local_edit_enabled: bool,
    local_remove_enabled: bool,
}

impl Default for AgentFileService {
    fn default() -> Self {
        Self::with_features(
            super::local_edit::feature_gate::enabled_from_env(),
            super::local_remove::feature_gate::enabled_from_env(),
        )
    }
}

impl AgentFileService {
    #[cfg(test)]
    pub(crate) fn with_local_edit_enabled(local_edit_enabled: bool) -> Self {
        Self::with_features(local_edit_enabled, false)
    }

    pub(crate) fn with_features(local_edit_enabled: bool, local_remove_enabled: bool) -> Self {
        Self {
            policies: PolicyRegistry::default(),
            scopes: RwLock::new(HashMap::new()),
            local_edit_enabled,
            local_remove_enabled,
        }
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        super::tool_catalog::definitions(self.local_edit_enabled, self.local_remove_enabled)
    }

    pub fn handles(&self, name: &str) -> bool {
        super::tool_catalog::handles(name, self.local_edit_enabled, self.local_remove_enabled)
    }

    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        if call.name == super::local_edit::contract::TOOL_NAME && self.local_edit_enabled {
            return match self.scope(&call.arguments).await {
                Ok(scope) => super::local_edit::execute(call, &scope).await,
                Err(error) => super::local_edit::scope_error_result(call, error),
            };
        }
        if super::local_remove::contract::handles(&call.name) && self.local_remove_enabled {
            return match self.scope(&call.arguments).await {
                Ok(scope) => super::local_remove::execute(call, &scope).await,
                Err(error) => super::local_remove::scope_error_result(call, error),
            };
        }
        let result = match call.name.as_str() {
            "dbx_file_open_scope" => self.open_scope(&call.arguments).await,
            "dbx_file_close_scope" => self.close_scope(&call.arguments).await,
            "dbx_file_list" => self.list(&call.arguments).await,
            "dbx_file_search" => self.search(&call.arguments).await,
            "dbx_file_read" => self.read(&call.arguments).await,
            "dbx_file_parse" => self.parse(&call.arguments).await,
            "dbx_file_stat" => self.stat(&call.arguments).await,
            "dbx_file_write" => self.write(&call.arguments).await,
            "dbx_wiki_status" => self.wiki_status(&call.arguments).await,
            "dbx_wiki_search" => self.wiki_search(&call.arguments).await,
            "dbx_wiki_build_evidence" => self.wiki_build_evidence(&call.arguments).await,
            "dbx_wiki_sync_manifest" => self.wiki_sync_manifest(&call.arguments).await,
            "dbx_wiki_update_from_session" => self.write(&call.arguments).await,
            _ => Err(format!("UNKNOWN_FILE_TOOL: {}", call.name)),
        };
        match result {
            Ok(value) => {
                tool_result(call, serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()), false)
            }
            Err(error) => tool_result(call, format!("Error: {error}"), true),
        }
    }

    async fn open_scope(&self, arguments: &Value) -> Result<Value, String> {
        let path = required_str(arguments, "path")?;
        let canonical_root = canonicalize_scope_root(path)?;
        let policy = self.policies.resolve(&canonical_root)?;
        let id = uuid::Uuid::new_v4().to_string();
        let scope =
            FileDirectoryScope { id: id.clone(), canonical_root: canonical_root.clone(), policy: Arc::clone(&policy) };
        self.scopes.write().await.insert(id.clone(), scope);
        Ok(json!({
            "scopeId": id,
            "policyId": policy.id(),
            "accessMode": policy.access_mode().as_str(),
            "rootPath": canonical_root,
        }))
    }

    async fn close_scope(&self, arguments: &Value) -> Result<Value, String> {
        let scope_id = required_str(arguments, "scope_id")?;
        let removed = self.scopes.write().await.remove(scope_id).is_some();
        Ok(json!({ "scopeId": scope_id, "closed": removed }))
    }

    async fn scope(&self, arguments: &Value) -> Result<FileDirectoryScope, String> {
        let scope_id = required_str(arguments, "scope_id")?;
        self.scopes
            .read()
            .await
            .get(scope_id)
            .cloned()
            .ok_or_else(|| {
                "FILE_SCOPE_NOT_FOUND: scope is closed or the application restarted; call dbx_file_open_scope again with the user-provided directory path"
                    .to_string()
            })
    }

    async fn list(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let relative = optional_str(arguments, "path").unwrap_or("");
        let depth = arguments.get("depth").and_then(Value::as_u64).unwrap_or(2).min(10) as usize;
        let glob = optional_str(arguments, "glob");
        let start = scope.resolve_relative(relative, false)?;
        let mut entries = Vec::new();
        collect_entries(&scope, &start, depth, glob, &mut entries)?;
        let truncated = entries.len() > MAX_LIST_RESULTS;
        entries.truncate(MAX_LIST_RESULTS);
        Ok(
            json!({ "scopeId": scope.id, "path": normalize_path(Path::new(relative)), "entries": entries, "truncated": truncated }),
        )
    }

    async fn search(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let query = required_str(arguments, "query")?;
        if query.is_empty() {
            return Err("FILE_SEARCH_QUERY_REQUIRED: query cannot be empty".to_string());
        }
        let relative = optional_str(arguments, "path").unwrap_or("");
        let use_regex = arguments.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let regex = use_regex
            .then(|| Regex::new(query).map_err(|error| format!("FILE_SEARCH_REGEX_INVALID: {error}")))
            .transpose()?;
        let start = scope.resolve_relative(relative, false)?;
        let mut files = Vec::new();
        collect_files(&scope, &start, &mut files)?;
        let mut matches = Vec::new();
        for path in files {
            let metadata =
                std::fs::metadata(&path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
            if metadata.len() > MAX_TEXT_FILE_BYTES {
                continue;
            }
            let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
            if !is_searchable_text(extension) {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => continue,
            };
            for (index, line) in text.lines().enumerate() {
                let matched = regex.as_ref().map_or_else(|| line.contains(query), |pattern| pattern.is_match(line));
                if matched {
                    matches.push(json!({
                        "path": relative_to_root(&scope.canonical_root, &path)?,
                        "line": index + 1,
                        "text": truncate_chars(line, 500),
                    }));
                    if matches.len() >= MAX_SEARCH_RESULTS {
                        return Ok(json!({ "scopeId": scope.id, "matches": matches, "truncated": true }));
                    }
                }
            }
        }
        Ok(json!({ "scopeId": scope.id, "matches": matches, "truncated": false }))
    }

    async fn read(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let relative = required_str(arguments, "path")?;
        let path = scope.resolve_relative(relative, false)?;
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
        if !scope.policy.allows_extension(extension, false) {
            return Err(format!("FILE_EXTENSION_READ_BLOCKED: .{extension}"));
        }
        let metadata =
            std::fs::metadata(&path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
        if metadata.len() > MAX_TEXT_FILE_BYTES {
            return Err("FILE_TOO_LARGE: text reads are limited to 2 MiB; use parse for structured files".to_string());
        }
        let bytes = std::fs::read(&path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
        let text =
            std::str::from_utf8(bytes).map_err(|_| "FILE_ENCODING_UNSUPPORTED: use dbx_file_parse".to_string())?;
        let start = arguments.get("start_line").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        let max_lines = arguments.get("max_lines").and_then(Value::as_u64).unwrap_or(200).clamp(1, 500) as usize;
        let total_lines = text.lines().count();
        let lines = text
            .lines()
            .enumerate()
            .skip(start - 1)
            .take(max_lines)
            .map(|(index, line)| json!({ "line": index + 1, "text": line }))
            .collect::<Vec<_>>();
        let next_line = start + lines.len();
        Ok(json!({
            "scopeId": scope.id,
            "path": normalize_path(Path::new(relative)),
            "contentHash": bytes_sha256(bytes),
            "lines": lines,
            "lineCount": total_lines,
            "truncated": next_line <= total_lines,
            "continuationLine": (next_line <= total_lines).then_some(next_line),
        }))
    }

    async fn parse(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let relative = required_str(arguments, "path")?;
        let path = scope.resolve_relative(relative, false)?;
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
        if !scope.policy.allows_extension(extension, false) {
            return Err(format!("FILE_EXTENSION_READ_BLOCKED: .{extension}"));
        }
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_PARSE_LIMIT as u64) as usize;
        let content = parse_document(&path, limit)?;
        Ok(json!({
            "scopeId": scope.id,
            "path": normalize_path(Path::new(relative)),
            "contentHash": file_sha256(&path)?,
            "document": content,
        }))
    }

    async fn stat(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let relative = required_str(arguments, "path")?;
        let path = scope.resolve_relative(relative, false)?;
        let metadata =
            std::fs::metadata(&path).map_err(|error| format!("FILE_STAT_FAILED: {}: {error}", path.display()))?;
        Ok(json!({
            "scopeId": scope.id,
            "path": normalize_path(Path::new(relative)),
            "file": metadata.is_file(),
            "directory": metadata.is_dir(),
            "size": metadata.len(),
            "contentHash": metadata.is_file().then(|| file_sha256(&path)).transpose()?,
            "modified": metadata.modified().ok().and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok()).map(|value| value.as_secs()),
        }))
    }

    async fn write(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        let relative = required_str(arguments, "path")?;
        let content = required_raw_str(arguments, "content")?;
        let outcome = write_text_file(
            &scope,
            WriteRequest {
                relative_path: relative,
                content,
                expected_hash: optional_str(arguments, "expected_hash"),
                expected_missing: arguments.get("expected_missing").and_then(Value::as_bool).unwrap_or(false),
            },
        )?;
        let audit = FileWriteAudit {
            scope_id: scope.id,
            policy_id: scope.policy.id().to_string(),
            relative_path: outcome.relative_path,
            previous_hash: outcome.previous_hash,
            content_hash: outcome.content_hash,
            hooks: outcome.hooks,
        };
        serde_json::to_value(audit).map_err(|error| format!("FILE_AUDIT_FAILED: {error}"))
    }

    async fn wiki_status(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        require_db_wiki_policy(&scope)?;
        let mut files = Vec::new();
        collect_files(&scope, &scope.canonical_root, &mut files)?;
        Ok(json!({
            "scopeId": scope.id,
            "policyId": scope.policy.id(),
            "documentCount": files.len(),
            "summaryPresent": scope.canonical_root.join("SUMMARY.md").is_file(),
            "manifestPresent": scope.canonical_root.join(".dbx-wiki/manifest.json").is_file(),
        }))
    }

    async fn wiki_search(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        require_db_wiki_policy(&scope)?;
        self.search(arguments).await
    }

    async fn wiki_build_evidence(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        require_db_wiki_policy(&scope)?;
        let question = required_str(arguments, "question")?;
        let mut search_arguments = arguments.clone();
        search_arguments["query"] = Value::String(question.to_string());
        let search = self.search(&search_arguments).await?;
        let matches = search.get("matches").cloned().unwrap_or_else(|| json!([]));
        Ok(json!({
            "schemaVersion": 1,
            "scopeId": scope.id,
            "question": question,
            "facts": matches,
            "warnings": [],
            "missingEvidence": [],
            "truncated": search.get("truncated").cloned().unwrap_or(Value::Bool(false)),
        }))
    }

    async fn wiki_sync_manifest(&self, arguments: &Value) -> Result<Value, String> {
        let scope = self.scope(arguments).await?;
        require_db_wiki_policy(&scope)?;
        super::file_write::sync_db_wiki_manifest(&scope.canonical_root)?;
        Ok(json!({ "scopeId": scope.id, "synced": true, "path": ".dbx-wiki/manifest.json" }))
    }
}
