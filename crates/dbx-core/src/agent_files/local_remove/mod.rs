pub mod contract;
mod directory_ops;
pub mod feature_gate;
mod trash;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use self::contract::{
    ContractError, DirectoryCreateRequest, DirectoryDeleteRequest, FileDeleteRequest, FileRestoreRequest,
    DIRECTORY_CREATE_TOOL, DIRECTORY_DELETE_TOOL, FILE_DELETE_TOOL, FILE_RESTORE_TOOL, SCHEMA_VERSION,
};
use crate::agent_events::{ToolCall, ToolResult};
use crate::agent_files::directory_scope::FileDirectoryScope;
use crate::agent_files::file_support::{normalize_path, tool_result};
use crate::agent_files::file_write::{file_sha256, sync_db_wiki_manifest};
use crate::agent_files::policy::FileScopeAccess;

struct ExecutionResult {
    value: Value,
    is_error: bool,
}

pub async fn execute(call: &ToolCall, scope: &FileDirectoryScope) -> ToolResult {
    let result = execute_inner(call, scope).await.unwrap_or_else(|error| error_execution(call, error, false, false));
    tool_result(
        call,
        serde_json::to_string_pretty(&result.value).unwrap_or_else(|_| result.value.to_string()),
        result.is_error,
    )
}

pub fn scope_error_result(call: &ToolCall, error: String) -> ToolResult {
    let result = error_execution(call, scope_error(error), false, false);
    tool_result(call, serde_json::to_string_pretty(&result.value).unwrap_or_else(|_| result.value.to_string()), true)
}

async fn execute_inner(call: &ToolCall, scope: &FileDirectoryScope) -> Result<ExecutionResult, ContractError> {
    ensure_scope(scope)?;
    match call.name.as_str() {
        FILE_DELETE_TOOL => {
            let request = FileDeleteRequest::parse(&call.arguments)?;
            ensure_scope_id(scope, &request.scope_id)?;
            delete_file(call, scope, request).await
        }
        FILE_RESTORE_TOOL => {
            let request = FileRestoreRequest::parse(&call.arguments)?;
            ensure_scope_id(scope, &request.scope_id)?;
            restore_file(call, scope, request).await
        }
        DIRECTORY_CREATE_TOOL => {
            let request = DirectoryCreateRequest::parse(&call.arguments)?;
            ensure_scope_id(scope, &request.scope_id)?;
            directory_ops::create(call, scope, &request.path)
                .await
                .map(|value| ExecutionResult { value, is_error: false })
        }
        DIRECTORY_DELETE_TOOL => {
            let request = DirectoryDeleteRequest::parse(&call.arguments)?;
            ensure_scope_id(scope, &request.scope_id)?;
            directory_ops::delete(call, scope, &request.path)
                .await
                .map(|value| ExecutionResult { value, is_error: false })
        }
        _ => Err(ContractError::new("UNKNOWN_FILE_TOOL", format!("unknown local remove tool: {}", call.name))),
    }
}

