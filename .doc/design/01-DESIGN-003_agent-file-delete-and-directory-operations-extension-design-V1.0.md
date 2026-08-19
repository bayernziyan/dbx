---
title: "Agent 文件删除与目录操作扩展设计"
doc_id: "01-DESIGN-003"
version: "V1.0"
status: "Review"
created_date: "2026-08-19"
last_updated: "2026-08-19"
maintainer: "DBX DB-Wiki Project"
constraint_level: "Normative"
review_cycle: "On-demand"
related_docs:
  - "01-DESIGN-001"
  - "01-DESIGN-002"
tags:
  - "DBX"
  - "Agent"
  - "Extension"
  - "File-Delete"
  - "Directory-Operation"
  - "Trash"
  - "DB-Wiki"
---

# Agent 文件删除与目录操作扩展设计

## 1. 文档定位与权威边界

本文定义在不侵入 DBX 原有数据库、Provider、Agent 主循环和前端流程的前提下，通过现有 `AgentFunctionExtension`、`AgentFileService`、scope 和目录策略扩展点，为 `db-wiki` 增加受控文件删除、文件恢复、空目录创建和空目录删除能力。

权威边界如下：

1. `01-DESIGN-001` 仍是当前 Active 上位设计；其 V1 明确不开放 delete、move 和 rename。
2. 本文是独立的 Review 候选设计，不在 Review 状态下改变现有 Active 运行边界。
3. `01-DESIGN-002` 继续负责现有文件局部编辑；本文不修改 `dbx_file_edit` 契约。
4. 本文评审通过并升级为 Active 前，必须同步修订 `01-DESIGN-001` 的对应禁止项或增加明确的版本化例外。
5. 本文只授权扩展模块和最小注册点，不授权顺带重构 DBX 原有文件、数据库或 AI 主流程。

## 2. 核心决策

本版本采用四个显式工具：

```text
dbx_file_delete       # 将单个普通文件移入 scope 内回收站
dbx_file_restore      # 按 delete receipt 将文件恢复到原路径
dbx_directory_create  # 创建一个空目录
dbx_directory_delete  # 删除一个空目录
```

明确不采用：

- 不开放 shell、PowerShell、cmd、bash 或任意进程执行。
- 不把文件删除塞入 `dbx_file_edit` 的文本 `delete` 操作。
- 不让 `dbx_file_write` 通过空内容模拟文件删除。
- 不让文件工具兼容目录目标；文件和目录使用不同工具。
- 不开放永久文件删除给模型；对外的 delete 是可恢复的 scope 内移除。
- 不开放递归目录删除；非空目录必须先逐项处理。
- 不开放 move、rename 或跨 scope 操作。

## 3. 外部工具模式参考与本项目取舍

Codex `apply_patch` 将 Add、Update、Delete 和 Move 作为不同文件操作，并由 workspace sandbox 约束可写根目录。OpenCode 的 patch 同样区分 Add、Update、Move 和 Delete，文件修改统一受 edit 权限控制；目录和更广泛的文件系统动作通常依赖受权限约束的 shell。

参考：

