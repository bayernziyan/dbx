pub mod contract;
pub mod feature_gate;
mod matcher;
mod receipt;
mod safety;
mod snapshot;
mod writer;

use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use self::contract::{ContractError, EditRequest, SCHEMA_VERSION};
use self::matcher::{apply_edits, AppliedEdits};
use self::receipt::MutationReceipt;
use self::snapshot::FileSnapshot;
use crate::agent_events::{ToolCall, ToolResult};
use crate::agent_files::directory_scope::FileDirectoryScope;
use crate::agent_files::file_support::{normalize_path, relative_to_root, tool_result};
use crate::agent_files::file_write::{bytes_sha256, sync_db_wiki_manifest, validate_text_format};
use crate::agent_files::policy::FileScopeAccess;

struct ExecutionResult {
    value: Value,
    is_error: bool,
}

pub async fn execute(call: &ToolCall, scope: &FileDirectoryScope) -> ToolResult {
    let result = execute_inner(call, scope).await.unwrap_or_else(|error| error_execution(call, error, None, false));
    tool_result(
        call,
        serde_json::to_string_pretty(&result.value).unwrap_or_else(|_| result.value.to_string()),
        result.is_error,
    )
}

pub fn scope_error_result(call: &ToolCall, error: String) -> ToolResult {
    let result = error_execution(call, scope_error(error), None, false);
    tool_result(call, serde_json::to_string_pretty(&result.value).unwrap_or_else(|_| result.value.to_string()), true)
}

async fn execute_inner(call: &ToolCall, scope: &FileDirectoryScope) -> Result<ExecutionResult, ContractError> {
    let request = EditRequest::parse(&call.arguments)?;
    if request.scope_id != scope.id {
        return Err(ContractError::new("FILE_SCOPE_NOT_FOUND", "scope_id does not match the opened scope"));
    }
    if scope.policy.id() != "db-wiki" {
        return Err(ContractError::new("WIKI_SCOPE_REQUIRED", "dbx_file_edit currently requires policyId=db-wiki"));
    }
    if scope.policy.access_mode() != FileScopeAccess::ReadWrite {
        return Err(ContractError::new("FILE_SCOPE_READ_ONLY", "this directory policy does not allow writes"));
    }

    let target = scope.resolve_relative(request.path.trim(), false).map_err(scope_error)?;
    if !target.is_file() {
        return Err(ContractError::new("FILE_NOT_FOUND", "dbx_file_edit requires an existing file"));
    }
    let relative_path = relative_to_root(&scope.canonical_root, &target).map_err(scope_error)?;
    if relative_path == ".dbx-wiki" || relative_path.starts_with(".dbx-wiki/") {
        return Err(ContractError::new(
            "FILE_EDIT_INTERNAL_PATH_RESERVED",
            "internal .dbx-wiki control files cannot be edited directly",
        ));
    }
    let extension = target.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if !scope.policy.allows_extension(extension, true) {
        return Err(ContractError::new(
            "FILE_EXTENSION_WRITE_BLOCKED",
            format!(".{extension} is not allowlisted for writes"),
        ));
    }

    let fingerprint = mutation_fingerprint(scope.policy.id(), &relative_path, &request);
    let lock = writer::lock_for(&target);
    let _guard = lock.lock().await;
    let current = FileSnapshot::load(&target)?;

    if let Some(mut prior) = receipt::read_receipt(scope, &fingerprint)? {
        if current.raw_hash == prior.content_hash {
            if !matches!(prior.manifest_status.as_str(), "succeeded" | "reconciled") {
                if sync_db_wiki_manifest(&scope.canonical_root).is_ok() {
                    prior.manifest_status = "reconciled".to_string();
                    let _ = receipt::write_receipt(scope, &fingerprint, &prior);
                }
            }
            return Ok(replayed_execution(call, &fingerprint, &prior));
        }
    }

    if !current.raw_hash.eq_ignore_ascii_case(&request.expected_hash) {
        if let Some(execution) =
            recover_replay_from_backup(call, scope, &request, &relative_path, &fingerprint, &current)?
        {
            return Ok(execution);
        }
        return Ok(error_execution(
            call,
            ContractError::new("FILE_HASH_CONFLICT", "the target changed after dbx_file_stat"),
            Some(&fingerprint),
            false,
        ));
    }

    let applied = apply_edits(&current.text, current.eol, &request.edits)?;
    if applied.text == current.text {
        return Ok(success_execution(
            call,
            &fingerprint,
            "no_change",
            false,
            &relative_path,
            &current.raw_hash,
            &current.raw_hash,
            None,
            None,
            &applied,
            "not_run",
            &[],
            None,
        ));
    }

    validate_final_text(extension, &applied.text)?;
    let safety = safety::validate_no_new_sensitive_content(&current.text, &applied.text)?;
    let final_bytes = current.encode(&applied.text);
    let intended_hash = bytes_sha256(&final_bytes);
    let backup_ref = receipt::ensure_backup(scope, &fingerprint, &current.raw, &request.expected_hash)?;
    let final_hash = writer::write_if_unchanged(&target, &request.expected_hash, &final_bytes)?;
    let hooks = writer::run_hooks(scope, &relative_path, &target, &intended_hash);
    let status = match hooks.status.as_str() {
        "succeeded" => "applied",
        "reconciled" => "applied_manifest_reconciled",
        _ => "applied_hook_unknown",
    };
    let mut mutation_receipt = MutationReceipt::new(
        call.id.clone(),
        &fingerprint,
        relative_path.clone(),
        current.raw_hash.clone(),
        final_hash.clone(),
        applied.changed_ranges.clone(),
        applied.additions,
        applied.deletions,
        hooks.status.clone(),
        status.to_string(),
    );
    let receipt_ref = match receipt::write_receipt(scope, &fingerprint, &mutation_receipt) {
        Ok(reference) => Some(reference),
        Err(error) => {
            mutation_receipt.status = "applied_receipt_failed".to_string();
            return Ok(success_execution(
                call,
                &fingerprint,
                "applied_receipt_failed",
                true,
                &relative_path,
                &current.raw_hash,
                &final_hash,
                Some(backup_ref),
                None,
                &applied,
                &hooks.status,
                &safety.existing_findings,
                Some(json!({ "code": error.code, "message": error.message })),
            ));
        }
    };
    let retention_warning = receipt::cleanup_backups(scope, &relative_path).err().map(|error| error.code);
    let mut execution = success_execution(
        call,
        &fingerprint,
        status,
        status == "applied_hook_unknown",
        &relative_path,
        &current.raw_hash,
        &final_hash,
        Some(backup_ref),
        receipt_ref,
        &applied,
        &hooks.status,
        &safety.existing_findings,
        hooks.error.map(|message| json!({ "code": "FILE_WRITE_HOOK_FAILED", "message": message })),
    );
    if let Some(warning) = retention_warning {
        execution.value["retentionWarning"] = Value::String(warning.to_string());
    }
    Ok(execution)
}

