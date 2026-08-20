use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct GuardConfig {
    pub server: ServerConfig,
    pub upstream: UpstreamConfig,
    pub static_files: StaticConfig,
    pub storage: StorageConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub policy: PolicyConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub public_base_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpstreamConfig {
    pub base_url: String,
    pub credential_file: PathBuf,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_seconds: u64,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StaticConfig {
    pub directory: PathBuf,
    #[serde(default = "default_index_file")]
    pub index_file: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub credentials_db: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_idle_minutes")]
    pub idle_timeout_minutes: u64,
    #[serde(default = "default_absolute_minutes")]
    pub absolute_timeout_minutes: u64,
    #[serde(default = "default_cookie_name")]
    pub cookie_name: String,
    #[serde(default = "default_ui_cookie_name")]
    pub ui_cookie_name: String,
    #[serde(default)]
    pub secure_cookies: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            idle_timeout_minutes: default_idle_minutes(),
            absolute_timeout_minutes: default_absolute_minutes(),
            cookie_name: default_cookie_name(),
            ui_cookie_name: default_ui_cookie_name(),
            secure_cookies: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default = "default_max_body_mib")]
    pub max_body_mib: usize,
    #[serde(default = "default_login_attempts")]
    pub login_max_attempts: u32,
    #[serde(default = "default_lockout_seconds")]
    pub login_lockout_seconds: u64,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            max_body_mib: default_max_body_mib(),
            login_max_attempts: default_login_attempts(),
            login_lockout_seconds: default_lockout_seconds(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyConfig {
    #[serde(default = "default_true")]
    pub viewer_default_deny: bool,
    #[serde(default)]
    pub viewer_allow: Vec<PolicyRuleConfig>,
    #[serde(default)]
    pub viewer_deny: Vec<PolicyRuleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyRuleConfig {
    pub path: String,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub methods: Vec<String>,
}

impl GuardConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).with_context(|| format!("read guard config {}", path.display()))?;
        let mut config: Self = toml::from_str(&raw).context("parse guard config")?;
        config.normalize(path)?;
        Ok(config)
    }

    fn normalize(&mut self, path: &Path) -> Result<()> {
        self.server.public_base_path = normalize_base_path(&self.server.public_base_path)?;
        self.upstream.base_url = self.upstream.base_url.trim_end_matches('/').to_string();
        if !(self.upstream.base_url.starts_with("http://127.0.0.1:")
            || self.upstream.base_url.starts_with("http://localhost:"))
        {
            bail!("upstream.base_url must target loopback HTTP");
        }
        if self.static_files.index_file.contains(['/', '\\']) || self.static_files.index_file.trim().is_empty() {
            bail!("static_files.index_file must be a single file name");
        }
        if self.security.max_body_mib == 0 {
            bail!("security.max_body_mib must be positive");
        }
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        absolutize(&mut self.upstream.credential_file, base);
        absolutize(&mut self.static_files.directory, base);
        absolutize(&mut self.storage.credentials_db, base);
        for rule in self.policy.viewer_allow.iter().chain(self.policy.viewer_deny.iter()) {
            if !rule.path.starts_with(&self.server.public_base_path) {
                bail!("policy path '{}' must start with public_base_path", rule.path);
            }
        }
        Ok(())
    }

    pub fn upstream_url(&self, path_and_query: &str) -> String {
        format!("{}{}", self.upstream.base_url, path_and_query)
    }

    pub fn public_path(&self, suffix: &str) -> String {
        let suffix = if suffix.starts_with('/') { suffix.to_string() } else { format!("/{suffix}") };
        if self.server.public_base_path == "/" {
            suffix
        } else {
            format!("{}{suffix}", self.server.public_base_path)
        }
    }

    pub fn max_body_bytes(&self) -> usize {
        self.security.max_body_mib.saturating_mul(1024 * 1024)
    }
}

fn absolutize(path: &mut PathBuf, base: &Path) {
    if path.is_relative() {
        *path = base.join(&*path);
    }
}

fn normalize_base_path(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok("/".to_string());
    }
    if trimmed.chars().any(|ch| ch.is_ascii_control() || ch.is_ascii_whitespace() || matches!(ch, ';' | ',')) {
        bail!("server.public_base_path contains invalid characters");
    }
    Ok(format!("/{trimmed}"))
}

const fn default_connect_timeout() -> u64 {
    5
}
const fn default_request_timeout() -> u64 {
    300
}
fn default_index_file() -> String {
    "index.html".to_string()
}
const fn default_idle_minutes() -> u64 {
    480
}
const fn default_absolute_minutes() -> u64 {
    1440
}
fn default_cookie_name() -> String {
    "dbx_guard_session".to_string()
}
fn default_ui_cookie_name() -> String {
    "dbx_guard_ui".to_string()
}
const fn default_max_body_mib() -> usize {
    128
}
const fn default_login_attempts() -> u32 {
    5
}
const fn default_lockout_seconds() -> u64 {
    60
}
const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{normalize_base_path, GuardConfig};

    #[test]
    fn normalizes_non_root_base_path() {
        assert_eq!(normalize_base_path("/dbx/").unwrap(), "/dbx");
        assert_eq!(normalize_base_path("/").unwrap(), "/");
        assert!(normalize_base_path("dbx admin").is_err());
    }

    #[test]
    fn example_configuration_is_parseable() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/guard.toml.example");
        let config = GuardConfig::load(&path).unwrap();
        assert_eq!(config.server.public_base_path, "/dbx");
        assert!(config.policy.viewer_default_deny);
        assert!(!config.policy.viewer_allow.is_empty());
        assert!(!config.policy.viewer_deny.is_empty());
    }
}
