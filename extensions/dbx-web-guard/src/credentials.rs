use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::SaltString;
use rand_core::OsRng;
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Viewer,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Viewer => "viewer",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "admin" => Ok(Self::Admin),
            "viewer" => Ok(Self::Viewer),
            _ => bail!("role must be admin or viewer"),
        }
    }

    pub const fn other(self) -> Self {
        match self {
            Self::Admin => Self::Viewer,
            Self::Viewer => Self::Admin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { path: path.into() };
        if let Some(parent) = store.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create credential directory {}", parent.display()))?;
        }
        let conn = Connection::open(&store.path).context("open guard credential database")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS role_credentials (
                role TEXT PRIMARY KEY CHECK(role IN ('admin','viewer')),
                password_hash TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .context("initialize guard credential database")?;
        Ok(store)
    }

    pub fn configured(&self) -> Result<bool> {
        Ok(self.hash(Role::Admin)?.is_some() && self.hash(Role::Viewer)?.is_some())
    }

    pub fn verify(&self, role: Role, password: &str) -> Result<bool> {
        let Some(hash) = self.hash(role)? else { return Ok(false) };
        verify_hash(&hash, password)
    }

    pub fn identify(&self, password: &str) -> Result<Option<Role>> {
        let admin = self.verify(Role::Admin, password)?;
        let viewer = self.verify(Role::Viewer, password)?;
        match (admin, viewer) {
            (true, false) => Ok(Some(Role::Admin)),
            (false, true) => Ok(Some(Role::Viewer)),
            (false, false) => Ok(None),
            (true, true) => bail!("admin and viewer credentials are not distinct"),
        }
    }

    pub fn set_password(&self, role: Role, password: &str) -> Result<()> {
        validate_password(password)?;
        if let Some(other_hash) = self.hash(role.other())? {
            if verify_hash(&other_hash, password)? {
                bail!("admin and viewer passwords must be different");
            }
        }
        let hash = hash_password(password)?;
        let conn = Connection::open(&self.path).context("open guard credential database")?;
        conn.execute(
            "INSERT INTO role_credentials(role,password_hash,updated_at) VALUES (?1,?2,CURRENT_TIMESTAMP)
             ON CONFLICT(role) DO UPDATE SET password_hash=excluded.password_hash,updated_at=CURRENT_TIMESTAMP",
            params![role.as_str(), hash],
        )
        .context("save role password hash")?;
        Ok(())
    }

    fn hash(&self, role: Role) -> Result<Option<String>> {
        let conn = Connection::open(&self.path).context("open guard credential database")?;
        conn.query_row("SELECT password_hash FROM role_credentials WHERE role=?1", params![role.as_str()], |row| {
            row.get(0)
        })
        .optional()
        .context("read role password hash")
    }
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!("hash role password: {error}"))?
        .to_string())
}

fn verify_hash(hash: &str, password: &str) -> Result<bool> {
    let parsed = PasswordHash::new(hash).map_err(|error| anyhow::anyhow!("parse role password hash: {error}"))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < 10 {
        bail!("password must contain at least 10 characters");
    }
    if password.chars().count() > 256 {
        bail!("password is too long");
    }
    Ok(())
}

pub fn read_confirmed_password(prompt: &str) -> Result<String> {
    let first = read_password_without_echo(prompt)?;
    let second = read_password_without_echo("Confirm password: ")?;
    if first != second {
        bail!("password confirmation does not match");
    }
    validate_password(&first)?;
    Ok(first)
}

#[cfg(windows)]
fn read_password_without_echo(prompt: &str) -> Result<String> {
    use std::io::Write;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, ReadConsoleW, SetConsoleMode, ENABLE_ECHO_INPUT, STD_INPUT_HANDLE,
    };

    print!("{prompt}");
    std::io::stdout().flush().context("flush password prompt")?;
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        bail!("password input requires an interactive Windows console");
    }
    let mut original_mode = 0;
    if unsafe { GetConsoleMode(handle, &mut original_mode) } == 0 {
        bail!("password input requires an interactive Windows console");
    }
    if unsafe { SetConsoleMode(handle, original_mode & !ENABLE_ECHO_INPUT) } == 0 {
        return Err(std::io::Error::last_os_error()).context("disable console echo");
    }
    let mut buffer = [0u16; 512];
    let mut read = 0;
    let result =
        unsafe { ReadConsoleW(handle, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut read, null_mut()) };
    let restore_result = unsafe { SetConsoleMode(handle, original_mode) };
    println!();
    if restore_result == 0 {
        return Err(std::io::Error::last_os_error()).context("restore console mode");
    }
    if result == 0 {
        return Err(std::io::Error::last_os_error()).context("read password from console");
    }
    let value = String::from_utf16(&buffer[..read as usize]).context("decode password from console")?;
    Ok(value.trim_end_matches(['\r', '\n']).to_string())
}

#[cfg(not(windows))]
fn read_password_without_echo(_prompt: &str) -> Result<String> {
    bail!("secure interactive password input is only supported on Windows")
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CredentialStore, Role};

    #[test]
    fn stores_distinct_role_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::open(dir.path().join("credentials.db")).unwrap();
        store.set_password(Role::Admin, "admin-password-1").unwrap();
        store.set_password(Role::Viewer, "viewer-password-1").unwrap();
        assert_eq!(store.identify("admin-password-1").unwrap(), Some(Role::Admin));
        assert_eq!(store.identify("viewer-password-1").unwrap(), Some(Role::Viewer));
        assert!(store.set_password(Role::Viewer, "admin-password-1").is_err());
    }
}