async fn delete_file(
    call: &ToolCall,
    scope: &FileDirectoryScope,
    request: FileDeleteRequest,
) -> Result<ExecutionResult, ContractError> {
    let (target, relative_path) = resolve_public_target(scope, &request.path, true)?;
    let fingerprint = trash::deletion_fingerprint(scope.policy.id(), &relative_path, &request.expected_hash);
    let lock = lock_for(&target);
    let _guard = lock.lock().await;

    if let Some(mut receipt) = trash::read_receipt(scope, &fingerprint)? {
        validate_deletion_receipt(&receipt, &fingerprint, &relative_path, &request.expected_hash)?;
        if !target.exists() {
            if let Some(payload) = trash::resolve_trash_path(scope, &fingerprint)? {
                let payload_hash = file_sha256(&payload).map_err(file_error)?;
                if payload_hash.eq_ignore_ascii_case(&request.expected_hash) {
                    let manifest_status = reconcile_manifest(scope);
                    receipt.manifest_status = manifest_status.clone();
                    receipt.status = "trashed".to_string();
                    let receipt_ref = trash::write_receipt(scope, &fingerprint, &receipt)?;
                    return Ok(success_execution(
                        call,
                        "replayed",
                        false,
                        &fingerprint,
                        &relative_path,
                        &request.expected_hash,
                        Some(&receipt.trash_ref),
                        Some(&receipt_ref),
                        &manifest_status,
                        false,
                        false,
                    ));
                }
            }
        }
    }

    if !target.exists() {
        return Err(ContractError::new("FILE_NOT_FOUND", "target file does not exist"));
    }
    if !target.is_file() {
        return Err(ContractError::new("FILE_DELETE_FILE_REQUIRED", "target is not a regular file"));
    }
    let extension = target.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if !scope.policy.allows_extension(extension, true) {
        return Err(ContractError::new("FILE_EXTENSION_WRITE_BLOCKED", format!(".{extension} is not allowlisted")));
    }

    let current_hash = file_sha256(&target).map_err(file_error)?;
    if !current_hash.eq_ignore_ascii_case(&request.expected_hash) {
        return Ok(error_execution(
            call,
            ContractError::new("FILE_HASH_CONFLICT", "target changed after dbx_file_stat"),
            false,
            false,
        ));
    }
    let trash_path = trash::prepare_trash_path(scope, &fingerprint)?;
    if trash_path.exists() {
        return Err(ContractError::new("FILE_DELETE_TRASH_CONFLICT", "trash payload already exists"));
    }
    let mut receipt = trash::DeletionReceipt::pending(
        call.id.clone(),
        &fingerprint,
        relative_path.clone(),
        current_hash.clone(),
        request.reason.trim().to_string(),
    );
    let receipt_ref = trash::write_receipt(scope, &fingerprint, &receipt)?;
    let rechecked_hash = file_sha256(&target).map_err(file_error)?;
    if rechecked_hash != current_hash {
        return Ok(error_execution(
            call,
            ContractError::new("FILE_WRITE_RACE", "target changed while preparing deletion"),
            false,
            false,
        ));
    }
    std::fs::rename(&target, &trash_path).map_err(|error| {
        ContractError::new("FILE_DELETE_FAILED", format!("failed to move target into recoverable trash: {error}"))
    })?;
    let payload_hash = file_sha256(&trash_path).map_err(file_error)?;
    if !payload_hash.eq_ignore_ascii_case(&current_hash) {
        return Ok(error_execution(
            call,
            ContractError::new("FILE_DELETE_PAYLOAD_HASH_MISMATCH", "trash payload hash differs from source hash"),
            true,
            false,
        ));
    }
    let manifest_status = reconcile_manifest(scope);
    receipt.status = "trashed".to_string();
    receipt.manifest_status = manifest_status.clone();
    let receipt_write = trash::write_receipt(scope, &fingerprint, &receipt);
    let (status, is_error, final_receipt_ref) = match receipt_write {
        Ok(reference) if manifest_status == "succeeded" => ("trashed", false, reference),
        Ok(reference) => ("trashed_manifest_unknown", true, reference),
        Err(_) => ("receipt_pending", true, receipt_ref),
    };
    Ok(success_execution(
        call,
        status,
        true,
        &fingerprint,
        &relative_path,
        &current_hash,
        Some(&receipt.trash_ref),
        Some(&final_receipt_ref),
        &manifest_status,
        is_error,
        false,
    ))
}

