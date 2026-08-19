use serde_json::{json, Value};

use crate::agent_events::ToolDefinition;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
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
    ]
}

pub fn handles(name: &str) -> bool {
    matches!(
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
