use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileScopeAccess {
    ReadOnly,
    ReadWrite,
}

impl FileScopeAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
        }
    }
}

pub trait DirectoryAllowlistPolicy: Send + Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, canonical_root: &Path) -> bool;
    fn validate_root(&self, canonical_root: &Path) -> Result<(), String>;
    fn access_mode(&self) -> FileScopeAccess;
    fn allows_extension(&self, extension: &str, write: bool) -> bool;
    fn after_write(&self, canonical_root: &Path, relative_path: &Path) -> Result<Vec<String>, String>;
}