- [Codex apply_patch instructions](https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/apply_patch_tool_instructions.md)
- [Codex workspace-write policy](https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/permissions/sandbox_mode/workspace_write.md)
- [OpenCode tools](https://opencode.ai/docs/tools)
- [OpenCode permissions](https://opencode.ai/v2/docs/permissions)

DBX 的取舍更窄：只在已打开的 `db-wiki` scope 内提供确定性 function tool，不复制通用 patch 语言和宿主 shell 权限。

## 4. 非侵入约束

### 4.1 禁止修改的主流程

本设计禁止修改：

```text
crates/dbx-core/src/agent_loop.rs
crates/dbx-core/src/agent_events.rs
crates/dbx-core/src/agent_tools.rs
crates/dbx-core/src/ai.rs
crates/dbx-core/src/storage.rs
crates/dbx-mcp/
src-tauri/
apps/desktop/
Provider 请求、流式解析和多轮推理
数据库连接、SQL 权限和执行链
现有 dbx_file_* 与 dbx_wiki_* 输入输出契约
DirectoryAllowlistPolicy trait
```

### 4.2 允许修改的扩展面

主体实现只能新增在：

```text
crates/dbx-core/src/agent_files/local_remove/
├── mod.rs
├── contract.rs
├── feature_gate.rs
├── file_delete.rs
├── directory_ops.rs
├── trash.rs
├── receipt.rs
└── safety.rs
```

现有源码只允许三个最小注册点和必要的 helper 可见性调整：

```text
agent_files/mod.rs          # 声明 local_remove 模块
agent_files/tool_catalog.rs # feature 开启时追加工具定义和 handles
agent_files/file_tools.rs   # 复用 scope 并分派到 local_remove
agent_files/file_write.rs   # 仅在必要时把既有无副作用 helper 调整为 pub(super)
```

这些注册改动不得改变 feature 关闭时的工具名称、顺序、Schema、read_only 标记和执行结果。

### 4.3 禁止为了“更整洁”引入的重构

- 不拆分或重写 `AgentFileService`。
- 不新增 crate、workspace member 或数据库表。
- 不抽象统一 FileMutationEngine。
- 不把现有 scope registry 搬到全局服务。
- 不修改 policy trait 增加 delete hook。
- 不给 Agent Loop 增加删除专用系统提示。

## 5. Feature Gate 与模式暴露

新增独立开关：

```text
DBX_AGENT_FILE_DELETE_TOOLS=1|true|on  -> 注册四个工具
其他值或环境变量缺失                  -> 不注册四个工具
```

规则：

1. 本能力默认关闭，避免升级后无配置地扩大破坏性工具面。
2. `DBX_AGENT_FILE_TOOLS`关闭时，本开关无效。
3. 四个工具只在 Agent 模式暴露，全部标记 `read_only=false`；Ask 模式不可见。
4. 开关关闭时，现有工具快照必须与改造前一致。
5. 工具选择规则只写入各自 description，不修改 Agent Loop 系统提示。
6. `DBX_AGENT_FILE_EDIT_TOOLS` 与本开关相互独立。

## 6. 共用安全前置条件

四个工具必须统一执行：

1. 使用调用参数中的 `scope_id` 从现有 registry 获取 `FileDirectoryScope`。
2. scope 不存在时原样返回 `FILE_SCOPE_NOT_FOUND`，由既有恢复策略重新 open 后重试。
3. scope 必须是 `policyId=db-wiki` 且 `accessMode=read-write`。
4. `path` 必须是规范相对路径；拒绝绝对路径、UNC、盘符、前导分隔符、`.`、`..` 和空组件。
5. 逐级拒绝 symlink 和 Windows reparse point。
6. 拒绝 scope 根目录本身。
7. 拒绝模型访问 `.dbx-wiki` 内部控制目录。
8. 拒绝路径大小写或规范化后指向受保护目标。

受保护目标至少包括：

```text
.dbx-wiki/
SUMMARY.md
```

`SUMMARY.md` 是 Wiki 入口，不允许模型通过 delete 删除；需要替换入口时使用既有 hash-guarded edit/write。

## 7. `dbx_file_delete` 契约

### 7.1 语义

`dbx_file_delete` 只接受现存普通文件。成功后，原路径立即不存在，但原始字节被原子移动到 scope 内部回收站，可由 `dbx_file_restore` 恢复。

### 7.2 输入 Schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "scope_id": { "type": "string", "minLength": 1 },
    "path": { "type": "string", "minLength": 1 },
    "expected_hash": {
      "type": "string",
      "pattern": "^[A-Fa-f0-9]{64}$"
    },
    "reason": { "type": "string", "minLength": 1, "maxLength": 500 }
  },
  "required": ["scope_id", "path", "expected_hash", "reason"]
}
```

约束：

- `expected_hash` 必须来自删除前的 `dbx_file_stat.contentHash`。
- `reason` 用于 receipt，不作为授权证明，不得包含 secret 或大段文件内容。
- 目标是目录时返回 `FILE_DELETE_FILE_REQUIRED`。
- 扩展名必须在当前 policy 的写白名单内。
- 不提供 `force`、`recursive`、`permanent` 或 glob 参数。

### 7.3 成功输出

```json
{
  "schemaVersion": 1,
  "toolCallId": "provider-tool-call-id",
  "status": "trashed",
  "deleteApplied": true,
  "path": "tables/legacy.md",
  "previousHash": "...",
  "trashRef": ".dbx-wiki/.trash/<fingerprint>/payload",
  "receiptRef": ".dbx-wiki/.deletions/<fingerprint>.receipt",
  "manifestStatus": "succeeded"
}
```

### 7.4 幂等重放

相同 `policy_id + relative_path + expected_hash` 形成删除 fingerprint。

- 原路径不存在、receipt 存在且 trash payload hash 匹配时返回 `replayed`。
- 原路径存在且 hash 仍匹配时允许首次执行。
- 原路径存在但 hash 不匹配时返回 `FILE_HASH_CONFLICT`。
- 原路径不存在且没有合法 receipt 时返回 `FILE_NOT_FOUND`，不得猜测已删除成功。

## 8. `dbx_file_restore` 契约

### 8.1 语义

恢复工具只能按服务端生成的 `receipt_ref` 把 payload 恢复到 receipt 记录的原路径。模型不能在恢复时指定任意目标路径。

### 8.2 输入 Schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "scope_id": { "type": "string", "minLength": 1 },
    "receipt_ref": { "type": "string", "minLength": 1 },
    "expected_missing": { "const": true }
  },
  "required": ["scope_id", "receipt_ref", "expected_missing"]
}
```

