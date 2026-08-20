use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::credentials::ensure_parent;

#[cfg(windows)]
pub fn protect_to_file(path: &Path, plaintext: &[u8]) -> Result<()> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPT_INTEGER_BLOB,
    };

    if plaintext.is_empty() {
        bail!("upstream credential cannot be empty");
    }
    ensure_parent(path)?;
    let input = CRYPT_INTEGER_BLOB { cbData: plaintext.len() as u32, pbData: plaintext.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let ok = unsafe {
        CryptProtectData(&input, null(), null(), null_mut(), null(), CRYPTPROTECT_LOCAL_MACHINE, &mut output)
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("encrypt upstream credential with DPAPI");
    }
    let encrypted = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, encrypted).with_context(|| format!("write temporary credential {}", temp.display()))?;
    replace_file(&temp, path)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        MoveFileExW(source_wide.as_ptr(), target_wide.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("replace credential {}", target.display()));
    }
    Ok(())
}

#[cfg(windows)]
pub fn unprotect_from_file(path: &Path) -> Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let encrypted = std::fs::read(path).with_context(|| format!("read encrypted credential {}", path.display()))?;
    if encrypted.is_empty() {
        bail!("encrypted upstream credential is empty");
    }
    let input = CRYPT_INTEGER_BLOB { cbData: encrypted.len() as u32, pbData: encrypted.as_ptr() as *mut u8 };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    let ok = unsafe { CryptUnprotectData(&input, null_mut(), null(), null_mut(), null(), 0, &mut output) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("decrypt upstream credential with DPAPI");
    }
    let plaintext = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(plaintext)
}

#[cfg(not(windows))]
pub fn protect_to_file(_path: &Path, _plaintext: &[u8]) -> Result<()> {
    bail!("DPAPI credential storage is only supported on Windows")
}

#[cfg(not(windows))]
pub fn unprotect_from_file(_path: &Path) -> Result<Vec<u8>> {
    bail!("DPAPI credential storage is only supported on Windows")
}

#[cfg(all(test, windows))]
mod tests {
    use super::{protect_to_file, unprotect_from_file};

    #[test]
    fn dpapi_roundtrip_and_rotation_replace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upstream.dpapi");
        protect_to_file(&path, b"first-upstream-password").unwrap();
        assert_eq!(unprotect_from_file(&path).unwrap(), b"first-upstream-password");
        protect_to_file(&path, b"second-upstream-password").unwrap();
        assert_eq!(unprotect_from_file(&path).unwrap(), b"second-upstream-password");
    }
}
