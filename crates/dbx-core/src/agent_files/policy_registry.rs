use std::path::Path;
use std::sync::Arc;

use super::db_wiki_policy::DbWikiDirectoryPolicy;
use super::policy::DirectoryAllowlistPolicy;
use super::read_only_policy::ReadOnlyDirectoryPolicy;

#[derive(Clone)]
pub struct PolicyRegistry {
    policies: Arc<Vec<Arc<dyn DirectoryAllowlistPolicy>>>,
    read_fallback: Arc<dyn DirectoryAllowlistPolicy>,
}

impl Default for PolicyRegistry {
    fn default() -> Self {
        Self {
            policies: Arc::new(vec![Arc::new(DbWikiDirectoryPolicy)]),
            read_fallback: Arc::new(ReadOnlyDirectoryPolicy),
        }
    }
}

impl PolicyRegistry {
    pub fn resolve(&self, canonical_root: &Path) -> Result<Arc<dyn DirectoryAllowlistPolicy>, String> {
        let matches = self.policies.iter().filter(|policy| policy.matches(canonical_root)).cloned().collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(Arc::clone(&self.read_fallback)),
            [policy] => {
                policy.validate_root(canonical_root)?;
                Ok(Arc::clone(policy))
            }
            _ => Err("FILE_SCOPE_POLICY_AMBIGUOUS: multiple directory allowlist policies matched".to_string()),
        }
    }
}
