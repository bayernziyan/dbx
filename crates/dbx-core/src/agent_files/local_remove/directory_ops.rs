use serde_json::{json, Value};

use super::contract::{ContractError, SCHEMA_VERSION};
use super::{lock_for, resolve_public_target};
use crate::agent_events::ToolCall;
use crate::agent_files::directory_scope::FileDirectoryScope;

pub async fn create(call: &ToolCall, scope: &FileDirectoryScope, relative: &str) -> Result<Value, ContractError> {
    let (target, relative_path) = resolve_public_target(scope, relative, true)?;
    let lock = lock_for(&target);
    let _guard = lock.lock().await;
    if target.exists() {
        return Err(ContractError::new("DIRECTORY_ALREADY_EXISTS", "target already exists"));
    }
    let parent =
        target.parent().ok_or_else(|| ContractError::new("DIRECTORY_PARENT_NOT_FOUND", "target has no parent"))?;
    if !parent.is_dir() {
        return Err(ContractError::new("DIRECTORY_PARENT_NOT_FOUND", "parent directory does not exist"));
    }
    std::fs::create_dir(&target)
        .map_err(|error| ContractError::new("DIRECTORY_CREATE_FAILED", format!("{}: {error}", target.display())))?;
    let fingerprint = super::trash::directory_fingerprint(scope.policy.id(), "directory-create", &relative_path);
    let receipt_ref =
        super::trash::write_directory_receipt(scope, &fingerprint, &call.id, "directory-create", &relative_path)?;
    Ok(json!({
        "schemaVersion": SCHEMA_VERSION,
        "toolCallId": call.id,
        "mutationFingerprint": format!("sha256:{fingerprint}"),
        "status": "created",
        "path": relative_path,
        "receiptRef": receipt_ref,
    }))
}

pub async fn delete(call: &ToolCall, scope: &FileDirectoryScope, relative: &str) -> Result<Value, ContractError> {
    let (target, relative_path) = resolve_public_target(scope, relative, false)?;
    let lock = lock_for(&target);
    let _guard = lock.lock().await;
    if !target.is_dir() {
        return Err(ContractError::new("DIRECTORY_REQUIRED", "target is not a directory"));
    }
    let mut entries = std::fs::read_dir(&target)
        .map_err(|error| ContractError::new("DIRECTORY_READ_FAILED", format!("{}: {error}", target.display())))?;
    if entries
        .next()
        .transpose()
        .map_err(|error| ContractError::new("DIRECTORY_READ_FAILED", error.to_string()))?
        .is_some()
    {
        return Err(ContractError::new(
            "DIRECTORY_NOT_EMPTY",
            "directory contains entries and recursive deletion is not supported",
        ));
    }
    std::fs::remove_dir(&target)
        .map_err(|error| ContractError::new("DIRECTORY_DELETE_FAILED", format!("{}: {error}", target.display())))?;
    let fingerprint = super::trash::directory_fingerprint(scope.policy.id(), "directory-delete", &relative_path);
    let receipt_ref =
        super::trash::write_directory_receipt(scope, &fingerprint, &call.id, "directory-delete", &relative_path)?;
    Ok(json!({
        "schemaVersion": SCHEMA_VERSION,
        "toolCallId": call.id,
        "mutationFingerprint": format!("sha256:{fingerprint}"),
        "status": "deleted_empty_directory",
        "path": relative_path,
        "receiptRef": receipt_ref,
    }))
}
