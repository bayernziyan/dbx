use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_events::ToolDefinition;

pub const TOOL_NAME: &str = "dbx_file_edit";
pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_EDITS: usize = 20;
pub const MAX_OLD_TEXT_BYTES: usize = 65_536;
pub const MAX_NEW_TEXT_BYTES: usize = 262_144;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub scope_id: String,
    pub path: String,
    pub expected_hash: String,
    pub edits: Vec<EditSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EditSpec {
    pub op: EditOperation,
    pub old_text: String,
    #[serde(default)]
    pub new_text: Option<String>,
    #[serde(default)]
    pub near_line: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditOperation {
    Replace,
    Delete,
    InsertBefore,
    InsertAfter,
}

impl EditRequest {
    pub fn parse(arguments: &Value) -> Result<Self, ContractError> {
        let request: Self = serde_json::from_value(arguments.clone())
            .map_err(|error| ContractError::new("INVALID_ARGUMENT", format!("invalid edit request: {error}")))?;
        if request.scope_id.trim().is_empty() {
            return Err(ContractError::new("INVALID_ARGUMENT", "scope_id is required"));
        }
        if request.path.trim().is_empty() {
            return Err(ContractError::new("INVALID_ARGUMENT", "path is required"));
        }
        if request.expected_hash.len() != 64 || !request.expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ContractError::new("INVALID_ARGUMENT", "expected_hash must be a 64-character SHA-256"));
        }
        if request.edits.is_empty() || request.edits.len() > MAX_EDITS {
            return Err(ContractError::new(
                "FILE_EDIT_LIMIT_EXCEEDED",
                format!("edits must contain between 1 and {MAX_EDITS} items"),
            ));
        }
        for (index, edit) in request.edits.iter().enumerate() {
            if edit.old_text.is_empty() || edit.old_text.len() > MAX_OLD_TEXT_BYTES {
                return Err(ContractError::new(
                    "FILE_EDIT_LIMIT_EXCEEDED",
                    format!("edits[{index}].old_text must contain 1..={MAX_OLD_TEXT_BYTES} bytes"),
                ));
            }
            if edit.new_text.as_ref().is_some_and(|value| value.len() > MAX_NEW_TEXT_BYTES) {
                return Err(ContractError::new(
                    "FILE_EDIT_LIMIT_EXCEEDED",
                    format!("edits[{index}].new_text exceeds {MAX_NEW_TEXT_BYTES} bytes"),
                ));
            }
            match edit.op {
                EditOperation::Delete if edit.new_text.as_ref().is_some_and(|value| !value.is_empty()) => {
                    return Err(ContractError::new(
                        "INVALID_ARGUMENT",
                        format!("edits[{index}].new_text must be absent or empty for delete"),
                    ));
                }
                EditOperation::Delete => {}
                _ if edit.new_text.is_none() => {
                    return Err(ContractError::new(
                        "INVALID_ARGUMENT",
                        format!("edits[{index}].new_text is required for this operation"),
                    ));
                }
                _ => {}
            }
        }
        Ok(request)
    }
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

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_NAME,
        description: "Edit an existing allowlisted UTF-8 text file with exact, unique old_text anchors. Call dbx_file_read or dbx_file_search for the anchor and dbx_file_stat for the raw-byte expected_hash before editing. Prefer this tool over dbx_file_write for existing files.",
        parameters: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "scope_id": { "type": "string", "minLength": 1 },
                "path": { "type": "string", "minLength": 1 },
                "expected_hash": { "type": "string", "pattern": "^[A-Fa-f0-9]{64}$" },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_EDITS,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "op": { "type": "string", "enum": ["replace", "delete", "insert_before", "insert_after"] },
                            "old_text": { "type": "string", "minLength": 1, "maxLength": MAX_OLD_TEXT_BYTES },
                            "new_text": { "type": "string", "maxLength": MAX_NEW_TEXT_BYTES },
                            "near_line": { "type": "integer", "minimum": 1 }
                        },
                        "required": ["op", "old_text"]
                    }
                }
            },
            "required": ["scope_id", "path", "expected_hash", "edits"]
        }),
        read_only: false,
        parallel_ok: false,
    }
}