约束：

- `receipt_ref` 必须严格匹配 `.dbx-wiki/.deletions/<sha256>.receipt`。
- receipt、payload、原路径和 hash 必须一致。
- 原路径已存在时返回 `FILE_RESTORE_TARGET_EXISTS`，不覆盖。
- 原父目录不存在时返回 `FILE_RESTORE_PARENT_NOT_FOUND`，不自动创建。
- 恢复后重新同步 Manifest。

## 9. `dbx_directory_create` 契约

### 9.1 语义

目录创建工具只创建一个空目录，父目录必须已经存在。多级目录需要从父到子逐次调用，避免一次调用跨越多个未经逐级校验的组件。

### 9.2 输入 Schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "scope_id": { "type": "string", "minLength": 1 },
    "path": { "type": "string", "minLength": 1 },
    "expected_missing": { "const": true }
  },
  "required": ["scope_id", "path", "expected_missing"]
}
```

规则：

- 目标已存在时返回 `DIRECTORY_ALREADY_EXISTS`。
- 父目录不存在时返回 `DIRECTORY_PARENT_NOT_FOUND`。
- 父目录是 symlink/reparse point 时拒绝。
- 不允许创建 `.dbx-wiki` 或点号开头的内部目录。
- 成功后返回 `status=created`、相对路径和 receiptRef。
- 空目录不进入 Manifest；后续新建文件后由既有 afterWrite 同步 Manifest。

## 10. `dbx_directory_delete` 契约

### 10.1 语义

目录删除工具只删除一个空目录，不递归扫描和删除内容。空目录不承载文件数据，因此 V1 直接删除目录项并保存审计 receipt，不创建 trash payload。

### 10.2 输入 Schema

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "scope_id": { "type": "string", "minLength": 1 },
    "path": { "type": "string", "minLength": 1 },
    "expected_empty": { "const": true }
  },
  "required": ["scope_id", "path", "expected_empty"]
}
```

规则：

- 目标不是目录时返回 `DIRECTORY_REQUIRED`。
- 目录存在任何条目时返回 `DIRECTORY_NOT_EMPTY`，并只返回有界 `entryCount`，不自动删除内容。
- scope 根、`.dbx-wiki`、受保护路径及其父目录不可删除。
- 最终调用操作系统空目录删除；若并发写入导致目录变为非空，返回冲突，文件不受影响。
- 成功返回 `status=deleted_empty_directory` 和 receiptRef。

