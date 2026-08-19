use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use tokio::sync::Mutex as AsyncMutex;

use super::contract::ContractError;
use crate::agent_files::directory_scope::FileDirectoryScope;
use crate::agent_files::file_write::{atomic_write, file_sha256, sync_db_wiki_manifest};

pub struct HookOutcome {
    pub status: String,
    pub error: Option<String>,
}

pub fn lock_for(path: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = guard.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    guard.insert(path.to_path_buf(), Arc::downgrade(&lock));
    lock
}

pub fn write_if_unchanged(path: &Path, expected_hash: &str, final_bytes: &[u8]) -> Result<String, ContractError> {
    let before = file_sha256(path).map_err(|error| ContractError::new("FILE_READ_FAILED", error))?;
    if !before.eq_ignore_ascii_case(expected_hash) {
        return Err(ContractError::new("FILE_WRITE_RACE", "the target changed before the final replace"));
    }
    atomic_write(path, final_bytes).map_err(|error| ContractError::new("FILE_WRITE_FAILED", error))?;
    let final_hash = file_sha256(path).map_err(|error| ContractError::new("FILE_READ_FAILED", error))?;
    let intended_hash = crate::agent_files::file_write::bytes_sha256(final_bytes);
    if final_hash != intended_hash {
        return Err(ContractError::new(
            "FILE_EDIT_FINAL_HASH_MISMATCH",
            "the final file hash differs from the intended edit result",
        ));
    }
    Ok(final_hash)
}

pub fn run_hooks(scope: &FileDirectoryScope, relative_path: &str, target: &Path, intended_hash: &str) -> HookOutcome {
    match scope.policy.after_write(&scope.canonical_root, Path::new(relative_path)) {
        Ok(_) => HookOutcome { status: "succeeded".to_string(), error: None },
        Err(error) => {
            let target_matches = file_sha256(target).is_ok_and(|hash| hash == intended_hash);
            if target_matches && scope.policy.id() == "db-wiki" {
                match sync_db_wiki_manifest(&scope.canonical_root) {
                    Ok(()) => HookOutcome { status: "reconciled".to_string(), error: Some(redact_hook_error(&error)) },
                    Err(reconcile_error) => {
                        HookOutcome { status: "unknown".to_string(), error: Some(redact_hook_error(&reconcile_error)) }
                    }
                }
            } else {
                HookOutcome { status: "unknown".to_string(), error: Some(redact_hook_error(&error)) }
            }
        }
    }
}

fn redact_hook_error(error: &str) -> String {
    let first_line = error.lines().next().unwrap_or("hook failed");
    first_line.chars().take(500).collect()
}
