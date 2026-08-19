use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::policy::DirectoryAllowlistPolicy;

#[derive(Clone)]
pub struct FileDirectoryScope {
    pub id: String,
    pub canonical_root: PathBuf,
    pub policy: Arc<dyn DirectoryAllowlistPolicy>,
}

impl FileDirectoryScope {
    pub fn resolve_relative(&self, relative: &str, allow_missing_leaf: bool) -> Result<PathBuf, String> {
        let relative = Path::new(relative.trim());
        if relative.as_os_str().is_empty() {
            return Ok(self.canonical_root.clone());
        }
        if relative.is_absolute()
            || relative.components().any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err("FILE_SCOPE_INVALID_PATH: only normalized relative paths are allowed".to_string());
        }
        let candidate = self.canonical_root.join(relative);
        let checked = if candidate.exists() {
            reject_reparse_components(&self.canonical_root, &candidate)?;
            candidate
                .canonicalize()
                .map_err(|error| format!("FILE_SCOPE_PATH_ERROR: failed to resolve {}: {error}", candidate.display()))?
        } else if allow_missing_leaf {
            let parent = candidate.parent().ok_or("FILE_SCOPE_INVALID_PATH: target has no parent")?;
            reject_reparse_components(&self.canonical_root, parent)?;
            let canonical_parent = parent
                .canonicalize()
                .map_err(|error| format!("FILE_SCOPE_PATH_ERROR: failed to resolve {}: {error}", parent.display()))?;
            canonical_parent.join(candidate.file_name().ok_or("FILE_SCOPE_INVALID_PATH: target has no file name")?)
        } else {
            return Err(format!("FILE_NOT_FOUND: {}", relative.display()));
        };
        if !checked.starts_with(&self.canonical_root) {
            return Err("FILE_SCOPE_ESCAPE: resolved path escaped the allowlisted root".to_string());
        }
        Ok(checked)
    }
}

pub fn canonicalize_scope_root(path: &str) -> Result<PathBuf, String> {
    let raw = PathBuf::from(path.trim());
    if !raw.is_absolute() {
        return Err("FILE_SCOPE_ABSOLUTE_PATH_REQUIRED: scope root must be absolute".to_string());
    }
    if !raw.is_dir() {
        return Err(format!("FILE_SCOPE_DIRECTORY_REQUIRED: {} is not a directory", raw.display()));
    }
    reject_reparse_components(&raw, &raw)?;
    raw.canonicalize().map_err(|error| format!("FILE_SCOPE_PATH_ERROR: failed to resolve {}: {error}", raw.display()))
}

fn reject_reparse_components(root: &Path, target: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component.as_os_str());
        if !current.exists() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| format!("FILE_SCOPE_PATH_ERROR: failed to inspect {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
            return Err(format!("FILE_SCOPE_REPARSE_POINT_BLOCKED: {}", current.display()));
        }
    }
    if target.starts_with(root) || root == target {
        Ok(())
    } else {
        Err("FILE_SCOPE_ESCAPE: path is outside the allowlisted root".to_string())
    }
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}
