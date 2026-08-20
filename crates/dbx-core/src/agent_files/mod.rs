pub mod audit;
pub mod db_wiki_policy;
pub mod directory_scope;
pub mod document;
pub mod extension;
pub mod file_support;
pub mod file_tools;
pub mod file_write;
pub mod local_edit;
pub mod local_remove;
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
        let service = AgentFileService::with_local_edit_enabled(true);
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
                "dbx_file_edit",
                "dbx_file_write",
                "dbx_wiki_status",
                "dbx_wiki_search",
                "dbx_wiki_build_evidence",
                "dbx_wiki_sync_manifest",
                "dbx_wiki_update_from_session",
            ]
        );
    }

    #[test]
    fn disabling_local_edit_preserves_the_previous_tool_snapshot() {
        let service = AgentFileService::with_local_edit_enabled(false);
        let names = service.definitions().into_iter().map(|tool| tool.name).collect::<Vec<_>>();
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
        assert!(!service.handles("dbx_file_edit"));
    }

    #[test]
    fn local_remove_tools_are_additive_and_have_distinct_responsibilities() {
        let service = AgentFileService::with_features(true, true);
        let definitions = service.definitions();
        let names = definitions.iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert!(names.contains(&"dbx_file_delete"));
        assert!(names.contains(&"dbx_file_restore"));
        assert!(names.contains(&"dbx_directory_create"));
        assert!(names.contains(&"dbx_directory_delete"));

        let description = |name: &str| definitions.iter().find(|tool| tool.name == name).unwrap().description;
        assert!(description("dbx_file_edit").contains("matched text"));
        assert!(description("dbx_file_delete").contains("never accepts directories"));
        assert!(description("dbx_file_write").contains("never deletes a file"));
        assert!(description("dbx_file_list").contains("inspect a directory"));
        assert!(description("dbx_file_stat").contains("before dbx_file_edit or dbx_file_delete"));
        assert!(description("dbx_wiki_sync_manifest").contains("already coordinate the Manifest"));
        assert!(description("dbx_directory_create").contains("intentionally empty directory"));
        assert!(definitions
            .iter()
            .filter(|definition| super::local_remove::contract::handles(definition.name))
            .all(|definition| !definition.read_only && !definition.parallel_ok));
    }

    #[test]
    fn disabling_local_remove_preserves_existing_tool_contracts() {
        let service = AgentFileService::with_features(true, false);
        assert!(!service.handles("dbx_file_delete"));
        assert!(!service
            .definitions()
            .iter()
            .any(|definition| super::local_remove::contract::handles(definition.name)));
        let write = service.definitions().into_iter().find(|definition| definition.name == "dbx_file_write").unwrap();
        assert_eq!(
            write.description,
            "Create or update an allowlisted UTF-8 text file inside a read-write scope. Existing files require expected_hash; new files require expected_missing=true."
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
    async fn dependent_file_tool_without_scope_returns_recoverable_scope_required_error() {
        let service = AgentFileService::default();
        let result = service.execute(&call("dbx_file_search", json!({ "query": "workflow" }))).await;

        assert!(result.is_error);
        assert!(result.content.contains("FILE_SCOPE_REQUIRED"));
        assert!(result.content.contains("dbx_file_open_scope"));
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
    async fn gbk_text_is_read_searchable_and_parseable_without_utf8_fallback_error() {
        let (_temp, root) = db_wiki();
        std::fs::create_dir(root.join("tables")).unwrap();
        let (encoded, _, had_errors) = encoding_rs::GBK.encode("# 流程实例\n发起人字段: request_user_id\n");
        assert!(!had_errors);
        std::fs::write(root.join("tables/instance.md"), encoded.as_ref()).unwrap();
        let service = AgentFileService::default();
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();

        let read = service
            .execute(&call("dbx_file_read", json!({ "scope_id": scope_id, "path": "tables/instance.md" })))
            .await;
        let read_json = parsed(&read);
        assert_eq!(read_json["encoding"], "gbk");
        assert_eq!(read_json["lines"][1]["text"], "发起人字段: request_user_id");

        let search =
            service.execute(&call("dbx_file_search", json!({ "scope_id": scope_id, "query": "发起人" }))).await;
        assert_eq!(parsed(&search)["matches"][0]["path"], "tables/instance.md");

        let parsed_document = service
            .execute(&call("dbx_file_parse", json!({ "scope_id": scope_id, "path": "tables/instance.md" })))
            .await;
        assert_eq!(parsed(&parsed_document)["document"]["lines"][1], "发起人字段: request_user_id");
    }

    #[tokio::test]
    async fn local_edit_replaces_exact_text_and_replays_without_reapplying() {
        let (_temp, root) = db_wiki();
        std::fs::create_dir(root.join("tables")).unwrap();
        std::fs::write(root.join("tables/tasks.md"), "# Tasks\nstatus: ready\nowner: team\n").unwrap();
        let service = AgentFileService::with_local_edit_enabled(true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "tables/tasks.md" }))).await;
        let hash = parsed(&stat)["contentHash"].as_str().unwrap().to_string();
        let arguments = json!({
            "scope_id": scope_id,
            "path": "tables/tasks.md",
            "expected_hash": hash,
            "edits": [{ "op": "replace", "old_text": "status: ready", "new_text": "status: done" }]
        });

        let updated = service.execute(&call("dbx_file_edit", arguments.clone())).await;
        let updated_json = parsed(&updated);
        assert_eq!(updated_json["status"], "applied");
        assert_eq!(updated_json["writeApplied"], true);
        assert_eq!(
            std::fs::read_to_string(root.join("tables/tasks.md")).unwrap(),
            "# Tasks\nstatus: done\nowner: team\n"
        );
        assert!(root.join(updated_json["backupRef"].as_str().unwrap()).is_file());
        assert!(root.join(updated_json["receiptRef"].as_str().unwrap()).is_file());

        let replayed = service.execute(&call("dbx_file_edit", arguments)).await;
        let replayed_json = parsed(&replayed);
        assert_eq!(replayed_json["status"], "replayed");
        assert_eq!(replayed_json["writeApplied"], false);
    }

    #[tokio::test]
    async fn local_edit_preserves_utf8_bom_and_crlf_using_stat_hash() {
        let (_temp, root) = db_wiki();
        let path = root.join("notes.md");
        std::fs::write(&path, b"\xEF\xBB\xBF# Notes\r\nstatus: ready\r\n").unwrap();
        let service = AgentFileService::with_local_edit_enabled(true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let stat = service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "notes.md" }))).await;
        let hash = parsed(&stat)["contentHash"].as_str().unwrap().to_string();

        let updated = service
            .execute(&call(
                "dbx_file_edit",
                json!({
                    "scope_id": scope_id,
                    "path": "notes.md",
                    "expected_hash": hash,
                    "edits": [{ "op": "replace", "old_text": "status: ready", "new_text": "status: done" }]
                }),
            ))
            .await;
        assert!(!updated.is_error, "{}", updated.content);
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.starts_with(b"\xEF\xBB\xBF"));
        assert_eq!(&bytes[3..], b"# Notes\r\nstatus: done\r\n");
    }

    #[tokio::test]
    async fn local_edit_rejects_ambiguous_anchor_hash_conflict_and_invalid_format() {
        let (_temp, root) = db_wiki();
        std::fs::write(root.join("duplicate.md"), "same\nsame\n").unwrap();
        std::fs::write(root.join("data.json"), "{\"status\":\"ready\"}\n").unwrap();
        let service = AgentFileService::with_local_edit_enabled(true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();

        let duplicate_stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "duplicate.md" }))).await;
        let duplicate_hash = parsed(&duplicate_stat)["contentHash"].as_str().unwrap().to_string();
        let ambiguous = service
            .execute(&call(
                "dbx_file_edit",
                json!({
                    "scope_id": scope_id,
                    "path": "duplicate.md",
                    "expected_hash": duplicate_hash,
                    "edits": [{ "op": "replace", "old_text": "same", "new_text": "changed" }]
                }),
            ))
            .await;
        assert!(ambiguous.is_error);
        assert!(ambiguous.content.contains("FILE_EDIT_ANCHOR_AMBIGUOUS"));

        let conflict = service
            .execute(&call(
                "dbx_file_edit",
                json!({
                    "scope_id": scope_id,
                    "path": "duplicate.md",
                    "expected_hash": "0".repeat(64),
                    "edits": [{ "op": "replace", "old_text": "same\nsame", "new_text": "changed" }]
                }),
            ))
            .await;
        assert!(conflict.is_error);
        assert!(conflict.content.contains("FILE_HASH_CONFLICT"));

        let json_stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "data.json" }))).await;
        let json_hash = parsed(&json_stat)["contentHash"].as_str().unwrap().to_string();
        let invalid = service
            .execute(&call(
                "dbx_file_edit",
                json!({
                    "scope_id": scope_id,
                    "path": "data.json",
                    "expected_hash": json_hash,
                    "edits": [{ "op": "replace", "old_text": "\"ready\"", "new_text": "broken" }]
                }),
            ))
            .await;
        assert!(invalid.is_error);
        assert!(invalid.content.contains("FILE_EDIT_FORMAT_INVALID"));
        assert_eq!(std::fs::read_to_string(root.join("data.json")).unwrap(), "{\"status\":\"ready\"}\n");
    }

    #[tokio::test]
    async fn concurrent_local_edits_with_one_hash_allow_only_one_commit() {
        let (_temp, root) = db_wiki();
        std::fs::write(root.join("race.md"), "status: ready\n").unwrap();
        let service = AgentFileService::with_local_edit_enabled(true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let stat = service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "race.md" }))).await;
        let hash = parsed(&stat)["contentHash"].as_str().unwrap().to_string();
        let left = call(
            "dbx_file_edit",
            json!({
                "scope_id": scope_id,
                "path": "race.md",
                "expected_hash": hash,
                "edits": [{ "op": "replace", "old_text": "status: ready", "new_text": "status: left" }]
            }),
        );
        let right = call(
            "dbx_file_edit",
            json!({
                "scope_id": scope_id,
                "path": "race.md",
                "expected_hash": hash,
                "edits": [{ "op": "replace", "old_text": "status: ready", "new_text": "status: right" }]
            }),
        );

        let (left_result, right_result) = tokio::join!(service.execute(&left), service.execute(&right));
        let successes = [left_result.is_error, right_result.is_error].into_iter().filter(|is_error| !*is_error).count();
        assert_eq!(successes, 1);
        let final_text = std::fs::read_to_string(root.join("race.md")).unwrap();
        assert!(matches!(final_text.as_str(), "status: left\n" | "status: right\n"));
    }

    #[tokio::test]
    async fn local_remove_trashes_and_restores_a_hash_guarded_file() {
        let (_temp, root) = db_wiki();
        std::fs::create_dir(root.join("tables")).unwrap();
        let target = root.join("tables/legacy.md");
        std::fs::write(&target, "# Legacy\n").unwrap();
        let service = AgentFileService::with_features(true, true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "tables/legacy.md" }))).await;
        let hash = parsed(&stat)["contentHash"].as_str().unwrap().to_string();

        let deleted = service
            .execute(&call(
                "dbx_file_delete",
                json!({
                    "scope_id": scope_id,
                    "path": "tables/legacy.md",
                    "expected_hash": hash,
                    "reason": "migrated to the current table documentation"
                }),
            ))
            .await;
        let deleted_json = parsed(&deleted);
        assert_eq!(deleted_json["status"], "trashed");
        assert_eq!(deleted_json["deleteApplied"], true);
        assert!(!target.exists());
        let trash_ref = deleted_json["trashRef"].as_str().unwrap();
        assert_eq!(std::fs::read(root.join(trash_ref)).unwrap(), b"# Legacy\n");
        let receipt_ref = deleted_json["receiptRef"].as_str().unwrap().to_string();
        let manifest = std::fs::read_to_string(root.join(".dbx-wiki/manifest.json")).unwrap();
        assert!(!manifest.contains("tables/legacy.md"));

        let replayed = service
            .execute(&call(
                "dbx_file_delete",
                json!({
                    "scope_id": scope_id,
                    "path": "tables/legacy.md",
                    "expected_hash": hash,
                    "reason": "migrated to the current table documentation"
                }),
            ))
            .await;
        assert_eq!(parsed(&replayed)["status"], "replayed");

        let restored = service
            .execute(&call(
                "dbx_file_restore",
                json!({ "scope_id": scope_id, "receipt_ref": receipt_ref, "expected_missing": true }),
            ))
            .await;
        let restored_json = parsed(&restored);
        assert_eq!(restored_json["status"], "restored");
        assert_eq!(restored_json["restoreApplied"], true);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "# Legacy\n");
        let manifest = std::fs::read_to_string(root.join(".dbx-wiki/manifest.json")).unwrap();
        assert!(manifest.contains("tables/legacy.md"));
    }

    #[tokio::test]
    async fn directory_tools_create_empty_and_reject_non_empty_delete() {
        let (_temp, root) = db_wiki();
        let service = AgentFileService::with_features(true, true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();

        let created = service
            .execute(&call(
                "dbx_directory_create",
                json!({ "scope_id": scope_id, "path": "archive", "expected_missing": true }),
            ))
            .await;
        assert_eq!(parsed(&created)["status"], "created");
        assert!(root.join("archive").is_dir());

        let written = service
            .execute(&call(
                "dbx_file_write",
                json!({
                    "scope_id": scope_id,
                    "path": "archive/note.md",
                    "content": "# Note\n",
                    "expected_missing": true
                }),
            ))
            .await;
        assert!(!written.is_error, "{}", written.content);

        let non_empty = service
            .execute(&call(
                "dbx_directory_delete",
                json!({ "scope_id": scope_id, "path": "archive", "expected_empty": true }),
            ))
            .await;
        assert!(non_empty.is_error);
        assert!(non_empty.content.contains("DIRECTORY_NOT_EMPTY"));
        assert!(root.join("archive/note.md").is_file());

        let stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "archive/note.md" }))).await;
        let hash = parsed(&stat)["contentHash"].as_str().unwrap().to_string();
        let deleted_file = service
            .execute(&call(
                "dbx_file_delete",
                json!({
                    "scope_id": scope_id,
                    "path": "archive/note.md",
                    "expected_hash": hash,
                    "reason": "remove obsolete archive"
                }),
            ))
            .await;
        assert!(!deleted_file.is_error, "{}", deleted_file.content);

        let deleted_directory = service
            .execute(&call(
                "dbx_directory_delete",
                json!({ "scope_id": scope_id, "path": "archive", "expected_empty": true }),
            ))
            .await;
        assert_eq!(parsed(&deleted_directory)["status"], "deleted_empty_directory");
        assert!(!root.join("archive").exists());
    }

    #[tokio::test]
    async fn local_remove_protects_summary_and_keeps_file_directory_roles_separate() {
        let (_temp, root) = db_wiki();
        std::fs::create_dir(root.join("tables")).unwrap();
        let service = AgentFileService::with_features(true, true);
        let opened = service.execute(&call("dbx_file_open_scope", json!({ "path": root.to_string_lossy() }))).await;
        let scope_id = parsed(&opened)["scopeId"].as_str().unwrap().to_string();
        let summary_stat =
            service.execute(&call("dbx_file_stat", json!({ "scope_id": scope_id, "path": "SUMMARY.md" }))).await;
        let summary_hash = parsed(&summary_stat)["contentHash"].as_str().unwrap().to_string();

        let protected = service
            .execute(&call(
                "dbx_file_delete",
                json!({
                    "scope_id": scope_id,
                    "path": "SUMMARY.md",
                    "expected_hash": summary_hash,
                    "reason": "should be rejected"
                }),
            ))
            .await;
        assert!(protected.is_error);
        assert!(protected.content.contains("FILE_REMOVE_PROTECTED_PATH"));

        let file_tool_on_directory = service
            .execute(&call(
                "dbx_file_delete",
                json!({
                    "scope_id": scope_id,
                    "path": "tables",
                    "expected_hash": "0".repeat(64),
                    "reason": "wrong tool"
                }),
            ))
            .await;
        assert!(file_tool_on_directory.is_error);
        assert!(file_tool_on_directory.content.contains("FILE_DELETE_FILE_REQUIRED"));

        let directory_tool_on_file = service
            .execute(&call(
                "dbx_directory_delete",
                json!({ "scope_id": scope_id, "path": "SUMMARY.md", "expected_empty": true }),
            ))
            .await;
        assert!(directory_tool_on_file.is_error);
        assert!(directory_tool_on_file.content.contains("FILE_REMOVE_PROTECTED_PATH"));
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
