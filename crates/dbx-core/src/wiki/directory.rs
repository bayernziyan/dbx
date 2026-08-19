use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct WikiDirectory {
    root: PathBuf,
}

impl WikiDirectory {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}