async fn restore_file(
    call: &ToolCall,
    scope: &FileDirectoryScope,
    request: FileRestoreRequest,
) -> Result<ExecutionResult, ContractError> {
    let (fingerprint, mut receipt) = trash::read_receipt_ref(scope, &request.receipt_ref)?;
    let (target, relative_path) = resolve_public_target(scope, &receipt.relative_path, true)?;
    if relative_path != receipt.relative_path {
        return Err(ContractError::new("FILE_RESTORE_RECEIPT_INVALID", "receipt path is not canonical"));
    }
    validate_deletion_receipt(&receipt, &fingerprint, &relative_path, &receipt.previous_hash)?;
    let extension = target.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if !scope.policy.allows_extension(extension, true) {
        return Err(ContractError::new("FILE_EXTENSION_WRITE_BLOCKED", format!(".{extension} is not allowlisted")));
    }
    let lock = lock_for(&target);
    let _guard = lock.lock().await;
    let payload = trash::resolve_trash_path(scope, &fingerprint)?;

    if target.exists() {
        if payload.is_none() && target.is_file() {
            let current_hash = file_sha256(&target).map_err(file_error)?;
            if current_hash.eq_ignore_ascii_case(&receipt.previous_hash) {
                return Ok(success_execution(
                    call,
                    "replayed",
                    false,
                    &fingerprint,
                    &relative_path,
                    &receipt.previous_hash,
                    None,
                    Some(&request.receipt_ref),
                    &receipt.manifest_status,
                    false,
                    true,
                ));
            }
        }
        return Err(ContractError::new("FILE_RESTORE_TARGET_EXISTS", "original path already exists"));
    }
    let payload =
        payload.ok_or_else(|| ContractError::new("FILE_RESTORE_PAYLOAD_NOT_FOUND", "trash payload is missing"))?;
    if !payload.is_file() {
        return Err(ContractError::new("FILE_RESTORE_PAYLOAD_INVALID", "trash payload is not a regular file"));
    }
    let payload_hash = file_sha256(&payload).map_err(file_error)?;
    if !payload_hash.eq_ignore_ascii_case(&receipt.previous_hash) {
        return Err(ContractError::new("FILE_RESTORE_PAYLOAD_HASH_MISMATCH", "trash payload hash is invalid"));
    }
    let parent = target
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| ContractError::new("FILE_RESTORE_PARENT_NOT_FOUND", "original parent directory is missing"))?;
    let _ = parent;
    std::fs::rename(&payload, &target).map_err(|error| {
        ContractError::new("FILE_RESTORE_FAILED", format!("failed to restore trash payload: {error}"))
    })?;
    let restored_hash = file_sha256(&target).map_err(file_error)?;
    let manifest_status = reconcile_manifest(scope);
    receipt.status = "restored".to_string();
    receipt.manifest_status = manifest_status.clone();
    let receipt_result = trash::write_receipt(scope, &fingerprint, &receipt);
    let is_error = manifest_status != "succeeded" || receipt_result.is_err();
    let status = if receipt_result.is_err() {
        "receipt_pending"
    } else if manifest_status == "succeeded" {
        "restored"
    } else {
        "restored_manifest_unknown"
    };
    Ok(success_execution(
        call,
        status,
        false,
        &fingerprint,
        &relative_path,
        &restored_hash,
        None,
        Some(&request.receipt_ref),
        &manifest_status,
        is_error,
        true,
    ))
}

pub(super) fn resolve_public_target(
    scope: &FileDirectoryScope,
    relative: &str,
    allow_missing_leaf: bool,
) -> Result<(PathBuf, String), ContractError> {
    let trimmed = relative.trim();
    let relative_path = Path::new(trimmed);
    if trimmed.is_empty()
        || relative_path.is_absolute()
        || relative_path.components().any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContractError::new("FILE_SCOPE_INVALID_PATH", "path must be a normalized relative path"));
    }
    let target = scope.resolve_relative(trimmed, allow_missing_leaf).map_err(scope_error)?;
    if target == scope.canonical_root {
        return Err(ContractError::new("FILE_REMOVE_PROTECTED_PATH", "scope root cannot be modified"));
    }
    let normalized = normalize_path(relative_path);
    let lower = normalized.to_ascii_lowercase();
    if lower == "summary.md" || lower == ".dbx-wiki" || lower.starts_with(".dbx-wiki/") {
        return Err(ContractError::new("FILE_REMOVE_PROTECTED_PATH", "protected db-wiki path cannot be modified"));
    }
    Ok((target, normalized))
}