## 11. Trash 与 Receipt 布局

内部目录固定为：

```text
<scope>/.dbx-wiki/
├── .trash/
│   └── <fingerprint>/
│       └── payload
├── .deletions/
│   └── <fingerprint>.receipt
└── .directory-mutations/
    └── <fingerprint>.receipt
```

要求：

1. fingerprint 和内部路径完全由服务端生成。
2. 模型不能用通用 read/list/search/stat/write/edit 访问内部目录。
3. receipt 不记录绝对路径、文件内容、secret 或 PII 原文。
4. 文件 payload 保留原始字节、BOM、换行和编码，不做文本解析或重写。
5. trash payload hash 必须等于删除前 `expected_hash`。
6. 删除 receipt 至少记录 schemaVersion、toolCallId、policyId、relativePath、previousHash、trashRef、reason、manifestStatus 和 epoch 时间。
7. 目录 receipt 只记录相对路径、操作类型和 epoch 时间。

## 12. 原子性、并发与失败状态

### 12.1 文件删除

```text
1. 获取规范化目标路径级异步锁
2. stat 并确认普通文件
3. 验证 expected_hash
4. 创建并校验内部 trash 目录
5. 写入 pending receipt
6. 再次验证源文件 hash
7. 同 volume rename 到 trash payload
8. 验证 payload hash
9. 同步 Manifest
10. 完成 receipt
```

源文件与 trash 位于同一 scope、同一文件系统，使用 rename 保证源路径移除和 payload 出现是单个文件系统操作。不得用 copy 后 remove 模拟原子删除。

### 12.2 文件恢复

恢复同样使用同 volume rename。目标已存在或父目录异常时必须失败关闭，不覆盖现有文件。

### 12.3 目录操作

目录创建使用单级 `create_dir`；空目录删除使用 `remove_dir`。不使用 `create_dir_all` 和递归删除 API。

### 12.4 部分成功状态

| status | `is_error` | 含义 |
|---|---:|---|
| `trashed` | false | 原路径移除、payload 与 Manifest 正常 |
| `replayed` | false | 相同删除已完成，返回既有 receipt |
| `restored` | false | payload 已恢复到原路径 |
| `created` | false | 空目录创建成功 |
| `deleted_empty_directory` | false | 空目录删除成功 |
| `trashed_manifest_unknown` | true | 文件已移入 trash，但 Manifest 状态未确认 |
| `restored_manifest_unknown` | true | 文件已恢复，但 Manifest 状态未确认 |
| `receipt_pending` | true | 文件状态已变化但 receipt 收尾失败，返回可核验 hash 和引用 |
| `conflict` | true | hash、目标存在状态或目录空状态发生变化 |
| `rejected` | true | scope、路径、policy 或受保护目标校验失败 |

所有部分成功结果必须明确 `deleteApplied` 或 `restoreApplied`，禁止只返回笼统错误。

## 13. Manifest 与 Wiki 一致性

本扩展不修改 `DirectoryAllowlistPolicy` trait。

- 文件 delete/restore 后直接复用现有 `sync_db_wiki_manifest` 确定性重建 Manifest。
- 目录创建和空目录删除不影响 Manifest document 列表，无需同步。
- Manifest 失败时重新核对源路径、payload 和 hash，再返回部分成功状态。
- `.dbx-wiki` 内部 trash 和 receipt 继续被 Manifest、搜索和证据构建忽略。
- 本扩展不自动编辑 `SUMMARY.md`；业务索引变化由调用方在删除前通过既有 edit/write 工具完成。

## 14. Agent 调用策略

### 14.1 兼容迁移

需要保留旧链接时：

```text
dbx_wiki_update_from_session 创建新文件
-> dbx_file_edit 把旧文件改为迁移说明
-> dbx_file_edit 更新 SUMMARY.md
-> 保留旧文件，不调用 delete
```

### 14.2 清理迁移

用户明确要求删除旧文件时：

