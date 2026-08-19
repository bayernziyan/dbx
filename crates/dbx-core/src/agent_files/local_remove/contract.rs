use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent_events::ToolDefinition;

pub const FILE_DELETE_TOOL: &str = "dbx_file_delete";
pub const FILE_RESTORE_TOOL: &str = "dbx_file_restore";
pub const DIRECTORY_CREATE_TOOL: &str = "dbx_directory_create";
pub const DIRECTORY_DELETE_TOOL: &str = "dbx_directory_delete";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileDeleteRequest {
    pub scope_id: String,
    pub path: String,
    pub expected_hash: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRestoreRequest {
    pub scope_id: String,
    pub receipt_ref: String,
    pub expected_missing: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryCreateRequest {
    pub scope_id: String,
    pub path: String,
    pub expected_missing: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryDeleteRequest {
    pub scope_id: String,
    pub path: String,
    pub expected_empty: bool,
}

#[derive(Debug, Clone)]
pub struct ContractError {
    pub code: &'static str,
    pub message: String,
}

impl ContractError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl FileDeleteRequest {
    pub fn parse(arguments: &Value) -> Result<Self, ContractError> {
        let request: Self = parse(arguments, "file delete")?;
        require_scope_id(&request.scope_id)?;
        require_path(&request.path)?;
        require_sha256(&request.expected_hash)?;
        if request.reason.trim().is_empty() || request.reason.chars().count() > 500 {
            return Err(ContractError::new("INVALID_ARGUMENT", "reason must contain 1..=500 characters"));
        }
        Ok(request)
    }
}

impl FileRestoreRequest {
    pub fn parse(arguments: &Value) -> Result<Self, ContractError> {
        let request: Self = parse(arguments, "file restore")?;
        require_scope_id(&request.scope_id)?;
        if !request.expected_missing {
            return Err(ContractError::new("INVALID_ARGUMENT", "expected_missing must be true"));
        }
        if request.receipt_ref.trim().is_empty() {
            return Err(ContractError::new("INVALID_ARGUMENT", "receipt_ref is required"));
        }
        Ok(request)
    }
}

impl DirectoryCreateRequest {
    pub fn parse(arguments: &Value) -> Result<Self, ContractError> {
        let request: Self = parse(arguments, "directory create")?;
        require_scope_id(&request.scope_id)?;
        require_path(&request.path)?;
        if !request.expected_missing {
            return Err(ContractError::new("INVALID_ARGUMENT", "expected_missing must be true"));
        }
        Ok(request)
    }
}

impl DirectoryDeleteRequest {
    pub fn parse(arguments: &Value) -> Result<Self, ContractError> {
        let request: Self = parse(arguments, "directory delete")?;
        require_scope_id(&request.scope_id)?;
        require_path(&request.path)?;
        if !request.expected_empty {
            return Err(ContractError::new("INVALID_ARGUMENT", "expected_empty must be true"));
        }
        Ok(request)
    }
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: FILE_DELETE_TOOL,
            description: "Remove one existing allowlisted file from its db-wiki path by moving it into the scope-internal recoverable trash. Call dbx_file_stat first and pass its raw-byte contentHash as expected_hash. This tool never accepts directories, globs, recursive deletion, or permanent deletion.",
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "scope_id": { "type": "string", "minLength": 1 },
                    "path": { "type": "string", "minLength": 1 },
                    "expected_hash": { "type": "string", "pattern": "^[A-Fa-f0-9]{64}$" },
                    "reason": { "type": "string", "minLength": 1, "maxLength": 500 }
                },
                "required": ["scope_id", "path", "expected_hash", "reason"]
            }),
            read_only: false,
            parallel_ok: false,
        },
        ToolDefinition {
            name: FILE_RESTORE_TOOL,
            description: "Restore a file previously removed by dbx_file_delete using its server-generated deletion receipt. The original path must still be missing; this tool never overwrites an existing file.",
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "scope_id": { "type": "string", "minLength": 1 },
                    "receipt_ref": { "type": "string", "minLength": 1 },
                    "expected_missing": { "const": true }
                },
                "required": ["scope_id", "receipt_ref", "expected_missing"]
            }),
            read_only: false,
            parallel_ok: false,
        },
        ToolDefinition {
            name: DIRECTORY_CREATE_TOOL,
            description: "Create one intentionally empty directory inside a writable db-wiki scope. The parent directory must already exist. Do not call this before dbx_file_write merely to prepare a file path, because file creation already creates missing parent directories.",
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "scope_id": { "type": "string", "minLength": 1 },
                    "path": { "type": "string", "minLength": 1 },
                    "expected_missing": { "const": true }
                },
                "required": ["scope_id", "path", "expected_missing"]
            }),
            read_only: false,
            parallel_ok: false,
        },
        ToolDefinition {
            name: DIRECTORY_DELETE_TOOL,
            description: "Delete one empty directory inside a writable db-wiki scope. Non-empty directories are rejected. This tool has no recursive, glob, or force mode.",
            parameters: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "scope_id": { "type": "string", "minLength": 1 },
                    "path": { "type": "string", "minLength": 1 },
                    "expected_empty": { "const": true }
                },
                "required": ["scope_id", "path", "expected_empty"]
            }),
            read_only: false,
            parallel_ok: false,
        },
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(name, FILE_DELETE_TOOL | FILE_RESTORE_TOOL | DIRECTORY_CREATE_TOOL | DIRECTORY_DELETE_TOOL)
}

fn parse<T: DeserializeOwned>(arguments: &Value, operation: &str) -> Result<T, ContractError> {
    serde_json::from_value(arguments.clone())
        .map_err(|error| ContractError::new("INVALID_ARGUMENT", format!("invalid {operation} request: {error}")))
}

fn require_scope_id(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        Err(ContractError::new("INVALID_ARGUMENT", "scope_id is required"))
    } else {
        Ok(())
    }
}

fn require_path(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        Err(ContractError::new("INVALID_ARGUMENT", "path is required"))
    } else {
        Ok(())
    }
}

fn require_sha256(value: &str) -> Result<(), ContractError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(ContractError::new("INVALID_ARGUMENT", "expected_hash must be a 64-character SHA-256"))
    }
}