pub(super) fn lock_for(path: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(lock) = guard.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    guard.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

fn ensure_scope(scope: &FileDirectoryScope) -> Result<(), ContractError> {
    if scope.policy.id() != "db-wiki" {
        return Err(ContractError::new("WIKI_SCOPE_REQUIRED", "local remove tools require policyId=db-wiki"));
    }
    if scope.policy.access_mode() != FileScopeAccess::ReadWrite {
        return Err(ContractError::new("FILE_SCOPE_READ_ONLY", "this directory policy does not allow mutations"));
    }
    Ok(())
}

fn ensure_scope_id(scope: &FileDirectoryScope, scope_id: &str) -> Result<(), ContractError> {
    if scope.id == scope_id {
        Ok(())
    } else {
        Err(ContractError::new("FILE_SCOPE_NOT_FOUND", "scope_id does not match the opened scope"))
    }
}

fn reconcile_manifest(scope: &FileDirectoryScope) -> String {
    if sync_db_wiki_manifest(&scope.canonical_root).is_ok() {
        "succeeded".to_string()
    } else {
        "unknown".to_string()
    }
}

fn validate_deletion_receipt(
    receipt: &trash::DeletionReceipt,
    fingerprint: &str,
    relative_path: &str,
    expected_hash: &str,
) -> Result<(), ContractError> {
    let expected_fingerprint = trash::deletion_fingerprint("db-wiki", relative_path, expected_hash);
    if receipt.schema_version != SCHEMA_VERSION
        || expected_fingerprint != fingerprint
        || receipt.mutation_fingerprint != format!("sha256:{fingerprint}")
        || receipt.relative_path != relative_path
        || !receipt.previous_hash.eq_ignore_ascii_case(expected_hash)
        || receipt.trash_ref != trash::trash_ref(fingerprint)
    {
        return Err(ContractError::new(
            "FILE_DELETE_RECEIPT_INVALID",
            "deletion receipt does not match its path, hash, or fingerprint",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn success_execution(
    call: &ToolCall,
    status: &str,
    delete_applied: bool,
    fingerprint: &str,
    relative_path: &str,
    previous_hash: &str,
    trash_ref: Option<&str>,
    receipt_ref: Option<&str>,
    manifest_status: &str,
    is_error: bool,
    restore_applied: bool,
) -> ExecutionResult {
    let mut value = json!({
        "schemaVersion": SCHEMA_VERSION,
        "toolCallId": call.id,
        "mutationFingerprint": format!("sha256:{fingerprint}"),
        "status": status,
        "deleteApplied": delete_applied,
        "restoreApplied": restore_applied,
        "path": relative_path,
        "previousHash": previous_hash,
        "manifestStatus": manifest_status,
    });
    if let Some(reference) = trash_ref {
        value["trashRef"] = Value::String(reference.to_string());
    }
    if let Some(reference) = receipt_ref {
        value["receiptRef"] = Value::String(reference.to_string());
    }
    ExecutionResult { value, is_error }
}

fn error_execution(
    call: &ToolCall,
    error: ContractError,
    delete_applied: bool,
    restore_applied: bool,
) -> ExecutionResult {
    let status = if matches!(
        error.code,
        "FILE_HASH_CONFLICT" | "FILE_WRITE_RACE" | "FILE_RESTORE_TARGET_EXISTS" | "DIRECTORY_NOT_EMPTY"
    ) {
        "conflict"
    } else {
        "rejected"
    };
    ExecutionResult {
        value: json!({
            "schemaVersion": SCHEMA_VERSION,
            "toolCallId": call.id,
            "status": status,
            "deleteApplied": delete_applied,
            "restoreApplied": restore_applied,
            "error": { "code": error.code, "message": error.message },
        }),
        is_error: true,
    }
}

fn scope_error(error: String) -> ContractError {
    let code = error.split(':').next().unwrap_or("FILE_SCOPE_ERROR");
    let code = match code {
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

fn file_error(error: String) -> ContractError {
    ContractError::new("FILE_READ_FAILED", error)
}