```text
dbx_wiki_update_from_session 创建新文件
-> dbx_file_edit 更新 SUMMARY.md
-> dbx_file_stat 获取旧文件 raw hash
-> dbx_file_delete 移入 trash
```

不应先把旧文件改成重定向占位再立即删除；两种模式只选一种。

### 14.3 新目录

```text
dbx_directory_create 创建父目录
-> dbx_wiki_update_from_session 在目录中创建文件
```

### 14.4 删除目录

```text
dbx_file_list 确认目录内容
-> 对明确要求删除的文件逐个 stat + delete
-> dbx_directory_delete 删除已空目录
```

模型不得根据 glob 或目录名推断批量删除意图。

## 15. 最小代码改造清单

| 文件 | 允许改动 | 对原有 DBX 的影响 |
|---|---|---|
| `agent_files/local_remove/*` | 新增全部契约、实现、trash 和 receipt | 独立扩展目录 |
| `agent_files/mod.rs` | 新增一行模块声明 | 无旧行为变化 |
| `agent_files/tool_catalog.rs` | 开关开启时追加四个工具 | 关闭时工具快照不变 |
| `agent_files/file_tools.rs` | 新增 feature 状态和四个局部分派 | 不改旧工具分支 |
| `agent_files/file_write.rs` | 最多调整 Manifest/hash helper 可见性 | 不改实现语义 |
| `agent_files/mod.rs` tests | 增加扩展开关和端到端测试 | 测试范围内 |

实现不需要：

- 修改 Agent Loop；
- 修改前端审批 UI；
- 增加数据库 migration；
- 修改 MCP 协议；
- 修改 SQL 权限；
- 新增第三方依赖。

## 16. 测试设计

### 16.1 零影响与工具契约

- 默认关闭时工具定义和 handles 快照与改造前一致。
- 显式开启时四个工具只在 Agent 模式可见。
- 所有 Schema 使用 `additionalProperties=false`。
- file 和 directory 工具互相拒绝错误目标类型。

### 16.2 路径与安全

- 绝对路径、UNC、`..`、`.`、空组件和前导分隔符拒绝。
- symlink/reparse point 源、父目录和 trash 内部路径拒绝。
- scope 根、`SUMMARY.md` 和 `.dbx-wiki` 拒绝。
- read-only scope 和非 db-wiki policy 拒绝。

### 16.3 文件删除与恢复

- 正确 hash 删除后原路径消失，payload 字节和 hash 完全一致。
- 错误 hash 不移动文件。
- 目录目标被 file delete 拒绝。
- 相同调用返回 replayed，不创建第二份 payload。
- restore 只恢复到 receipt 原路径。
- restore 遇到现存目标时不覆盖。
- delete/restore 后 Manifest 不包含/重新包含目标文件。
- Manifest 和 receipt 失败返回准确部分成功状态。

### 16.4 目录操作

- 创建单个空目录成功。
- 父目录缺失时拒绝，不隐式 create_dir_all。
- 删除空目录成功。
- 非空目录拒绝且内容保持不变。
- 并发新增文件时 remove_dir 失败关闭。
- 不存在 recursive 参数和递归代码路径。

### 16.5 回归命令

```powershell
cargo fmt --all -- --check
cargo test -p dbx-core --no-default-features --lib agent_files::tests -- --nocapture
cargo test -p dbx-core --no-default-features --lib agent_loop::tests:: -- --nocapture
cargo check -p dbx-core --no-default-features
```

Agent Loop focused tests只用于证明工具扩展没有改变主循环，不为 delete 修改 Agent Loop 测试代码。

## 17. 分阶段交付

### D0：扩展骨架与关闭态证明

- 新增 `local_remove` 空实现骨架和显式 feature 构造选项。
- 完成关闭态工具快照测试。
- 不向正式 catalog 暴露未完成工具。

### D1：目录操作

- 实现单级目录创建和空目录删除。
- 完成路径、reparse point、根目录和非空保护测试。

### D2：文件 trash delete

