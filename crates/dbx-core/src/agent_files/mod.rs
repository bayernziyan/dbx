pub mod audit;
pub mod db_wiki_policy;
pub mod directory_scope;
pub mod document;
pub mod extension;
pub mod file_support;
pub mod file_tools;
pub mod file_write;
pub mod policy;
pub mod policy_registry;
pub mod read_only_policy;
pub mod tool_catalog;

pub use extension::AgentFunctionRegistry;
pub use file_tools::AgentFileService;

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use crate::agent_events::ToolCall;

    use super::AgentFileService;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall { id: format!("{name}-id"), name: name.to_string(), arguments, provider_payload: None }
    }

    fn db_wiki() -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("db-wiki");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("SUMMARY.md"), "# Database Wiki\n").unwrap();
        (temp, root)
    }

    fn parsed(result: &crate::agent_events::ToolResult) -> Value {
        assert!(!result.is_error, "{}", result.content);
        serde_json::from_str(&result.content).unwrap()
    }

    #[test]
    fn definitions_are_additive_and_namespaced() {
        let service = AgentFileService::default();
        let definitions = service.definitions();
        assert!(definitions.iter().all(|tool| !tool.parallel_ok), "scope-dependent tools must preserve order");
        let names = definitions.into_iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "dbx_file_open_scope",
                "dbx_file_close_scope",
                "dbx_file_list",
                "dbx_file_search",
                "dbx_file_read",
                "dbx_file_parse",
                "dbx_file_stat",
                "dbx_file_write",
                "dbx_wiki_status",
                "dbx_wiki_search",
                "dbx_wiki_build_evidence",
                "dbx_wiki_sync_manifest",
                "dbx_wiki_update_from_session",
            ]
        );
    }

    #[tokio::test]
    async fn ordinary_directory_opens_read_only_and_rejects_write() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("notes.md"), "# Notes\n").unwrap();
        let service = AgentFileService::default();
        let opened =
            service.execute(&call("dbx_file_open_scope", json!({ "path": temp.path().to_string_lossy() }))).await;
        let opened_json = parsed(&opened);
        assert_eq!(opened_json["policyId"], "generic-read-only");
        assert_eq!(opened_json["accessMode"], "read-only");
        let scope_id = opened_json["scopeId"].as_str().unwrap();

        let read = service.execute(&call("dbx_file_read", json!({ "scope_id": scope_id, "path": "notes.md" }))).await;
        assert!(!read.is_error, "{}", read.content);
        let write = service
            .execute(&call(
                "dbx_file_write",
                json!({ "scope_id": scope_id, "path": "notes.md", "content": "changed", "expected_hash": "x" }),
            ))
            .await;
        assert!(write.is_error);
        assert!(write.content.contains("FILE_SCOPE_READ_ONLY"));
    }

    #[tokio::test]
    async fn empty_db_wiki_directory_is_allowlisted_by_leaf_name() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("db-wiki");
        std::fs::create_dir(&root).unwrap();
        let service = AgentFileService::default();
        let result = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        assert_eq!(parsed(&result)["policyId"], "db-wiki");
    }

    #[tokio::test]
    async fn scope_reads_parses_and_writes_with_hash_guards() {
        let (_temp, root) = db_wiki();
        std::fs::create_dir(root.join("tables")).unwrap();
        std::fs::write(root.join("tables/tasks.md"), "# Tasks\nstatus: ready\n").unwrap();
        std::fs::write(root.join("tables/status.csv"), "code,label\n1,Ready\n").unwrap();
        let service = AgentFileService::default();
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();

        let listed = service
            .execute(&call("dbx_file_list", json!({ "scope_id": scope_id, "path": "tables", "depth": 1 })))
            .await;
        assert!(parsed(&listed)["entries"].as_array().unwrap().iter().any(|entry| entry["path"] == "tables/tasks.md"));

        let read =
            service.execute(&call("dbx_file_read", json!({ "scope_id": scope_id, "path": "tables/tasks.md" }))).await;
        let read_json = parsed(&read);
        let hash = read_json["contentHash"].as_str().unwrap();
        assert_eq!(read_json["lines"][1]["text"], "status: ready");

        let parsed_csv = service
            .execute(&call("dbx_file_parse", json!({ "scope_id": scope_id, "path": "tables/status.csv" })))
            .await;
        assert_eq!(parsed(&parsed_csv)["document"]["headers"][0], "code");

        let updated = service
            .execute(&call(
                "dbx_file_write",
                json!({
                    "scope_id": scope_id,
                    "path": "tables/tasks.md",
                    "content": "# Tasks\nstatus: done\n",
                    "expected_hash": hash,
                }),
            ))
            .await;
        assert!(!updated.is_error, "{}", updated.content);
        assert_eq!(std::fs::read_to_string(root.join("tables/tasks.md")).unwrap(), "# Tasks\nstatus: done\n");
        assert!(root.join(".dbx-wiki/manifest.json").is_file());

        let stale = service
            .execute(&call(
                "dbx_file_write",
                json!({
                    "scope_id": scope_id,
                    "path": "tables/tasks.md",
                    "content": "stale",
                    "expected_hash": hash,
                }),
            ))
            .await;
        assert!(stale.is_error);
        assert!(stale.content.contains("FILE_HASH_CONFLICT"));
    }

    #[tokio::test]
    async fn scope_blocks_parent_traversal_and_write_extensions() {
        let (_temp, root) = db_wiki();
        let service = AgentFileService::default();
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let traversal =
            service.execute(&call("dbx_file_read", json!({ "scope_id": scope_id, "path": "../secret.txt" }))).await;
        assert!(traversal.is_error);
        assert!(traversal.content.contains("FILE_SCOPE_INVALID_PATH"));

        let binary = service
            .execute(&call(
                "dbx_file_write",
                json!({ "scope_id": scope_id, "path": "malware.exe", "content": "x", "expected_missing": true }),
            ))
            .await;
        assert!(binary.is_error);
        assert!(binary.content.contains("FILE_EXTENSION_WRITE_BLOCKED"));
    }
}