fn recover_replay_from_backup(
    call: &ToolCall,
    scope: &FileDirectoryScope,
    request: &EditRequest,
    relative_path: &str,
    fingerprint: &str,
    current: &FileSnapshot,
) -> Result<Option<ExecutionResult>, ContractError> {
    let Some(raw) = receipt::read_backup(scope, fingerprint)? else {
        return Ok(None);
    };
    let backup = FileSnapshot::from_raw(raw)?;
    if !backup.raw_hash.eq_ignore_ascii_case(&request.expected_hash) {
        return Err(ContractError::new(
            "FILE_EDIT_BACKUP_CONFLICT",
            "the mutation backup does not match expected_hash",
        ));
    }
    let applied = apply_edits(&backup.text, backup.eol, &request.edits)?;
    let extension = Path::new(relative_path).extension().and_then(|value| value.to_str()).unwrap_or_default();
    validate_final_text(extension, &applied.text)?;
    let safety = safety::validate_no_new_sensitive_content(&backup.text, &applied.text)?;
    let intended_hash = bytes_sha256(&backup.encode(&applied.text));
    if current.raw_hash != intended_hash {
        return Ok(None);
    }
    let manifest_status = if sync_db_wiki_manifest(&scope.canonical_root).is_ok() { "reconciled" } else { "unknown" };
    let receipt_value = MutationReceipt::new(
        call.id.clone(),
        fingerprint,
        relative_path.to_string(),
        backup.raw_hash,
        current.raw_hash.clone(),
        applied.changed_ranges.clone(),
        applied.additions,
        applied.deletions,
        manifest_status.to_string(),
        "replayed".to_string(),
    );
    let receipt_ref = receipt::write_receipt(scope, fingerprint, &receipt_value)?;
    Ok(Some(success_execution(
        call,
        fingerprint,
        "replayed",
        false,
        relative_path,
        &receipt_value.previous_hash,
        &receipt_value.content_hash,
        Some(receipt_value.backup_ref.clone()),
        Some(receipt_ref),
        &applied,
        manifest_status,
        &safety.existing_findings,
        None,
    )))
}

fn validate_final_text(extension: &str, content: &str) -> Result<(), ContractError> {
    validate_text_format(extension, content).map_err(|error| ContractError::new("FILE_EDIT_FORMAT_INVALID", error))?;
    if extension.eq_ignore_ascii_case("md") {
        validate_markdown(content)?;
    }
    Ok(())
}