- 实现 hash guard、路径锁、pending receipt、原子 rename 和 replay。
- 完成 Manifest 协调和部分成功状态。

### D3：文件 restore

- 实现 receipt 驱动恢复、目标存在保护和 Manifest 协调。
- 完成 delete/restore 往返测试。

### D4：评审与启用

- 完成 focused tests 和本地真实 `db-wiki` 验证。
- 修订 `01-DESIGN-001` 上位边界。
- 本文从 Review 升级为 Active 后，才允许在受控配置中开启。

## 18. 验收标准

1. 所有实现主体位于 `agent_files/local_remove/`。
2. 未修改本文4.1列出的主流程和协议。
3. feature 默认关闭，关闭态旧工具快照和行为不变。
4. delete 只能作用于单个普通文件并要求 raw `expected_hash`。
5. delete 通过同 scope 原子 rename 移入不可由模型直接访问的 trash。
6. restore 只能按 receipt 恢复到原路径且不得覆盖。
7. 目录创建只创建单级空目录。
8. 目录删除只删除空目录，不存在递归删除路径。
9. scope 根、`SUMMARY.md` 和 `.dbx-wiki` 始终不可删除。
10. symlink、reparse point、scope escape 和非 db-wiki policy 全部拒绝。
11. 文件 delete/restore 后 Manifest 确定性同步。
12. 任何部分成功状态明确实际文件状态、hash 和恢复引用。
13. 不增加数据库表、前端页面、MCP 协议或第三方依赖。
14. focused tests、format check 和 no-default-features check 通过。

## 19. 回滚与清理

功能回滚只需关闭 `DBX_AGENT_FILE_DELETE_TOOLS`，四个工具立即从模型工具集消失，现有 read/write/edit/Wiki 工具不变。

代码回滚只需移除：

- `agent_files/local_remove/`；
- `mod.rs`模块声明；
- tool catalog 的条件注册；
- file_tools 的局部分派；
- helper 可见性调整。

已进入 trash 的文件不因关闭 feature 自动恢复或永久删除。运维可以依据 receipt 进行核验恢复。trash 永久清理由独立运维策略负责，不作为 Agent function tool 暴露，也不在本文实现自动定时清理。

## 20. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 模型误删文件 | 默认关闭、Agent-only、单文件、hash guard、trash 可恢复 |
| 批量删除扩大影响 | 无 glob、无 recursive、无目录非空删除 |
| 路径逃逸 | 复用 scope 规范化并逐级拒绝 symlink/reparse point |
| 删除后 Manifest 漂移 | 确定性重建并返回明确部分成功状态 |
| receipt 丢失导致状态不明 | pending receipt、payload hash 和 replay 协调 |
| restore 覆盖新文件 | 强制 expected_missing，目标存在即拒绝 |
| trash 累积占用空间 | 不自动清理；交由受控运维策略按 receipt 和 hash 处理 |
| 注册改动影响旧工具 | 默认关闭快照测试和独立 feature gate |
| 设计与 Active 上位文档冲突 | Review 状态不授权实现；Active 前同步修订上位边界 |

## 21. 已决策与后续项

### 21.1 已决策

- 删除能力采用独立 `local_remove` 扩展，不进入 `local_edit`。
- 文件 delete 默认移动到 scope 内 trash，不开放永久删除。
- 提供 receipt 驱动的文件 restore。
- 文件与目录工具分离。
- 目录创建只支持单级，目录删除只支持空目录。
- feature 默认关闭且只在 Agent 模式暴露。
- 不修改 Agent Loop、Provider、Storage、前端、MCP 和数据库权限。
- 不修改 `DirectoryAllowlistPolicy` trait。
- 不提供 move、rename、recursive、glob 和 shell。

### 21.2 后续独立评审项

- 是否提供受控的 trash retention/purge 运维命令。
- 是否允许非空目录整体移入 trash。
- 是否将删除扩展桥接到 MCP 或 CLI Provider。
- 是否为其他 write policy 开放相同能力。
- 是否增加 Wiki 引用反向检查，在删除前报告引用目标。
