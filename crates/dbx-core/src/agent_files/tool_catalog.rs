use serde_json::{json, Value};

use crate::agent_events::ToolDefinition;

pub fn definitions(local_edit_enabled: bool, local_remove_enabled: bool) -> Vec<ToolDefinition> {
    let mut definitions = vec![
        tool(
            "dbx_file_open_scope",
            "Open a read scope from an absolute directory path explicitly present in the user prompt. Any canonical directory can be read; write access is granted only when a registered write allowlist policy matches (V1: final directory name db-wiki).",
            json!({"type":"object","properties":{"path":{"type":"string","description":"Absolute directory path explicitly supplied by the user"}},"required":["path"]}),
            true,
            false,
        ),
        tool(
            "dbx_file_close_scope",
            "Close an open file scope.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"}},"required":["scope_id"]}),
            true,
            false,
        ),
        tool(
            "dbx_file_list",
            "List files and directories inside an open scope using relative paths.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string","default":""},"depth":{"type":"integer","minimum":0,"maximum":10,"default":2},"glob":{"type":"string"}},"required":["scope_id"]}),
            true,
            true,
        ),
        tool(
            "dbx_file_search",
            "Search UTF-8 text files inside an open scope and return bounded line matches.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"query":{"type":"string"},"path":{"type":"string","default":""},"regex":{"type":"boolean","default":false}},"required":["scope_id","query"]}),
            true,
            true,
        ),
        tool(
            "dbx_file_read",
            "Read a bounded line window from a UTF-8 text file inside an open scope. Use dbx_file_parse for Office or structured files.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string"},"start_line":{"type":"integer","minimum":1,"default":1},"max_lines":{"type":"integer","minimum":1,"maximum":500,"default":200}},"required":["scope_id","path"]}),
            true,
            true,
        ),
        tool(
            "dbx_file_parse",
            "Parse a supported text, CSV/TSV, Excel, or DOCX file into bounded structured JSON.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":2000,"default":200}},"required":["scope_id","path"]}),
            true,
            true,
        ),
        tool(
            "dbx_file_stat",
            "Return metadata and SHA-256 for a file inside an open scope.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string"}},"required":["scope_id","path"]}),
            true,
            true,
        ),
        tool(
            "dbx_file_write",
            "Create or update an allowlisted UTF-8 text file inside a read-write scope. Existing files require expected_hash; new files require expected_missing=true.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string"},"content":{"type":"string"},"expected_hash":{"type":"string"},"expected_missing":{"type":"boolean","default":false}},"required":["scope_id","path","content"]}),
            false,
            false,
        ),
        tool(
            "dbx_wiki_status",
            "Return status for a db-wiki file scope, including document count and manifest presence.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"}},"required":["scope_id"]}),
            true,
            true,
        ),
        tool(
            "dbx_wiki_search",
            "Search text evidence inside a db-wiki scope and return bounded relative-path citations.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"query":{"type":"string"}},"required":["scope_id","query"]}),
            true,
            true,
        ),
        tool(
            "dbx_wiki_build_evidence",
            "Build a bounded evidence pack for a natural-language question from text files in a db-wiki scope.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"question":{"type":"string"}},"required":["scope_id","question"]}),
            true,
            true,
        ),
        tool(
            "dbx_wiki_sync_manifest",
            "Regenerate .dbx-wiki/manifest.json for a db-wiki scope.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"}},"required":["scope_id"]}),
            false,
            false,
        ),
        tool(
            "dbx_wiki_update_from_session",
            "Write session-derived knowledge to a text file in a db-wiki scope using the same hash guards as dbx_file_write.",
            json!({"type":"object","properties":{"scope_id":{"type":"string"},"path":{"type":"string"},"content":{"type":"string"},"expected_hash":{"type":"string"},"expected_missing":{"type":"boolean","default":false}},"required":["scope_id","path","content"]}),
            false,
            false,
        ),
    ];
    if local_edit_enabled {
        let write_index =
            definitions.iter().position(|definition| definition.name == "dbx_file_write").unwrap_or(definitions.len());
        let mut edit = super::local_edit::contract::definition();
        if local_remove_enabled {
            edit.description = "Edit content inside an existing allowlisted UTF-8 text file with exact, unique old_text anchors. The edit operation named delete removes only matched text; it never removes the file itself. Use dbx_file_delete to remove a whole file. Call dbx_file_stat for the raw-byte expected_hash before editing.";
        }
        definitions.insert(write_index, edit);
    }
    if local_remove_enabled {
        for definition in &mut definitions {
            if definition.name == "dbx_file_list" {
                definition.description = "List files and subdirectories inside an open scope using a normalized relative path and bounded depth (0..10). Use this existing traversal tool to inspect a directory before dbx_directory_delete; it never mutates or recursively deletes entries.";
            } else if definition.name == "dbx_file_stat" {
                definition.description = "Return metadata and the raw-byte SHA-256 contentHash for a file inside an open scope. Use this hash as expected_hash before dbx_file_edit or dbx_file_delete; stat never mutates the target.";
            } else if definition.name == "dbx_file_write" {
                definition.description = "Create or update an allowlisted UTF-8 text file inside a read-write scope; it never deletes a file. Existing files require expected_hash and new files require expected_missing=true. Missing parent directories may be created as part of writing the file; use dbx_directory_create only when an empty directory is itself required.";
            } else if definition.name == "dbx_wiki_update_from_session" {
                definition.description = "Create or update session-derived knowledge in a db-wiki text file using the same hash guards as dbx_file_write; it never deletes files. Use dbx_file_delete only when the user explicitly requests whole-file removal.";
            } else if definition.name == "dbx_wiki_sync_manifest" {
                definition.description = "Regenerate .dbx-wiki/manifest.json for a db-wiki scope. File write, edit, delete, and restore already coordinate the Manifest; call this tool only to repair independent drift or an explicitly reported unknown Manifest status.";
            }
        }
        let write_index = definitions
            .iter()
            .position(|definition| definition.name == "dbx_file_write")
            .map_or(definitions.len(), |index| index + 1);
        definitions.splice(write_index..write_index, super::local_remove::contract::definitions());
    }
    definitions
}

pub fn handles(name: &str, local_edit_enabled: bool, local_remove_enabled: bool) -> bool {
    (local_edit_enabled && name == super::local_edit::contract::TOOL_NAME)
        || (local_remove_enabled && super::local_remove::contract::handles(name))
        || matches!(
            name,
            "dbx_file_open_scope"
                | "dbx_file_close_scope"
                | "dbx_file_list"
                | "dbx_file_search"
                | "dbx_file_read"
                | "dbx_file_parse"
                | "dbx_file_stat"
                | "dbx_file_write"
                | "dbx_wiki_status"
                | "dbx_wiki_search"
                | "dbx_wiki_build_evidence"
                | "dbx_wiki_sync_manifest"
                | "dbx_wiki_update_from_session"
        )
}

fn tool(
    name: &'static str,
    description: &'static str,
    parameters: Value,
    read_only: bool,
    _parallel_ok: bool,
) -> ToolDefinition {
    // Scope lifecycle and dependent file calls must preserve the model's order.
    // In particular, a batched open_scope + read/list response must never run
    // the dependent read before the scope exists.
    ToolDefinition { name, description, parameters, read_only, parallel_ok: false }
}
