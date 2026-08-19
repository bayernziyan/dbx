use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::directory_scope::FileDirectoryScope;
use super::policy::FileScopeAccess;

pub struct WriteRequest<'a> {
    pub relative_path: &'a str,
    pub content: &'a str,
    pub expected_hash: Option<&'a str>,
    pub expected_missing: bool,
}

pub struct WriteOutcome {
    pub relative_path: String,
    pub previous_hash: Option<String>,
    pub content_hash: String,
    pub hooks: Vec<String>,
}

pub fn write_text_file(scope: &FileDirectoryScope, request: WriteRequest<'_>) -> Result<WriteOutcome, String> {
    if scope.policy.access_mode() != FileScopeAccess::ReadWrite {
        return Err("FILE_SCOPE_READ_ONLY: this directory policy does not allow writes".to_string());
    }
    let target = scope.resolve_relative(request.relative_path, true)?;
    let extension = target.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if !scope.policy.allows_extension(extension, true) {
        return Err(format!("FILE_EXTENSION_WRITE_BLOCKED: .{extension}"));
    }
    validate_text_format(extension, request.content)?;
    let previous_hash = target.is_file().then(|| file_sha256(&target)).transpose()?;
    match (previous_hash.as_deref(), request.expected_hash, request.expected_missing) {
        (Some(_), _, true) => return Err("FILE_EXPECTED_MISSING_CONFLICT: target already exists".to_string()),
        (Some(actual), Some(expected), false) if actual.eq_ignore_ascii_case(expected) => {}
        (Some(_), None, false) => {
            return Err("FILE_EXPECTED_HASH_REQUIRED: updating an existing file requires expectedHash".to_string())
        }
        (Some(_), Some(_), false) => return Err("FILE_HASH_CONFLICT: target changed after it was read".to_string()),
        (None, Some(_), _) => return Err("FILE_HASH_CONFLICT: target does not exist".to_string()),
        (None, None, false) => {
            return Err("FILE_EXPECTED_MISSING_REQUIRED: creating a file requires expectedMissing=true".to_string())
        }
        (None, None, true) => {}
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("FILE_WRITE_FAILED: {}: {error}", parent.display()))?;
        let checked_parent = scope.resolve_relative(
            parent
                .strip_prefix(&scope.canonical_root)
                .map_err(|_| "FILE_SCOPE_ESCAPE: target parent escaped the scope")?
                .to_string_lossy()
                .as_ref(),
            false,
        )?;
        if checked_parent != parent {
            return Err("FILE_SCOPE_ESCAPE: target parent changed while preparing the write".to_string());
        }
    }
    atomic_write(&target, request.content.as_bytes())?;
    let content_hash = file_sha256(&target)?;
    let hooks = scope.policy.after_write(&scope.canonical_root, Path::new(request.relative_path))?;
    Ok(WriteOutcome { relative_path: normalize_relative(request.relative_path), previous_hash, content_hash, hooks })
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("FILE_READ_FAILED: {}: {error}", path.display()))?;
    Ok(bytes_sha256(&bytes))
}

pub fn bytes_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("FILE_WRITE_FAILED: target has no parent")?;
    let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("file");
    let temp = parent.join(format!(".{name}.{}.tmp", &uuid::Uuid::new_v4().simple().to_string()[..8]));
    let result = (|| {
        let mut file =
            File::create(&temp).map_err(|error| format!("FILE_WRITE_FAILED: {}: {error}", temp.display()))?;
        file.write_all(content).map_err(|error| format!("FILE_WRITE_FAILED: {}: {error}", temp.display()))?;
        file.sync_all().map_err(|error| format!("FILE_WRITE_FAILED: {}: {error}", temp.display()))?;
        fs::rename(&temp, path).map_err(|error| format!("FILE_WRITE_FAILED: {}: {error}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn validate_text_format(extension: &str, content: &str) -> Result<(), String> {
    match extension.to_ascii_lowercase().as_str() {
        "json" => serde_json::from_str::<serde_json::Value>(content)
            .map(|_| ())
            .map_err(|error| format!("FORMAT_JSON_INVALID: {error}")),
        "yaml" | "yml" => serde_yaml_ng::from_str::<serde_yaml_ng::Value>(content)
            .map(|_| ())
            .map_err(|error| format!("FORMAT_YAML_INVALID: {error}")),
        "xml" => {
            let mut reader = quick_xml::Reader::from_str(content);
            loop {
                match reader.read_event() {
                    Ok(quick_xml::events::Event::Eof) => break Ok(()),
                    Ok(_) => {}
                    Err(error) => break Err(format!("FORMAT_XML_INVALID: {error}")),
                }
            }
        }
        "csv" | "tsv" => {
            let mut reader = csv::ReaderBuilder::new()
                .delimiter(if extension.eq_ignore_ascii_case("tsv") { b'\t' } else { b',' })
                .flexible(true)
                .from_reader(content.as_bytes());
            for record in reader.records() {
                record.map_err(|error| format!("FORMAT_DELIMITED_INVALID: {error}"))?;
            }
            Ok(())
        }
        "md" | "txt" | "sql" => Ok(()),
        other => Err(format!("FILE_EXTENSION_WRITE_BLOCKED: .{other}")),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    manifest_version: u32,
    wiki_schema_version: u32,
    entry: String,
    documents: Vec<ManifestDocument>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDocument {
    path: String,
    kind: String,
    sha256: String,
}

pub fn sync_db_wiki_manifest(root: &Path) -> Result<(), String> {
    let mut documents = Vec::new();
    collect_manifest_documents(root, root, &mut documents)?;
    documents.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = Manifest { manifest_version: 1, wiki_schema_version: 1, entry: "SUMMARY.md".to_string(), documents };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| format!("MANIFEST_WRITE_FAILED: {error}"))?;
    let directory = root.join(".dbx-wiki");
    fs::create_dir_all(&directory).map_err(|error| format!("MANIFEST_WRITE_FAILED: {error}"))?;
    atomic_write(&directory.join("manifest.json"), &bytes)
}

fn collect_manifest_documents(root: &Path, current: &Path, output: &mut Vec<ManifestDocument>) -> Result<(), String> {
    for entry in
        fs::read_dir(current).map_err(|error| format!("MANIFEST_SCAN_FAILED: {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| format!("MANIFEST_SCAN_FAILED: {error}"))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| format!("MANIFEST_SCAN_FAILED: {error}"))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if path.file_name().and_then(|value| value.to_str()) != Some(".dbx-wiki") {
                collect_manifest_documents(root, &path, output)?;
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase();
        if !["md", "txt", "sql", "json", "yaml", "yml", "xml", "csv", "tsv", "xls", "xlsx", "docx"]
            .contains(&extension.as_str())
        {
            continue;
        }
        let relative = path.strip_prefix(root).map_err(|_| "MANIFEST_SCAN_FAILED: path escaped root")?;
        output.push(ManifestDocument {
            path: normalize_path(relative),
            kind: document_kind(relative),
            sha256: file_sha256(&path)?,
        });
    }
    Ok(())
}

fn document_kind(path: &Path) -> String {
    if path.file_name().and_then(|value| value.to_str()) == Some("SUMMARY.md") {
        "summary".to_string()
    } else if path.starts_with("tables") {
        "table".to_string()
    } else if path.starts_with("queries") {
        "query".to_string()
    } else {
        path.extension().and_then(|value| value.to_str()).unwrap_or("file").to_ascii_lowercase()
    }
}

fn normalize_path(path: &Path) -> String {
    path.components().map(|component| component.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn normalize_relative(path: &str) -> String {
    normalize_path(Path::new(path))
}
