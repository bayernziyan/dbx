use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::contract::{ContractError, SCHEMA_VERSION};
use crate::agent_files::directory_scope::FileDirectoryScope;
use crate::agent_files::file_write::atomic_write;

const TRASH_DIRECTORY: &str = ".dbx-wiki/.trash";
const DELETION_DIRECTORY: &str = ".dbx-wiki/.deletions";
const DIRECTORY_MUTATION_DIRECTORY: &str = ".dbx-wiki/.directory-mutations";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionReceipt {
    pub schema_version: u32,
    pub tool_call_id: String,
    pub mutation_fingerprint: String,
    pub relative_path: String,
    pub previous_hash: String,
    pub reason: String,
    pub trash_ref: String,
    pub manifest_status: String,
    pub status: String,
    pub written_at_epoch_seconds: u64,
}

impl DeletionReceipt {
    pub fn pending(
        tool_call_id: String,
        fingerprint: &str,
        relative_path: String,
        previous_hash: String,
        reason: String,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tool_call_id,
            mutation_fingerprint: format!("sha256:{fingerprint}"),
            relative_path,
            previous_hash,
            reason,
            trash_ref: trash_ref(fingerprint),
            manifest_status: "not_run".to_string(),
            status: "pending".to_string(),
            written_at_epoch_seconds: now_epoch_seconds(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryMutationReceipt<'a> {
    schema_version: u32,
    tool_call_id: &'a str,
    mutation_fingerprint: String,
    operation: &'a str,
    relative_path: &'a str,
    written_at_epoch_seconds: u64,
}

pub fn deletion_fingerprint(policy_id: &str, relative_path: &str, expected_hash: &str) -> String {
    fingerprint(&[policy_id, "file-delete", relative_path, &expected_hash.to_ascii_lowercase()])
}

pub fn directory_fingerprint(policy_id: &str, operation: &str, relative_path: &str) -> String {
    fingerprint(&[policy_id, operation, relative_path])
}

pub fn trash_ref(fingerprint: &str) -> String {
    format!("{TRASH_DIRECTORY}/{fingerprint}/payload")
}

pub fn receipt_ref(fingerprint: &str) -> String {
    format!("{DELETION_DIRECTORY}/{fingerprint}.receipt")
}

pub fn prepare_trash_path(scope: &FileDirectoryScope, fingerprint: &str) -> Result<PathBuf, ContractError> {
    ensure_internal_directory(scope, &format!("{TRASH_DIRECTORY}/{fingerprint}"))?;
    scope.resolve_relative(&trash_ref(fingerprint), true).map_err(scope_error)
}

pub fn resolve_trash_path(scope: &FileDirectoryScope, fingerprint: &str) -> Result<Option<PathBuf>, ContractError> {
    resolve_existing_internal(scope, &trash_ref(fingerprint))
}

pub fn read_receipt(scope: &FileDirectoryScope, fingerprint: &str) -> Result<Option<DeletionReceipt>, ContractError> {
    let Some(path) = resolve_existing_internal(scope, &receipt_ref(fingerprint))? else {
        return Ok(None);
    };
    read_receipt_path(&path).map(Some)
}

pub fn read_receipt_ref(
    scope: &FileDirectoryScope,
    reference: &str,
) -> Result<(String, DeletionReceipt), ContractError> {
    let fingerprint = fingerprint_from_receipt_ref(reference)?;
    let expected_reference = receipt_ref(&fingerprint);
    let path = scope
        .resolve_relative(&expected_reference, false)
        .map_err(|_| ContractError::new("FILE_RESTORE_RECEIPT_NOT_FOUND", "deletion receipt does not exist"))?;
    Ok((fingerprint, read_receipt_path(&path)?))
}

pub fn write_receipt(
    scope: &FileDirectoryScope,
    fingerprint: &str,
    receipt: &DeletionReceipt,
) -> Result<String, ContractError> {
    let directory = ensure_internal_directory(scope, DELETION_DIRECTORY)?;
    let path = directory.join(format!("{fingerprint}.receipt"));
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| ContractError::new("FILE_DELETE_RECEIPT_WRITE_FAILED", error.to_string()))?;
    atomic_write(&path, &bytes).map_err(|error| ContractError::new("FILE_DELETE_RECEIPT_WRITE_FAILED", error))?;
    Ok(receipt_ref(fingerprint))
}

pub fn write_directory_receipt(
    scope: &FileDirectoryScope,
    fingerprint: &str,
    tool_call_id: &str,
    operation: &str,
    relative_path: &str,
) -> Result<String, ContractError> {
    let directory = ensure_internal_directory(scope, DIRECTORY_MUTATION_DIRECTORY)?;
    let reference = format!("{DIRECTORY_MUTATION_DIRECTORY}/{fingerprint}.receipt");
    let receipt = DirectoryMutationReceipt {
        schema_version: SCHEMA_VERSION,
        tool_call_id,
        mutation_fingerprint: format!("sha256:{fingerprint}"),
        operation,
        relative_path,
        written_at_epoch_seconds: now_epoch_seconds(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| ContractError::new("DIRECTORY_RECEIPT_WRITE_FAILED", error.to_string()))?;
    atomic_write(&directory.join(format!("{fingerprint}.receipt")), &bytes)
        .map_err(|error| ContractError::new("DIRECTORY_RECEIPT_WRITE_FAILED", error))?;
    Ok(reference)
}

fn read_receipt_path(path: &Path) -> Result<DeletionReceipt, ContractError> {
    let bytes = std::fs::read(path).map_err(|error| {
        ContractError::new("FILE_DELETE_RECEIPT_READ_FAILED", format!("{}: {error}", path.display()))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ContractError::new("FILE_DELETE_RECEIPT_INVALID", format!("invalid receipt: {error}")))
}

fn fingerprint_from_receipt_ref(reference: &str) -> Result<String, ContractError> {
    let path = Path::new(reference.trim());
    if path.is_absolute() || path.components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(ContractError::new(
            "FILE_RESTORE_RECEIPT_INVALID",
            "receipt_ref must be a normalized relative path",
        ));
    }
    let normalized = path.components().map(|value| value.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    if !normalized.starts_with(&format!("{DELETION_DIRECTORY}/"))
        || path.extension().and_then(|value| value.to_str()) != Some("receipt")
    {
        return Err(ContractError::new("FILE_RESTORE_RECEIPT_INVALID", "receipt_ref is not a deletion receipt"));
    }
    let fingerprint = path.file_stem().and_then(|value| value.to_str()).unwrap_or_default();
    if !is_fingerprint(fingerprint) || normalized != receipt_ref(fingerprint) {
        return Err(ContractError::new("FILE_RESTORE_RECEIPT_INVALID", "receipt_ref has an invalid fingerprint"));
    }
    Ok(fingerprint.to_string())
}

fn ensure_internal_directory(scope: &FileDirectoryScope, relative: &str) -> Result<PathBuf, ContractError> {
    let mut current = PathBuf::new();
    for component in Path::new(relative).components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(ContractError::new("FILE_REMOVE_INTERNAL_PATH_INVALID", "internal path is not normalized"));
        }
        current.push(component.as_os_str());
        let current_text = current.to_string_lossy();
        match scope.resolve_relative(&current_text, false) {
            Ok(path) if path.is_dir() => {}
            Ok(_) => {
                return Err(ContractError::new(
                    "FILE_REMOVE_INTERNAL_PATH_INVALID",
                    "internal control path is not a directory",
                ));
            }
            Err(error) if error.starts_with("FILE_NOT_FOUND:") => {
                let path = scope.resolve_relative(&current_text, true).map_err(scope_error)?;
                std::fs::create_dir(&path).map_err(|create_error| {
                    ContractError::new(
                        "FILE_REMOVE_INTERNAL_PATH_CREATE_FAILED",
                        format!("{}: {create_error}", path.display()),
                    )
                })?;
                scope.resolve_relative(&current_text, false).map_err(scope_error)?;
            }
            Err(error) => return Err(scope_error(error)),
        }
    }
    scope.resolve_relative(relative, false).map_err(scope_error)
}

fn resolve_existing_internal(scope: &FileDirectoryScope, relative: &str) -> Result<Option<PathBuf>, ContractError> {
    let candidate = scope.canonical_root.join(relative);
    if !candidate.exists() {
        return Ok(None);
    }
    scope.resolve_relative(relative, false).map(Some).map_err(scope_error)
}

fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn is_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn scope_error(error: String) -> ContractError {
    ContractError::new("FILE_REMOVE_INTERNAL_PATH_INVALID", error)
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