fn validate_markdown(content: &str) -> Result<(), ContractError> {
    let fences = content.lines().filter(|line| line.trim_start().starts_with("```")).count();
    if fences % 2 != 0 {
        return Err(ContractError::new("FORMAT_MARKDOWN_INVALID", "unbalanced fenced code block"));
    }
    let mut lines = content.lines();
    if lines.next() == Some("---") {
        let mut front_matter = Vec::new();
        let mut closed = false;
        for line in lines {
            if line == "---" {
                closed = true;
                break;
            }
            front_matter.push(line);
        }
        if !closed {
            return Err(ContractError::new("FORMAT_MARKDOWN_INVALID", "unclosed YAML front matter"));
        }
        serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&front_matter.join("\n"))
            .map_err(|error| ContractError::new("FORMAT_MARKDOWN_INVALID", format!("invalid front matter: {error}")))?;
    }
    Ok(())
}

fn mutation_fingerprint(policy_id: &str, relative_path: &str, request: &EditRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(policy_id.as_bytes());
    hasher.update([0]);
    hasher.update(normalize_path(Path::new(relative_path)).as_bytes());
    hasher.update([0]);
    hasher.update(request.expected_hash.to_ascii_lowercase().as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(&request.edits).unwrap_or_default());
    format!("{:x}", hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
fn success_execution(
    call: &ToolCall,
    fingerprint: &str,
    status: &str,
    is_error: bool,
    relative_path: &str,
    previous_hash: &str,
    content_hash: &str,
    backup_ref: Option<String>,
    receipt_ref: Option<String>,
    applied: &AppliedEdits,
    manifest_status: &str,
    existing_findings: &[&str],
    error: Option<Value>,
) -> ExecutionResult {
    let mut value = json!({
        "schemaVersion": SCHEMA_VERSION,
        "toolCallId": call.id,
        "mutationFingerprint": format!("sha256:{fingerprint}"),
        "status": status,
        "writeApplied": matches!(status, "applied" | "applied_manifest_reconciled" | "applied_hook_unknown" | "applied_receipt_failed"),
        "path": relative_path,
        "previousHash": previous_hash,
        "contentHash": content_hash,
        "changedRanges": applied.changed_ranges,
        "additions": applied.additions,
        "deletions": applied.deletions,
        "manifestStatus": manifest_status,
        "existingFindingWarnings": existing_findings,
    });
    if let Some(reference) = backup_ref {
        value["backupRef"] = Value::String(reference);
    }
    if let Some(reference) = receipt_ref {
        value["receiptRef"] = Value::String(reference);
    }
    if let Some(error) = error {
        value["error"] = error;
    }
    ExecutionResult { value, is_error }
}

fn replayed_execution(call: &ToolCall, fingerprint: &str, receipt: &MutationReceipt) -> ExecutionResult {
    ExecutionResult {
        value: json!({
            "schemaVersion": SCHEMA_VERSION,
            "toolCallId": call.id,
            "mutationFingerprint": format!("sha256:{fingerprint}"),
            "status": "replayed",
            "writeApplied": false,
            "path": receipt.relative_path,
            "previousHash": receipt.previous_hash,
            "contentHash": receipt.content_hash,
            "backupRef": receipt.backup_ref,
            "receiptRef": receipt::receipt_ref(fingerprint),
            "changedRanges": receipt.changed_ranges,
            "additions": receipt.additions,
            "deletions": receipt.deletions,
            "manifestStatus": receipt.manifest_status,
        }),
        is_error: false,
    }
}

fn error_execution(
    call: &ToolCall,
    error: ContractError,
    fingerprint: Option<&str>,
    write_applied: bool,
) -> ExecutionResult {
    let status = if matches!(error.code, "FILE_HASH_CONFLICT" | "FILE_WRITE_RACE") { "conflict" } else { "rejected" };
    let mut value = json!({
        "schemaVersion": SCHEMA_VERSION,
        "toolCallId": call.id,
        "status": status,
        "writeApplied": write_applied,
        "error": { "code": error.code, "message": error.message },
    });
    if let Some(fingerprint) = fingerprint {
        value["mutationFingerprint"] = Value::String(format!("sha256:{fingerprint}"));
    }
    ExecutionResult { value, is_error: true }
}

fn scope_error(error: String) -> ContractError {
    let code = error.split(':').next().unwrap_or("FILE_SCOPE_ERROR").to_string();
    let code: &'static str = match code.as_str() {
        "INVALID_ARGUMENT" => "INVALID_ARGUMENT",
        "FILE_SCOPE_READ_ONLY" => "FILE_SCOPE_READ_ONLY",
        "FILE_SCOPE_INVALID_PATH" => "FILE_SCOPE_INVALID_PATH",
        "FILE_SCOPE_ESCAPE" => "FILE_SCOPE_ESCAPE",
        "FILE_SCOPE_REPARSE_POINT_BLOCKED" => "FILE_SCOPE_REPARSE_POINT_BLOCKED",
        "FILE_NOT_FOUND" => "FILE_NOT_FOUND",
        _ => "FILE_SCOPE_ERROR",
    };
    ContractError::new(code, error)
}
