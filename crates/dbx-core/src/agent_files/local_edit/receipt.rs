use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::contract::{ContractError, SCHEMA_VERSION};
use super::matcher::ChangedRange;
use crate::agent_files::directory_scope::FileDirectoryScope;
use crate::agent_files::file_write::{atomic_write, bytes_sha256};

const BACKUP_DIRECTORY: &str = ".dbx-wiki/.backups";
const MUTATION_DIRECTORY: &str = ".dbx-wiki/.mutations";
const MAX_BACKUPS_PER_PATH: usize = 20;
const BACKUP_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationReceipt {
    pub schema_version: u32,
    pub tool_call_id: String,
    pub mutation_fingerprint: String,
    pub relative_path: String,
    pub previous_hash: String,
    pub content_hash: String,
    pub backup_ref: String,
    pub changed_ranges: Vec<ChangedRange>,
    pub additions: usize,
    pub deletions: usize,
    pub manifest_status: String,
    pub status: String,
    pub written_at_epoch_seconds: u64,
}

impl MutationReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tool_call_id: String,
        fingerprint: &str,
        relative_path: String,
        previous_hash: String,
        content_hash: String,
        changed_ranges: Vec<ChangedRange>,
        additions: usize,
        deletions: usize,
        manifest_status: String,
        status: String,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            tool_call_id,
            mutation_fingerprint: format!("sha256:{fingerprint}"),
            relative_path,
            previous_hash,
            content_hash,
            backup_ref: backup_ref(fingerprint),
            changed_ranges,
            additions,
            deletions,
            manifest_status,
            status,
            written_at_epoch_seconds: now_epoch_seconds(),
        }
    }
}

pub fn backup_ref(fingerprint: &str) -> String {
    format!("{BACKUP_DIRECTORY}/{fingerprint}.bak")
}

pub fn receipt_ref(fingerprint: &str) -> String {
    format!("{MUTATION_DIRECTORY}/{fingerprint}.receipt")
}

pub fn read_receipt(scope: &FileDirectoryScope, fingerprint: &str) -> Result<Option<MutationReceipt>, ContractError> {
    let Some(path) = resolve_existing_internal(scope, &receipt_ref(fingerprint))? else {
        return Ok(None);
    };
    let bytes = std::fs::read(&path)
        .map_err(|error| ContractError::new("FILE_EDIT_RECEIPT_READ_FAILED", format!("{}: {error}", path.display())))?;
    let receipt = serde_json::from_slice(&bytes).map_err(|error| {
        ContractError::new("FILE_EDIT_RECEIPT_INVALID", format!("invalid mutation receipt: {error}"))
    })?;
    Ok(Some(receipt))
}

pub fn write_receipt(
    scope: &FileDirectoryScope,
    fingerprint: &str,
    receipt: &MutationReceipt,
) -> Result<String, ContractError> {
    let directory = ensure_internal_directory(scope, MUTATION_DIRECTORY)?;
    let path = directory.join(format!("{fingerprint}.receipt"));
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| ContractError::new("FILE_EDIT_RECEIPT_WRITE_FAILED", error.to_string()))?;
    atomic_write(&path, &bytes).map_err(|error| ContractError::new("FILE_EDIT_RECEIPT_WRITE_FAILED", error))?;
    Ok(receipt_ref(fingerprint))
}

pub fn read_backup(scope: &FileDirectoryScope, fingerprint: &str) -> Result<Option<Vec<u8>>, ContractError> {
    let Some(path) = resolve_existing_internal(scope, &backup_ref(fingerprint))? else {
        return Ok(None);
    };
    std::fs::read(&path)
        .map(Some)
        .map_err(|error| ContractError::new("FILE_EDIT_BACKUP_READ_FAILED", format!("{}: {error}", path.display())))
}

pub fn ensure_backup(
    scope: &FileDirectoryScope,
    fingerprint: &str,
    raw: &[u8],
    expected_hash: &str,
) -> Result<String, ContractError> {
    let directory = ensure_internal_directory(scope, BACKUP_DIRECTORY)?;
    let path = directory.join(format!("{fingerprint}.bak"));
    if path.exists() {
        let existing = std::fs::read(&path).map_err(|error| {
            ContractError::new("FILE_EDIT_BACKUP_READ_FAILED", format!("{}: {error}", path.display()))
        })?;
        if !bytes_sha256(&existing).eq_ignore_ascii_case(expected_hash) {
            return Err(ContractError::new(
                "FILE_EDIT_BACKUP_CONFLICT",
                "an existing backup for this mutation has an unexpected hash",
            ));
        }
        return Ok(backup_ref(fingerprint));
    }
    if !bytes_sha256(raw).eq_ignore_ascii_case(expected_hash) {
        return Err(ContractError::new("FILE_HASH_CONFLICT", "backup source no longer matches expected_hash"));
    }
    atomic_write(&path, raw).map_err(|error| ContractError::new("FILE_EDIT_BACKUP_WRITE_FAILED", error))?;
    Ok(backup_ref(fingerprint))
}

pub fn cleanup_backups(scope: &FileDirectoryScope, relative_path: &str) -> Result<(), ContractError> {
    let Some(mutations) = resolve_existing_internal(scope, MUTATION_DIRECTORY)? else {
        return Ok(());
    };
    let mut receipts = Vec::new();
    for entry in std::fs::read_dir(&mutations).map_err(|error| {
        ContractError::new("FILE_EDIT_RETENTION_FAILED", format!("{}: {error}", mutations.display()))
    })? {
        let entry = entry.map_err(|error| ContractError::new("FILE_EDIT_RETENTION_FAILED", error.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("receipt") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()).filter(|value| is_fingerprint(value)) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let Ok(receipt) = serde_json::from_slice::<MutationReceipt>(&bytes) else { continue };
        if receipt.relative_path == relative_path {
            receipts.push((stem.to_string(), receipt.written_at_epoch_seconds));
        }
    }
    receipts.sort_by_key(|(_, written)| std::cmp::Reverse(*written));
    let now = now_epoch_seconds();
    for (index, (fingerprint, written)) in receipts.into_iter().enumerate() {
        let expired = now.saturating_sub(written) > BACKUP_RETENTION.as_secs();
        if index < MAX_BACKUPS_PER_PATH && !expired {
            continue;
        }
        if let Some(path) = resolve_existing_internal(scope, &backup_ref(&fingerprint))? {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

fn ensure_internal_directory(scope: &FileDirectoryScope, relative: &str) -> Result<PathBuf, ContractError> {
    let mut current = PathBuf::new();
    for component in Path::new(relative).components() {
        current.push(component.as_os_str());
        let current_text = current.to_string_lossy();
        match scope.resolve_relative(&current_text, false) {
            Ok(path) => {
                if !path.is_dir() {
                    return Err(ContractError::new(
                        "FILE_EDIT_INTERNAL_PATH_INVALID",
                        format!("internal path is not a directory: {}", path.display()),
                    ));
                }
            }
            Err(error) if error.starts_with("FILE_NOT_FOUND:") => {
                let path = scope.resolve_relative(&current_text, true).map_err(scope_error)?;
                std::fs::create_dir(&path).map_err(|create_error| {
                    ContractError::new(
                        "FILE_EDIT_INTERNAL_PATH_CREATE_FAILED",
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

fn scope_error(error: String) -> ContractError {
    ContractError::new("FILE_EDIT_INTERNAL_PATH_INVALID", error)
}

fn is_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
