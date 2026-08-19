use std::path::Path;

use super::contract::ContractError;
use crate::agent_files::file_support::MAX_TEXT_FILE_BYTES;
use crate::agent_files::file_write::bytes_sha256;

const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EolKind {
    None,
    Lf,
    CrLf,
    Mixed,
}

#[derive(Debug, Clone)]
pub struct FileSnapshot {
    pub raw: Vec<u8>,
    pub text: String,
    pub raw_hash: String,
    pub has_bom: bool,
    pub eol: EolKind,
    pub ends_with_newline: bool,
}

impl FileSnapshot {
    pub fn load(path: &Path) -> Result<Self, ContractError> {
        let raw = std::fs::read(path)
            .map_err(|error| ContractError::new("FILE_READ_FAILED", format!("{}: {error}", path.display())))?;
        Self::from_raw(raw)
    }

    pub fn from_raw(raw: Vec<u8>) -> Result<Self, ContractError> {
        if raw.len() as u64 > MAX_TEXT_FILE_BYTES {
            return Err(ContractError::new(
                "FILE_TOO_LARGE",
                format!("text edits are limited to {MAX_TEXT_FILE_BYTES} bytes"),
            ));
        }
        let has_bom = raw.starts_with(UTF8_BOM);
        let content = if has_bom { &raw[UTF8_BOM.len()..] } else { raw.as_slice() };
        let text = std::str::from_utf8(content)
            .map_err(|_| ContractError::new("FILE_ENCODING_WRITE_UNSUPPORTED", "only UTF-8 text files can be edited"))?
            .to_string();
        let eol = detect_eol(&text);
        let ends_with_newline = text.ends_with('\n') || text.ends_with('\r');
        let raw_hash = bytes_sha256(&raw);
        Ok(Self { raw, text, raw_hash, has_bom, eol, ends_with_newline })
    }

    pub fn encode(&self, text: &str) -> Vec<u8> {
        debug_assert_eq!(self.ends_with_newline, self.text.ends_with('\n') || self.text.ends_with('\r'));
        let mut bytes = Vec::with_capacity(text.len() + if self.has_bom { UTF8_BOM.len() } else { 0 });
        if self.has_bom {
            bytes.extend_from_slice(UTF8_BOM);
        }
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }
}

pub fn normalize_tool_text(value: &str, eol: EolKind) -> Result<String, ContractError> {
    let normalized = value.replace("\r\n", "\n");
    if normalized.contains('\r') {
        return Err(ContractError::new("FILE_EDIT_INVALID_LINE_ENDING", "bare carriage returns are not supported"));
    }
    if eol == EolKind::Mixed && normalized.contains('\n') {
        return Err(ContractError::new(
            "FILE_EDIT_MIXED_EOL_MULTILINE_UNSUPPORTED",
            "multi-line edits are disabled for mixed-line-ending files",
        ));
    }
    Ok(match eol {
        EolKind::CrLf => normalized.replace('\n', "\r\n"),
        _ => normalized,
    })
}

fn detect_eol(text: &str) -> EolKind {
    let bytes = text.as_bytes();
    let mut lf = 0usize;
    let mut crlf = 0usize;
    let mut bare_cr = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' => {
                bare_cr += 1;
                index += 1;
            }
            b'\n' => {
                lf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    match (lf, crlf, bare_cr) {
        (0, 0, 0) => EolKind::None,
        (_, _, count) if count > 0 => EolKind::Mixed,
        (count, 0, 0) if count > 0 => EolKind::Lf,
        (0, count, 0) if count > 0 => EolKind::CrLf,
        _ => EolKind::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::{EolKind, FileSnapshot};

    #[test]
    fn snapshot_hashes_raw_bom_bytes_and_preserves_them() {
        let raw = b"\xEF\xBB\xBFline\r\n".to_vec();
        let snapshot = FileSnapshot::from_raw(raw.clone()).unwrap();
        assert!(snapshot.has_bom);
        assert_eq!(snapshot.eol, EolKind::CrLf);
        assert_eq!(snapshot.encode(&snapshot.text), raw);
        assert_eq!(snapshot.raw_hash, crate::agent_files::file_write::bytes_sha256(&raw));
    }
}
