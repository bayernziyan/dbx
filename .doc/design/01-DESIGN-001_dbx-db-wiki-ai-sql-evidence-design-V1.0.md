---
title: "DBX 数据库 Wiki 目录与 AI SQL 证据闭环设计"
doc_id: "01-DESIGN-001"
version: "V1.0"
status: "Active"
created_date: "2026-08-18"
last_updated: "2026-08-19"
maintainer: "DBX DB-Wiki Project"
constraint_level: "Normative"
review_cycle: "On-demand"
related_docs: []
tags:
  - "DBX"
  - "DB-Wiki"
  - "AI-SQL"
  - "MCP"
  - "Text-to-SQL"
---

# DBX 数据库 Wiki 目录与 AI SQL 证据闭环设计

## 1. 文档定位

本文是 DBX 内部 AI Agent 读取和更新提示词指定的 `db-wiki` 目录、支撑 AI SQL 生成和知识反哺的 V1.0 最终执行设计，状态为 `Active`。M1-M3 的实现边界、工具契约、目录白名单、安全规则、验收指标和非侵入约束以本文为权威；21.2 仅是非阻塞后续项，不影响 V1 执行。

本文覆盖：

- DBX 内置 AI Agent 如何从提示词提取数据库 Wiki 目录并建立临时 `db-wiki` 读写作用域。
- 当前 MiniMax 等 API 模型如何复用现有 Agent loop，多轮调用目录文件发现、读取和解析工具。
- 如何把 Wiki、实时数据库结构和 DBX 现有 Database Docs 组合成 SQL 证据包。
- 如何记录查询过程中的知识缺口，并自动更新当前 `db-wiki` 目录内的文件。
- 如何保证scope隔离、目录白名单、来源追溯和回滚。

本文不包含具体代码实现。数据库写操作继续遵守 DBX 原有权限；文件写入只允许发生在已校验的 `db-wiki` scope 内，并使用 expected hash、格式校验和原子替换，不需要逐文件前端审批。

## 2. 源码基线与已确认现状

### 2.1 源码基线

| 项目 | 值 |
|---|---|
| 来源 | `C:/Users/ziyan/Downloads/Compressed/dbx-main.zip` |
| ZIP SHA-256 | `3084DE21A3C0F819D5DEB3989AA50D6ACBA90819DCE15C61BEFC9D2C01A944C0` |
| DBX 版本 | `0.5.87` |
| 前端包管理器 | `pnpm@10.27.0` |
| 新项目目录 | `E:/workspace/dbx-db-wiki` |
| Git 历史 | ZIP 不包含 `.git`，当前目录是源码快照而非完整 Git clone |

### 2.2 DBX 已有能力

DBX 已有 Database Docs，不需要重新开发一套基础数据库文档浏览器：

- `crates/dbx-core/src/docs/collector.rs`：采集实时 Schema。
- `crates/dbx-core/src/docs/snapshot.rs`：`SchemaSnapshot`、表、字段、索引、外键、枚举和关系模型。
- `crates/dbx-core/src/docs/annotations.rs`：可提交仓库的本地注释文件、原子保存和数据库注释覆盖规则。
- `crates/dbx-core/src/docs/export.rs`：独立 HTML 导出。
- `src-tauri/src/commands/docs.rs`：采集、加载、合并、保存和导出命令。
- `apps/desktop/src/docs/`：Wiki 索引、搜索、表详情、字段、枚举、关系图和注释编辑 UI。
- `apps/desktop/src/components/docs/DatabaseDocsDialog.vue`：Database Docs 对话框和注释自动保存。

DBX AI Agent已有数据库工具，但没有`db-wiki`目录文件工具：

- `crates/dbx-core/src/agent_tools.rs` 提供表、字段、样例、查询和 Explain 工具。
- `crates/dbx-core/src/ai_cli_agent.rs` 控制 Pi Agent 可见的 DBX MCP 工具白名单。
- `crates/dbx-core/src/ai_pi_agent_cli.rs` 将 DBX MCP 通过 Bridge 注册到 Pi Coding Agent。
- `crates/dbx-mcp/src/server.rs` 暴露连接、Schema 和 SQL 工具。

### 2.3 当前 db-wiki 特征

现有 BPM96 `db-wiki` 是项目内 Markdown 知识库，包含：

- `SUMMARY.md`：入口和表索引。
- `tables/`：表、字段、状态和值域。
- `enums/`：枚举和布尔标记。
- `relationships.md`：表与字段关系。
- `queries/`：已知业务查询和运维 SQL。
- 场景分析文档：流程初始化、模板发布、搜索等。

当前不足：

- 文档缺少统一、机器可解析的来源和可信度元数据。
- 没有统一索引和结构化检索接口。
- Wiki、Database Docs 注释和实时 Schema 之间存在潜在双重权威。
- 查询成功、失败和用户修正没有形成审核式知识闭环。
- 当前 DBX Agent 无法直接使用这些 Markdown 作为 SQL 证据。
- DBX 内部 MiniMax Agent 已有多轮 function-calling 和数据库工具，但缺少通用目录只读文件发现/解析，以及基于 `db-wiki` 写白名单的更新和会话知识沉淀能力。

## 3. 目标与非目标

### 3.1 目标

1. 用户在提示词或 Prompt 模板中提供一个具体绝对目录；任意通过共享路径安全校验的目录均可只读，数据库 Wiki 目录的末级名称为 `db-wiki`。
2. Agent调用通用`dbx_file_open_scope`后，DBX校验真实目录并返回本次会话临时scope id；未命中写策略时使用`generic-read-only`，V1内置`db-wiki`读写策略。
3. scope 建立后，AI SQL 生成前必须获得有引用的 Wiki Evidence Pack；未提供有效目录时保持原有兼容流程。
4. Wiki 不足时，允许使用 DBX 实时 Schema 工具补充物理事实。
5. 查询过程产生缺口、漂移、纠正和已验证 SQL 候选。
6. scope 内允许 Agent 自动创建和更新白名单文件，不增加前端逐文件审批；写入必须进行 expected hash、敏感信息、格式和原子替换校验。
7. DBX 内置 Agent 与外部 MCP 客户端共享相同的 Wiki 核心服务。
8. Wiki索引可随时从scope目录文件重建，不成为第二权威源。
9. DBX 内部 AI Agent 使用当前配置的 MiniMax 等模型，在 `db-wiki` scope 内完成多轮规划、文件搜索/读取、数据库取证、SQL 生成和下一步判断。
10. Agent 可以把当前会话总结为带 Wiki/数据库 Citation 的结构化知识，并直接更新当前 scope 内的 Wiki 文件。
11. 通用文件工具不包含Wiki业务判断；任意目录复用同一套list/search/read/parse协议，未来写场景通过新增目录白名单policy复用write协议，V1只交付`db-wiki`写policy。

### 3.2 非目标

- 不开放 shell、进程执行或未在用户/模板消息中明确提供的目录。模型可以提取任意绝对目录作为只读根目录，但 scope 建立后只能使用相对路径访问该目录内部。
- 不引入项目、Workspace 或 projectId 概念；源码目录只有在用户明确给出绝对路径时才可只读，除非命中独立写白名单，否则不可修改。
- 不自动收集或沉淀密码、Token、个人数据和大段业务样本。
- 不因一次查询成功就自动修改 Wiki。
- 不把完整 OpenMetadata/DataHub 平台嵌入 DBX。
- 第一阶段不引入向量数据库或模型微调。
- 不改变 DBX 现有数据库 SQL 权限模型。

## 4. 核心原则

### 4.1 权威源分层

| 信息类型 | 首要来源 | 冲突处理 |
|---|---|---|
| 实时表、字段、索引、外键 | DBX `SchemaSnapshot` | 实时结构优先，记录Schema Drift并按任务要求更新Wiki |
| 预期表结构和迁移 | 源码实体、DDL、迁移脚本 | 与线上不一致时同时保留预期态和运行态 |
| 业务含义、术语、枚举语义 | 已审核 db-wiki | 不允许 AI 根据字段名直接猜测 |
| 数据库 COMMENT | 实时数据库 | 作为物理元数据，可被本地已审核说明覆盖但不可丢失 |
| 查询案例 | 用户确认通过的 SQL | 未确认的 SQL 只能作为候选 |
| AI 推断 | 无 | 必须标记 `provisional`，不能成为权威事实 |

### 4.2 单一核心服务

Wiki解析、搜索、证据包、scope写入和安全检查必须放在`dbx-core`。Desktop、内置Agent和MCP Server只做适配，禁止各自实现一套逻辑。

### 4.3 白名单内自动写入

`db-wiki` scope 建立后，Agent 可以在其内部自动创建或更新白名单文件，不需要前端确认。写入边界：

- 每次写入携带读取时的 expected hash；目标被外部修改时拒绝覆盖并要求重新读取。
- 采用同目录唯一临时文件、flush 和原子替换。
- 写入前执行路径、扩展名、格式、secret/PII 和内容安全检查。
- 更新后写入审计、同步 Manifest 并重建受影响索引。
- V1 不提供删除、移动、重命名和目录外写入。

## 5. 总体架构

```text
DBX Desktop AI Agent Service
  ├── current AI config: MiniMax / other API model
  ├── conversation + prompt template
  └── existing Native Agent Loop
          │
          ├── parse db-wiki absolute path from prompt
          ├── dbx_file_open_scope(path)
          ├── generic file function tools
          └── db-wiki semantic + database tools
          │
          ▼
Evidence Ledger + Open Questions
          │
    evidence enough?
      ├── no  -> next file/wiki/schema tool call
      └── yes -> SQL / analysis / automatic wiki update
                         │
                         ▼
              Atomic Write + Manifest + Reindex
```

DBX AI Agent 服务始终是用户入口。模型负责从提示词解析目录、决定下一次读取和停止时机；`dbx-core` 负责通用目录只读安全校验、`db-wiki` 写白名单校验、确定性文件工具、数据库权限、证据账本、预算和安全写入。

### 5.1 Native Agent Loop（V1 唯一路径）

- 继续使用 DBX 现有 `run_agent_loop`，MiniMax 保持当前 endpoint、API key、model、thinking、tool calling 和上下文配置。
- 仅追加通用`dbx_file_*`function tools和`dbx_wiki_*`语义工具，不修改现有模型请求、上下文压缩、取消、数据库工具和SQL权限语义。
- OpenCode/Codex 不参与 V1 执行路径，也不需要传入 workspace。
- 未成功建立FileDirectoryScope时拒绝执行通用文件工具，普通数据库Agent行为保持不变。

### 5.2 最小侵入原则

- 不重写现有 Agent loop，也不改变 MiniMax Provider 协议；只增加目录 scope 与 function tools 扩展。
- 不修改现有 `execute_query`、SQL 风险分类、连接只读、生产保护和精确 SQL 确认语义。
- 不开发目录选择、Workspace管理、逐文件Diff审批和项目映射前端。
- 功能未启用或提示词没有有效绝对目录路径时，不改变现有 Agent 行为。

## 6. db-wiki 目录作用域与隔离

### 6.1 FileDirectoryScope

`dbx_file_open_scope`先对提示词解析出的绝对路径执行共享安全校验，再选择访问策略并生成进程内临时scope。普通目录使用只读fallback，命中写白名单的目录获得读写能力。scope跨同一聊天的多次Agent请求保留，直到显式close或DBX重启：

```json
{
  "scopeId": "wiki-scope-opaque-id",
  "rootPath": "E:/workspace/example/.memory/db-wiki",
  "policyId": "db-wiki",
  "accessMode": "read-write"
}
```

规则：

1. 根路径只来自当前提示词或选中的 Prompt 模板，不从当前工作目录推断、不持久化为项目配置。
2. `scopeId`是不透明随机值，绑定规范化根路径并保存在进程级共享registry；单次Agent请求结束或取消不会清除，显式close或DBX重启后失效。
3. 文件工具建立 scope 后只接受 `scopeId + relativePath`，不再接受绝对路径。
4. scope 文件读写权限与数据库连接权限独立；建立 Wiki scope 不提升数据库写权限。
5. 前端跨轮历史不持久化工具结果中的`scopeId`；因此每个需要文件的新用户请求都先以会话中明确路径调用`open_scope`，不得从助手文本猜测或复用ID。重复打开同一路径是安全操作，Agent不得在任务结束时自动close，除非用户明确要求或主动放弃该根目录。

### 6.2 提示词目录提取

Agent 从提示词中提取明确的目录文本，例如：

```text
数据库 Wiki 目录：E:\workspace\bpma-bpm96-v\.memory\db-wiki
```

规则：

1. 只有用户/模板消息中明确出现的路径可用于 `open_scope`；模型不能根据环境变量、用户名或相似目录自行猜测。
2. 同一提示词存在多个候选绝对目录时返回`FILE_DIRECTORY_AMBIGUOUS`，要求用户明确，不自动选择。
3. 路径缺失时返回`FILE_DIRECTORY_REQUIRED`；Agent仍可使用原有数据库工具，但不能调用文件工具。
4. 路径只用于本次会话，不写入 Manifest、Wiki 内容或查询轨迹。

### 6.3 目录白名单策略注册表

通用文件层不硬编码`db-wiki`，而是通过`DirectoryAllowlistPolicy`扩展：

```text
DirectoryAllowlistPolicy
├── id()
├── matches(canonicalRoot)
├── validateRoot(canonicalRoot)
├── accessMode() -> read-only | read-write
├── allowedExtensions()
└── afterWrite(relativePath)
```

`dbx_file_open_scope`先执行所有策略共享的基础校验：

1. 路径是绝对路径，词法归一化后不含未解析 `.`、`..` 或设备路径别名。
2. 目录真实存在，规范化真实路径的每级父目录不存在符号链接或NTFS重解析点逃逸。
3. 写策略注册表按规范化根目录匹配；命中唯一策略时执行`validateRoot`并获得其访问能力，多个策略匹配返回`FILE_SCOPE_POLICY_AMBIGUOUS`。
4. 没有写策略匹配时自动使用`generic-read-only`，允许受格式、大小和路径边界约束的读取；任何写操作返回`FILE_SCOPE_READ_ONLY`。

V1内置策略：

```json
{
  "id": "db-wiki",
  "leafDirectoryName": "db-wiki",
  "accessMode": "read-write",
  "afterWrite": ["sync-manifest", "reindex"]
}
```

此外内置`generic-read-only` fallback，适用于所有通过共享安全校验但未命中写策略的绝对目录。当前只要共享安全校验通过即可建立只读scope；末级目录名等于`db-wiki`时升级为读写scope，`SUMMARY.md`和Manifest可以在后续写入/同步时创建。后续写场景通过注册新policy扩展，不修改通用工具名称、参数和执行流程。系统不引入projectId，也不要求目录属于代码项目。

### 6.4 scope 内路径语义

```json
{
  "scopeId": "wiki-scope-opaque-id",
  "relativePath": "tables/ecl_tasks.md"
}
```

规则：

1. `relativePath` 必须是相对路径，禁止盘符、UNC、前导 `/`/`\`、`..` 和空组件。
2. 拼接后对目标及现存父目录重新规范化，最终路径必须仍在 scope 根目录内。
3. 新建文件只允许白名单扩展名；V1 不允许删除、移动、重命名或创建根目录外的目录。
4. 同一会话切换根目录必须先关闭旧 scope 并重新 `open_scope`，旧 scope id 立即失效。

## 7. 与现有 Database Docs 的融合

### 7.1 复用范围

直接复用：

- `SchemaSnapshot` 作为实时物理结构模型。
- `DocTable`、`ColumnInfo`、`Relationship`、`DocEnum`。
- Database Docs 搜索、表详情、枚举、关系和 HTML 导出 UI。
- `docs_notes_path` 配置能力。
- `save_annotations` 的同目录临时文件、flush 和原子 rename 模式。
- Rust/TypeScript fixture conformance 测试模式。

### 7.2 不直接复用的边界

现有 `AnnotationFile` 只适合项目、分组、表和字段的简短说明，不应强行承载：

- 查询案例；
- 业务术语；
- 参数定义；
- 来源、可信度和验证时间；
- 自动更新审计；
- 查询反馈。

因此不把全部 db-wiki 塞进 `AnnotationFile`。新增 `WikiKnowledgeStore`，并通过 Overlay 合并到 `SchemaSnapshot`：

```text
数据库 COMMENT
    ↓
现有 Local Annotation
    ↓
已审核 Wiki Knowledge Overlay
    ↓
AI Evidence Pack / Docs UI
```

任何被覆盖的来源必须保留在 provenance 中，不能静默丢弃。

### 7.3 合并优先级与冲突展示

| 内容 | AI 证据首要来源 | Database Docs 展示 | 冲突处理 |
|---|---|---|---|
| 表、字段、类型、索引、外键 | 实时 `SchemaSnapshot` | 实时结构 | Wiki 和 Annotation 只能补充说明，不能覆盖物理结构 |
| 业务含义、术语、枚举语义 | `verified` Wiki | Wiki 为主说明 | 与Local Annotation冲突时展示双方及来源，Evidence使用Wiki并记录冲突，不静默覆盖 |
| 数据库 COMMENT | 实时数据库 | 作为原始物理说明 | Local Annotation 可按现有规则覆盖显示，但必须保留 `shadowed_note` |
| Local Annotation | 本地辅助证据 | 保持现有编辑体验 | 不得静默覆盖 `verified` Wiki；冲突时标为 `local-conflict` |
| `provisional` Wiki 或 AI 推断 | 非权威候选 | 单独告警展示 | 不覆盖任何已审核说明，不参与无告警 SQL 生成 |

Overlay 结果必须为每个展示值保留 `effectiveSource`、全部 `sourceVariants` 和 `conflictStatus`。UI 的视觉主次可以变化，但不得改变上述证据优先级。

### 7.4 迁移策略

- 现有 Database Docs 注释继续可用。
- 提供“发布到当前Wiki目录”动作，把注释写入当前scope并同步Manifest/索引。
- 第一阶段不改变现有注释文件格式。
- 需要扩充 `AnnotationFile` 时另行升级 `formatVersion`，同步 Rust、TypeScript 和 fixture 测试。

## 8. db-wiki 文档规范

### 8.1 目录

```text
.memory/db-wiki/
├── SUMMARY.md
├── tables/
├── enums/
├── relationships.md
├── queries/
├── glossary/
├── parameters/
├── examples/
└── .dbx-wiki/
    └── manifest.json
```

`.dbx-wiki/manifest.json`只保存非敏感的Wiki、文档和版本信息。全文索引不写入目录。

### 8.2 Manifest

Manifest是DB-Wiki的版本化目录清单，不是全文索引、数据库连接配置或运行时缓存。它可由scope内Wiki源文件确定性生成。建议格式：

```json
{
  "manifestVersion": 1,
  "displayName": "BPM96 Database Wiki",
  "wikiSchemaVersion": 1,
  "entry": "SUMMARY.md",
  "databaseTypes": ["mariadb"],
  "documents": [
    {
      "path": "tables/ecl_request_sheet.md",
      "kind": "table",
      "object": "ecl_request_sheet",
      "sha256": "<document-sha256>"
    },
    {
      "path": "relationships.md",
      "kind": "relationships",
      "sha256": "<document-sha256>"
    }
  ]
}
```

约束：

1. 只允许scope相对路径，路径分隔符统一为`/`，条目按`path`稳定排序；`documents`不登记Manifest自身，避免自引用hash。
2. 不保存 `rootUri`、绝对路径、连接 id、数据库凭据、DSN、Token、本地索引路径、查询结果或个人数据。
3. 不保存不能确定性生成的时间戳；验证时间属于文档 Front Matter，索引构建时间属于本地 `wiki_index_state`。
4. 文档新增、删除或内容hash不一致时返回`manifest-drift`。只读扫描可以继续；Agent调用`dbx_wiki_sync_manifest`后按当前文件确定性修复Manifest并刷新索引。
5. Manifest缺失但存在`SUMMARY.md`时允许建立scope；首次写入或显式同步时自动生成Manifest并记录审计。
6. 全文 FTS、BM25 分值和结构化派生索引只保存在 `DBX_DATA_DIR`，不进入 Manifest。

### 8.3 Front Matter

```yaml
---
schema_version: 1
kind: table
object: ecl_request_sheet
database_type: mariadb
status: verified
sources:
  - type: entity
    path: src/main/java/.../RequestSheetBean.java
    symbol: RequestSheetBean
    revision: <git-revision>
  - type: database
    connection_id: <non-secret-id>
    verified_at: 2026-08-18T12:00:00+08:00
last_verified_at: 2026-08-18T12:00:00+08:00
---
```

### 8.4 表文档标准章节

1. 业务用途。
2. 物理字段。
3. 主键、索引和约束。
4. 字段业务含义。
5. 枚举、选项和参数值。
6. 表与字段关联。
7. 常用过滤条件。
8. 已验证 SQL。
9. 敏感数据分类。
10. 来源、验证时间和待确认项。

### 8.5 值域安全分类

允许写入：

- 代码枚举；
- 布尔值和静态状态码；
- 字典编码；
- 静态配置参数；
- 用户明确确认的低基数业务选项。

禁止自动写入：

- 密码、Token、连接 DSN 和密钥；
- 姓名、手机号、证件号等个人信息；
- 真实业务记录和自由文本样本；
- 未经确认的 AI 推断；
- 带真实业务主键的 SQL 示例。

## 9. 索引与检索

### 9.1 索引位置

```text
<DBX_DATA_DIR>/wiki-index/<wiki-project-id>.sqlite
```

索引是派生物，可从 Markdown 和 Manifest 完整重建。

### 9.2 db-wiki 文件探索与索引检索协同

Agent 有两条互补读取通道：

| 通道 | 用途 | 新鲜度 | 权威边界 |
|---|---|---|---|
| Wiki 目录文件工具 | 即时查看 `db-wiki` 内的 Markdown、文本、Office 和其他白名单文件 | 每次读取当前文件 | 未审核文件内容只能作为带 Citation 的来源事实或候选 |
| Wiki 索引工具 | 检索已整理、已审核的业务语义和查询案例 | 由 Manifest/hash 检测漂移 | `verified` Wiki 是业务语义首要来源 |

V1只使用DBX Native Agent loop、通用`dbx_file_*`工具和`dbx_wiki_*`语义工具，不引入CodingAgentBackend。

典型探索过程：

1. Agent 根据用户问题、Prompt 模板和已有会话上下文生成首轮搜索词、glob 或相对目录。
2. 先用list/search获取少量候选路径和命中片段，不一次读取整个目录或大文件。
3. 对高相关文件按行号或字节窗口分段读取，提取表、字段、枚举、JOIN、参数和未决问题。
4. 把每次读取结果记入 Evidence Ledger，随后由 Agent 判断继续搜索、换关键词、读取引用文件、查询 Wiki，还是验证实时 Schema。
5. 文件在两次读取之间发生变化时，以内容 hash 标记旧证据失效并重新分析，禁止把不同版本片段拼成同一已验证事实。

scope 内的白名单文件由目录工具即时读取；可沉淀为稳定知识的文件必须登记 Manifest。未登记文件可以读取，但返回 `manifest-drift` 告警，写入后必须同步 Manifest。

### 9.3 第一阶段 Wiki 检索

```text
精确标识符匹配
  + 中文/英文别名
  + SQLite FTS5/BM25
  + 表字段结构过滤
  + 关系图一至两跳扩展
  + 当前连接、数据库、可信度和新鲜度重排
```

建立独立语义单元：

- table；
- column；
- enum/value；
- parameter；
- relationship/join path；
- glossary term；
- verified query；
- warning/constraint。

不把整份长 Markdown 作为一个检索单元。

### 9.4 Evidence Pack

`build_evidence` 返回有界上下文：

```json
{
  "schemaVersion": 1,
  "scopeId": "wiki-scope-opaque-id",
  "state": "evidence-fresh",
  "question": "查询进行中的流程实例",
  "tokenBudget": 6000,
  "truncated": false,
  "facts": [
    {
      "evidenceId": "column:ecl_request_sheet.status_",
      "kind": "column",
      "subject": "ecl_request_sheet.status_",
      "value": "流程实例状态",
      "status": "verified",
      "sourceKind": "wiki",
      "verifiedAt": "2026-08-18T12:00:00+08:00",
      "citationIds": ["cite-1"],
      "runtimeValidation": {
        "status": "matched",
        "snapshotHash": "sha256:..."
      }
    }
  ],
  "joinPaths": [
    {
      "evidenceId": "join:request-sheet-to-template",
      "from": "ecl_request_sheet",
      "to": "ecl_request_sheet_template",
      "predicate": "ecl_request_sheet.req_template_id = ecl_request_sheet_template.id",
      "status": "verified",
      "citationIds": ["cite-2"]
    }
  ],
  "verifiedQueries": [],
  "warnings": [],
  "missingEvidence": [],
  "citations": [
    {
      "citationId": "cite-1",
      "sourceType": "wiki",
      "scopeId": "wiki-scope-opaque-id",
      "document": "tables/ecl_request_sheet.md",
      "anchor": "字段业务含义",
      "contentHash": "sha256:...",
      "sourceRefs": ["entity:RequestSheetBean", "database:snapshot"]
    }
  ]
}
```

契约要求：

1. `facts`、`joinPaths` 和 `verifiedQueries` 中每个元素都有稳定 `evidenceId`，并通过 `citationIds` 关联至少一个 Citation；禁止仅在顶层堆放无法回溯到事实的引用。
2. Citation 至少包含 `sourceType`、`scopeId`、目录相对路径和内容 hash；Markdown引用增加标题锚点，文本/Office文件引用增加行号、段落或单元格范围。
3. 每条事实必须包含状态、来源类别和验证时间；运行态校验必须区分 `matched`、`missing`、`conflict` 和 `not-checked`。
4. `schemaVersion`、`tokenBudget` 和 `truncated` 是必填字段；被预算截断的事实必须通过 `missingEvidence` 或 continuation contract 明示。
5. Rust、TypeScript 和 MCP `outputSchema` 共享同一规范，并通过 fixture conformance 校验。
6. 未审核文件中的事实默认状态为 `source-observed`；只有与实时 Schema 或已审核 Wiki 交叉验证后，才能升级为 `cross-validated` 或 `verified`。

### 9.5 第二阶段可选能力

向量检索仅用于业务描述与物理名称差异较大的召回，不替代：

- 精确表字段匹配；
- 外键和关系图；
- 实时 Schema 校验；
- 可信度与当前 scope 过滤。

## 10. AI Agent 编排与 SQL 生成链

### 10.1 处理链

```text
1. 解析用户问题、选中的 Prompt 模板和明确的 `db-wiki` 绝对目录
2. 调用`dbx_file_open_scope`并由`db-wiki`policy建立临时FileDirectoryScope，再建立ResearchSession、工具预算和初始Open Questions
3. Agent 调用 `dbx_wiki_list/search/read/parse/build_evidence` 或原有数据库工具
4. 把命中片段、文件 hash、行号、Wiki Citation 和推断状态写入 Evidence Ledger
5. Agent 判断证据是否充分：不足则调整关键词、路径或引用关系并返回步骤 3
6. 对候选表、字段、JOIN、枚举和参数执行实时 Schema 校验，必要时只读采样
7. 若仍缺证据、存在冲突、需要用户选择或预算耗尽，则返回结构化 nextAction
8. 证据充分且目标是 SQL 时，生成 SQL 草案并执行 AST、方言、标识符和作用域校验
9. 对只读 SQL 执行 EXPLAIN，展示 SQL、证据引用、假设和风险
10. 按 DBX 现有权限策略执行或仅返回分析结论
11. 记录脱敏轨迹；用户要求更新或沉淀知识时，Agent 在当前 scope 内使用 expected hash 自动写入并刷新 Manifest/索引
```

SQL 输出至少包含：

- 目标连接、数据库和 Wiki scope；
- 使用的表、字段和关联条件；
- 枚举/参数解释；
- Wiki 引用；
- 实时结构验证结果；
- 未确认假设；
- SQL 和风险等级。

规则：

- Wiki 没有依据的业务语义不得猜测。
- Wiki 与实时数据库在本次 SQL 使用的对象上冲突时不执行，先形成漂移报告并按任务要求更新 Wiki；无关对象的漂移只展示告警，不阻断当前 SQL。
- Agent 不得仅因为首次搜索无命中就结束；在预算内应尝试同义词、标识符、引用路径、相关 Wiki 单元或实时 Schema，直至满足停止条件。
- 查询默认设置有界行数。
- `EXPLAIN` 成功只证明物理可执行性，不证明业务语义正确。
- 写 SQL 继续使用 DBX 当前逐次确认、目标绑定和 SQL 精确匹配机制。

### 10.2 Wiki 状态与执行策略

| 状态 | 条件 | SQL 生成 | Explain/执行 |
|---|---|---|---|
| `disabled` | 提示词未提供 `db-wiki` 目录 | 保持 DBX 现有兼容流程 | 按现有 DBX 权限策略 |
| `scope-invalid` | 路径歧义、目录名/标记/规范化校验失败 | 不生成目录相关 SQL，返回稳定错误 | 禁止目录工具；数据库工具按原策略 |
| `evidence-missing` | scope 已建立，但所需业务语义无已审核证据 | 仅生成标注缺口的候选，不得把字段名推断写成事实 | 默认禁止；纯物理元数据请求可按实时 Schema 只读处理 |
| `evidence-fresh` | 所需事实有引用且运行态校验通过 | 正常生成 | 按现有 DBX 权限策略 |
| `evidence-conflict` | 当前SQL使用的Wiki事实与运行态冲突 | 生成差异说明并按任务要求更新Wiki，不生成可直接执行版本 | 禁止 |

“所有 AI SQL 包含证据”的验收范围是已成功建立 scope 的会话；`disabled` 兼容流程单独回归。目录路径存在歧义或校验失败时禁止静默选择其他目录。

### 10.3 db-wiki 内容注入边界

`db-wiki` 内的 Markdown、文本、Office内容、数据库 COMMENT、Annotation、查询案例和检索片段都作为不可信数据，而不是系统指令：

1. 解析器只抽取允许的 Front Matter、标题章节和结构化字段；原始 HTML、脚本、嵌入对象和危险链接不进入模型上下文。
2. Evidence Pack 使用结构化字段传递内容，并在提示词中置于明确的“数据引用”边界内；任何“忽略规则”“调用工具”“切换目录”等文本只作为被引用内容。
3. Wiki 内容不得改变 scope 根目录、工具白名单、数据库权限或系统提示词。
4. 已验证 SQL 只是参考证据，不因来自 Wiki 获得执行授权。
5. 检测到疑似提示词注入时保留原始 Citation，标记 `content-safety-warning` 并从可执行证据中排除；是否改写该内容由用户任务决定。

### 10.4 ResearchSession、Evidence Ledger 与停止条件

ResearchSession 是多轮对话期间的结构化研究状态，不等于原始聊天记录：

```json
{
  "sessionId": "research-session-id",
  "goal": "生成查询进行中流程实例的 SQL",
  "intent": "generate-sql",
  "iteration": 3,
  "evidenceIds": ["column:ecl_request_sheet.status_"],
  "openQuestions": ["进行中包含哪些状态值"],
  "wikiScopeId": "wiki-scope-opaque-id",
  "searchedPaths": ["tables", "queries/workflow"],
  "nextAction": "search-more",
  "budget": {
    "toolCallsUsed": 8,
    "toolCallsLimit": 30,
    "contextTokensRemaining": 5000
  }
}
```

Agent 每轮工具调用后更新 Evidence Ledger 和 Open Questions，并按以下顺序判断：

1. **证据充分**：目标表、字段、JOIN、必要枚举/参数均有 Citation，且运行态校验无阻断冲突；进入 SQL 或最终分析。
2. **需要继续探索**：存在可由授权文件、Wiki 或 Schema 工具回答的问题；生成下一次只读工具调用。
3. **需要用户输入**：目录路径、数据库、业务口径或多个同等候选无法安全自动选择；返回 `request-user-input`。
4. **证据冲突**：返回 `report-conflict`，展示双方来源并可按任务要求更新 Wiki。
5. **预算耗尽**：返回 `budget-exhausted` 和当前已知/未知清单，不伪装成完整答案。
6. **策略阻断**：返回 `stop-by-policy`，不能通过继续调用工具绕过 scope 根目录或数据库权限。

`nextAction` 至少支持 `search-more`、`read-file`、`validate-schema`、`generate-sql`、`return-analysis`、`request-user-input`、`report-conflict`、`budget-exhausted` 和 `stop-by-policy`。预算来自 DBX 设置或会话配置，不在业务代码中硬编码机器相关值。

## 11. Agent 知识工具契约

### 11.1 通用目录scope与文件工具

| 工具 | 作用 |
|---|---|
| `dbx_file_open_scope` | 校验提示词给出的绝对目录并返回临时scope id；普通目录只读，命中写白名单时按策略升级权限 |
| `dbx_file_close_scope` | 主动关闭scope；不因单次Agent请求结束自动关闭 |
| `dbx_file_list` | 按scope id、相对目录、depth和glob列出文件或子目录 |
| `dbx_file_search` | 在scope内按文本、标识符或正则搜索，返回相对路径、范围和有界片段 |
| `dbx_file_read` | 按相对路径和行号/字节窗口分段读取，返回content hash和continuation |
| `dbx_file_parse` | 选择格式适配器，把文本或Office文件解析为有界结构化片段、章节/工作表元数据和Citation |
| `dbx_file_stat` | 返回文件类型、大小、hash和修改状态 |
| `dbx_file_write` | 按scope策略创建或更新白名单文件，校验expected hash后原子写入 |

约束：

- 除 `open_scope` 外，所有工具都要求有效scope id；Ask和Agent模式均注册只读文件工具，只有Agent模式注册写入、Manifest同步和会话知识回写工具。
- 所有文件操作使用 `scopeId + relativePath`，禁止再次传绝对路径、`..`、符号链接/重解析点逃逸和跨scope glob。
- 默认忽略`.git`、构建产物、依赖缓存、非白名单二进制、大文件和敏感文件名；规则由DBX内置策略提供。
- `read`必须分段返回并带`contentHash`、范围和continuation；Agent不能要求一次读取整个目录。
- scope生命周期和依赖该scope的工具按模型调用顺序串行执行，避免`open_scope`之后的读取被提前调度。V1不提供rename、delete、shell或进程执行能力。
- 当前V1提供通用目录只读fallback，并内置`db-wiki`作为首个写白名单策略；后续其他写目录场景只扩展`DirectoryAllowlistPolicy`注册表，不新增一套文件工具协议。

### 11.2 通用文档格式适配器

格式解析使用统一扩展口，不在 Agent 工具分支中按扩展名堆叠实现：

```text
FileDocumentAdapter
├── supports(extension, mime, signature)
├── inspect(path) -> metadata
├── extract(path, cursor, budget) -> DocumentChunk
├── validateWrite(originalHash, content)
└── renderWrite(content) -> bytes
```

`DocumentChunk` 至少包含 `format`、`relativePath`、`contentHash`、`section/sheet`、`range`、`text/rows`、`warnings`、`truncated` 和 `continuation`。扩展名、MIME 和文件签名不一致时失败关闭，禁止把二进制内容直接塞入模型上下文。

#### V1 格式矩阵

| 格式 | 读取/解析 | 更新方式 | 执行阶段 |
|---|---|---|---|
| `md`、`txt`、`sql` | UTF-8/UTF-8 BOM 及受控编码检测，按行分段 | 文本 unified diff + expected hash | M1 读，M2 写 |
| `json`、`yaml/yml`、`xml` | 文本读取 + 语法/结构校验 | 文本 diff，应用前重新解析 | M1 读，M2 写 |
| `csv`、`tsv` | 流式表头/行窗口、编码和分隔符检测 | 结构化行/单元格更新或文本重写 | M1读，M2写 |
| `xlsx` | 复用`calamine`/现有Table Import，按工作表和行列窗口解析 | 不使用文本diff；结构化单元格更新，重新生成并校验工作簿 | M1读，M3写 |
| `xls` | 新增 `calamine` 只读适配，按工作表和行列窗口解析 | V1 不写，建议另存 `xlsx` | M1 读 |
| `docx` | 新增ZIP/XML只读适配，提取段落、标题、表格和关系告警 | V1不原位写；可把提取知识写入Markdown Wiki文件 | M3读 |
| `doc` | 旧二进制格式不原生解析 | V1 不写；返回 `FORMAT_CONVERSION_REQUIRED` | 后续通过显式外部转换器扩展 |

边界：

1. M1/M2 的完成不以 DOC/DOCX 原样编辑或保留 XLSX 全部样式、公式、宏为条件。
2. `xlsx` 结构化更新必须保留未修改工作表并对公式、合并单元格、隐藏工作表和宏给出保真度告警；无法保证时只生成新文件预览，不覆盖原文件。
3. `.xlsm`、密码保护 Office 文件、嵌入对象和宏默认拒绝写入。
4. `docx` 和 `xlsx` 解析结果是证据视图，不自动成为可回写的完整文档模型。
5. 文件过大、压缩比异常或 ZIP 条目越界时按 zip-bomb/资源预算策略终止解析。

### 11.3 db-wiki 语义工具

以下工具是 `policyId=db-wiki` 的语义扩展，不改变 11.1 的通用文件工具契约；后续场景可以按 policy 继续扩展各自的语义工具集。

| 工具 | 作用 |
|---|---|
| `dbx_wiki_status` | 返回当前scope的索引状态、文档数和漂移状态 |
| `dbx_wiki_search` | 搜索表、字段、枚举、参数、关系和 SQL 案例 |
| `dbx_wiki_read` | 按文档和标题读取精确片段 |
| `dbx_wiki_build_evidence` | 为自然语言问题生成 SQL 证据包 |
| `dbx_wiki_diff_schema` | 对比实时 Schema 与 Wiki，只返回差异 |

Wiki语义工具要求scope的`policyId=db-wiki`；其他policy建立的通用文件scope不能调用Wiki Evidence、Manifest和索引工具。

### 11.4 自动写入与知识沉淀工具

以下写入工具同样绑定 `db-wiki` policy；通用文件创建/更新能力仍由 `dbx_file_write` 承载，语义工具只负责基于 ResearchSession/Evidence Ledger 生成可写内容。

| 工具 | 作用 |
|---|---|
| `dbx_wiki_update_from_session` | 从ResearchSession/Evidence Ledger生成知识内容并直接写入当前scope的目标Wiki文件 |
| `dbx_wiki_sync_manifest` | 根据当前scope白名单文件确定性更新Manifest并触发索引刷新 |
| `dbx_wiki_record_feedback` | 保存脱敏的结构化纠正、结果判定和候选事实，不接收原始结果集 |

写入不需要前端审批，但必须：

1. 使用有效scope id和相对路径，写入目标始终位于规范化根目录内，并符合当前policy的accessMode和allowedExtensions。
2. 更新已有文件时携带expected hash；不匹配返回 `WIKI_FILE_CONFLICT` 并要求重新读取。
3. 新建/更新内容通过扩展名、格式、secret/PII和提示词注入检查。
4. 写入采用同目录临时文件、flush和原子替换，随后记录通用审计并执行当前policy的afterWrite hooks；`db-wiki`策略会同步Manifest并刷新索引。
5. Agent最终回答必须列出本轮自动创建/更新的相对文件及结果；部分失败不得报告整批成功。

## 12. 查询反哺闭环

### 12.1 可观测信号

- Wiki 未命中，但实时数据库存在相关对象。
- SQL 返回未知表、未知字段或类型不匹配。
- 用户纠正字段含义、枚举或关联关系。
- SQL 成功执行且用户确认业务结果正确。
- 代码枚举、DDL、数据库和 Wiki 不一致。
- 高频使用同一关联路径。
- 发现新的静态参数或低基数选项候选。

### 12.2 状态机

```text
Observed
  -> Candidate
  -> CrossValidated
  -> Applied
  -> Indexed
```

任何证据不足的候选可以进入 `Rejected` 或 `Expired`，不得静默进入 Wiki。

### 12.3 自动更新记录

```json
{
  "scopeId": "wiki-scope-opaque-id",
  "targetFile": "tables/ecl_request_sheet.md",
  "expectedHash": "...",
  "reason": "实时字段与 Wiki 不一致",
  "evidence": [],
  "confidence": "verified",
  "sensitivity": "internal",
  "contentHash": "...",
  "result": "applied"
}
```

### 12.4 更新粒度

- 表字段和索引：数据库采集形成机械更新记录并按任务要求写入。
- 枚举：代码枚举与数据库低基数结果交叉验证。
- 参数：代码定义、默认值和实际配置三方标注。
- 业务术语：必须由用户或权威文档确认。
- 查询案例：必须成功执行且由用户确认业务正确。
- 关联关系：优先真实外键，其次已验证 JOIN，再次代码引用；推断关系保持 provisional。

### 12.5 查询轨迹与反馈最小化

`dbx_wiki_record_feedback` 只允许保存：

- SQL hash，不保存完整SQL；进入查询案例的脱敏SQL由Agent写入scope内Wiki文件并记录来源。
- Evidence id、Citation id、连接/数据库的非秘密内部 id、错误类别、返回行数区间和用户判定。
- 不含原始值的结构化纠正，例如“字段含义错误”“JOIN 不成立”“结果符合预期”。

禁止保存原始查询结果、单元格值、自由文本业务样本、连接串、凭据和 PII。写入 `wiki_query_trace`、`wiki_feedback` 和Wiki文件前执行同一套secret/PII检测。轨迹默认保留30天，`retentionDays=0`可关闭，过期自动清理。

### 12.6 从会话总结自动更新 Wiki

当用户要求“总结以上会话并更新 Wiki”时，Agent 不写入完整聊天摘要，而是执行：

```text
ResearchSession
  + Evidence Ledger
  + 当前用户确认的业务口径
        ↓
Session Knowledge Candidate
        ↓
事实去重、Citation 校验、敏感信息扫描
        ↓
按目标文件拆分并校验 expected hash
        ↓
Atomic Write + Manifest 更新 + Reindex
```

Knowledge Candidate 至少包含：

- `facts`：可沉淀事实，每条引用 Evidence id 和 Citation id；
- `verifiedQueries`：仅包含已执行成功且用户确认业务正确的脱敏 SQL；
- `openQuestions`：未解决问题，写入待确认项而非权威结论；
- `rejectedInferences`：本轮被证伪或冲突的推断，防止再次被当成候选；
- `targetHints`：建议更新的相对Wiki文件和章节，最终目标由scope路径校验服务验证。

规则：

1. 原始聊天记录不是 Wiki 来源，模型总结也不是权威证据；只有 Evidence Ledger 中有有效 Citation 的事实才能进入普通内容。
2. 用户在会话中明确给出的业务口径可以作为 `user-confirmed` 来源，但必须记录确认轮次摘要和时间，不能伪装成代码或数据库验证。
3. 没有 Citation 的有价值结论只能进入“待确认项”，状态为 `provisional`。
4. 不把完整会话、模型思考过程、原始工具输出或数据库结果集写入 Wiki。
5. 一次会话可以更新多个文件，但每个文件独立执行hash、格式、安全和冲突检测；部分应用不得把整批标为成功。

## 13. 安全与治理

### 13.1 文件安全

- 根目录由`dbx_file_open_scope`从提示词路径建立，并按6.3的policy注册表校验。
- open_scope之后的工具输入只接受scope相对路径，拒绝绝对路径、`..`、符号链接逃逸和NTFS重解析点逃逸。
- 对每级现存父目录检查符号链接/重解析点并规范化；创建临时文件和原子替换前后再次验证最终父目录仍在授权 Root 内，防止检查与使用之间的路径替换。
- 仅允许白名单扩展名和 Manifest。
- 写入采用同目录唯一临时文件、flush、原子替换。
- 写入前校验expected hash，冲突时重新读取并生成新内容。

### 13.2 数据库安全

- Wiki 工具不能提升连接的 SQL 权限。
- 查询默认只读。
- 数据库写入确认与Wiki目录自动写入相互独立；Wiki scope不能提升数据库写权限。
- Wiki 中的 SQL 示例不可被直接当成已授权写操作。
- 生产保护、连接只读和数据库权限仍是最终上限。

### 13.3 内容安全

- scope内文件和Wiki内容按10.3作为不可信数据解析，疑似提示词注入不得进入可执行证据。
- Wiki文件、查询轨迹和反馈保存前执行统一的secret/PII扫描。
- 查询轨迹仅保留 SQL hash、结构化引用和脱敏摘要。
- 默认不保存完整查询结果。
- 来源不明、过期或互相冲突的事实必须展示告警。
- 工具输出必须清理内部连接详情和其他scope存在性信息。

## 14. 存储模型

建议 DBX 本地存储增加：

| 表/集合 | 用途 |
|---|---|
| `wiki_index_state` | 索引版本、文件 hash 和最后构建时间 |
| `wiki_query_trace` | 脱敏的查询证据轨迹 |
| `wiki_feedback` | 用户纠正和验证结果 |
| `wiki_scope_audit` | scope根路径hash、session、打开/关闭时间和文件写入审计，不保存项目映射 |

全文内容和FTS表按规范化根路径hash保存在派生索引中，不放进主`dbx.db`，避免主库膨胀和scope串扰。

ResearchSession只存在于当前Agent运行和会话上下文中。FileDirectoryScope保存在进程级共享registry，可跨同一聊天的多次Agent请求复用，但不做跨重启持久化；重启后Agent应根据错误提示重新open_scope。

轨迹、反馈和审计使用scope根路径hash与session关联，不使用projectId；绝对路径不写入普通查询轨迹。

## 15. UI 设计

### 15.1 无新增权限设置页

V1不增加Workspace/项目/目录授权设置页。用户在提示词或Prompt模板中直接提供`db-wiki`路径，Agent调用`open_scope`后自动读写。DBX现有AI配置页只继续管理MiniMax模型和Agent回合等设置。

### 15.2 复用 Database Docs

在现有 Database Docs 中增加：

- 当前 Wiki scope和验证状态；
- Wiki 证据来源；
- 本地注释与 Wiki 说明的差异；
- 当前scope写入和索引刷新结果；
- “同步选中表”；
- Schema Drift 视图；
- 自动更新审计列表。

### 15.3 AI 对话

- 显示当前连接、数据库和Wiki目录scope状态。
- Evidence 折叠面板。
- Coding Agent 式工具时间线：计划目标、搜索、分段读取、Schema 校验、Open Questions 和停止原因。
- 每条 SQL 的 Wiki 引用。
- 知识缺口提示。
- “总结本会话并更新 Wiki”动作，只基于结构化Evidence Ledger写入当前scope。
- 展示本轮自动创建/更新的相对文件、旧/新hash、失败原因和索引刷新状态，不要求用户审批。

## 16. 代码改造范围

### 16.1 新增核心模块

```text
crates/dbx-core/src/wiki/
├── mod.rs
├── directory.rs
├── markdown.rs
├── manifest.rs
├── index.rs
├── search.rs
├── evidence.rs
├── write.rs
├── scope.rs
├── sync.rs
├── feedback.rs
└── security.rs

crates/dbx-core/src/agent_files/
├── mod.rs
├── extension.rs
├── policy.rs
├── policy_registry.rs
├── db_wiki_policy.rs
├── file_tools.rs
├── directory_scope.rs
├── file_write.rs
├── audit.rs
└── document/
    ├── mod.rs
    ├── text.rs
    ├── structured_text.rs
    ├── delimited.rs
    ├── excel.rs
    └── docx.rs

crates/dbx-core/src/agent_knowledge/
├── mod.rs
├── research_session.rs
└── session_summary.rs
```

### 16.2 修改模块

| 文件/模块 | 改动 |
|---|---|
| `crates/dbx-core/src/docs/*` | 增加 Wiki Overlay 与 provenance，保留现有 Snapshot |
| `crates/dbx-core/src/agent_loop.rs` | 增加`AgentFunctionExtension`工具合成与分派钩子；scope由`open_scope`工具在会话内建立，继续复用现有多轮循环、并发、取消和压缩逻辑 |
| `crates/dbx-core/src/agent_tools.rs` | 保持现有数据库工具及SQL语义；不把Wiki文件实现塞入现有match分支 |
| `crates/dbx-core/src/ai.rs` | 不改变 MiniMax 请求协议；沿用现有 tool-calling 流和 reasoning details 回放 |
| `crates/dbx-mcp/src/server.rs` | 暴露统一Wiki Evidence和scope文件工具时复用相同目录校验服务 |
| `src-tauri/src/commands/ai.rs` | 不增加workspace/project参数；继续传递原始提示词和session id |
| `src-tauri/src/commands/docs.rs` | 增加Wiki搜索、自动写入和同步命令 |
| `apps/desktop/src/docs/*` | 扩展现有 Docs UI，而非新建平行浏览器 |
| `apps/desktop/src/components/docs/*` | 当前scope状态、漂移、证据和自动写入审计 |
| DBX storage/migrations | 只增加scope写入审计、轨迹、反馈和留存清理；不增加项目/Workspace映射 |

### 16.3 非侵入式 Function Call 扩展边界

V1 不是重构现有 Agent，而是在现有 Agent loop 上增加一个附加式 function-call 扩展口：

```text
AgentFunctionExtension
├── definitions(context) -> ToolDefinition[]
├── handles(toolName) -> bool
└── execute(call, scopedContext) -> ToolResult
```

现有代码允许修改的集成点严格限定为：

| 现有文件 | 允许的附加改动 |
|---|---|
| `agent_loop.rs` | 构建基础工具后追加扩展definitions；执行非基础工具时委托扩展registry |
| `lib.rs` | 注册Agent Knowledge扩展服务，不改变原AI命令输入 |
| storage | 仅增加scope文件写入审计；不保存目录映射 |

明确禁止：

- 不修改 `run_agent_loop` 的回合控制、上下文压缩、并行/串行调度、取消、重试和最终答案校验算法。
- 不改变 `agent_tools` 现有数据库工具名称、JSON Schema、返回结构、执行逻辑和 SQL 权限；禁止复制后改名形成第二套工具链。
- 不修改 MiniMax/OpenAI-compatible 请求、流式 tool-call 解析和 reasoning details 回放。
- 不修改 `execute_query`、SQL 风险分类、连接只读、生产保护和精确 SQL 确认逻辑。
- V1 不修改 Codex、Claude Code、Pi、OpenCode 等 CLI Provider 的命令构建、工作目录和事件解析。
- 不要求`db-wiki`所在目录属于代码项目，也不要求增加DBX专用源码、配置或埋点；已有Manifest属于Wiki自身规范。

行为保持要求：

1. feature flag关闭时不注册任何`dbx_file_*`和`dbx_wiki_*`扩展工具，基础工具列表和当前Agent行为必须与改造前一致。
2. 扩展初始化失败只返回 `AGENT_KNOWLEDGE_EXTENSION_UNAVAILABLE`，不得影响数据库浏览、原有 AI 工具或普通 Ask 模式。
3. 通用文件工具使用`dbx_file_*`，Wiki语义工具使用`dbx_wiki_*`；名称冲突时启动失败，不覆盖基础工具。
4. 新增能力的主体代码全部位于`agent_files/`、`agent_knowledge/`和`wiki/`；现有文件只保留上述组合与分派接线。
5. 必须用基础工具列表快照和feature-flag回归证明“未启用即零行为变化”。

### 16.4 防漂移要求

- Rust 数据结构与 TypeScript 类型继续使用 fixture conformance 验证。
- Wiki 核心逻辑只实现一次。
- MCP 与 Desktop 返回相同结构化结果。
- MCP 工具提供版本化 `outputSchema`，Evidence Pack 与 Citation 使用稳定 id 关联。
- 各`FileDocumentAdapter`使用统一fixture验证分段、编码、结构、截断、hash和错误码。
- 文档格式升级必须带版本迁移和回滚读取策略。

## 17. 分阶段实施

### M1：MiniMax 原生 Agent db-wiki scope与只读探索

- 不修改`ai_agent_stream`输入；Agent从原始提示词解析绝对目录并调用`dbx_file_open_scope`。
- 附加式`AgentFunctionExtension`、DirectoryAllowlistPolicy注册表、feature flag和`dbx_file_open_scope/close_scope/list/search/read/parse/stat`工具。
- 文本、结构化文本、CSV/TSV、XLS/XLSX 只读适配；分段、hash、continuation 和资源预算。
- 末级`db-wiki`目录名、规范化路径、重解析点和scope相对路径校验。
- Markdown/Manifest解析、FTS5和Wiki Search/Read/Evidence。
- ResearchSession、Evidence Ledger、Open Questions、nextAction、回合/工具/上下文预算和停止条件。
- scope文件行号/段落/单元格Citation、Wiki Citation和版本化`outputSchema`。
- scope文件/Wiki提示词注入隔离、敏感路径和大文件保护。
- Database Docs 按 7.3 展示 Wiki Overlay、来源变体和冲突。

M1退出条件：当前MiniMax Agent从提示词解析一个有效`db-wiki`目录，建立临时scope，并完成至少两轮list/search/read/parse，引用Markdown/文本/Excel证据给出结构化分析或明确停止原因；不发生文件和数据库写入。

### M2：SQL闭环与scope内自动更新

- 实时 Schema 对照、SQL AST、方言、标识符、作用域校验和 EXPLAIN。
- 多轮文件/Wiki 证据充分后生成 SQL；缺口、冲突和预算耗尽返回结构化 nextAction。
- `dbx_file_write`、expected hash、格式校验、原子写入和通用审计；`db-wiki`policy执行Manifest同步和索引刷新。
- `md/txt/sql/json/yaml/xml/csv/tsv`自动更新；结构化格式写入后重新解析，不需要前端审批。
- “总结以上会话”生成带Citation的知识并直接写入当前scope，不保存完整聊天或原始结果集。
- Manifest 更新、增量重建索引、有界查询轨迹和留存清理。
- UI仅展示工具时间线、Evidence Ledger和本轮自动写入结果，不增加目录选择或审批交互。

M2退出条件：MiniMax Agent完成“解析目录→读Wiki文件→校验实时Schema→生成并Explain SQL→总结会话→expected hash校验→自动原子写入→重建索引”端到端流程；文件写入无需前端确认，数据库写入仍遵守原策略。

### M3：Office 结构化能力

- DOCX ZIP/XML 只读适配，提取标题、段落、表格、关系和解析告警。
- XLSX结构化单元格更新、工作簿重建、未修改工作表保留、expected hash和保真度检查。
- 密码保护、宏、嵌入对象、异常压缩包和不可保真文件失败关闭。
- `.doc` 只返回转换要求；不在 DBX 核心实现旧二进制 Word 编辑器。

M3退出条件：DOCX可稳定提取有Citation的文本/表格证据；简单XLSX可在结构化校验后自动更新，复杂工作簿不能保真时拒绝覆盖原文件。

### M4：可选 Coding Agent 与知识增强

- 评估OpenCode/Codex`AgentExecutionBackend`、db-wiki目录scope传递、模型配置桥接和事件归一化；不作为V1 M1/M2依赖。
- Glossary、同义词、验证查询案例库、关系图、可选向量召回/rerank 和离线 Text-to-SQL 评测集。

## 18. 测试与验收

### 18.1 单元测试

- Markdown 和 Front Matter 解析。
- AgentFunctionExtension 工具合成、命名冲突、禁用降级和独立分派。
- DirectoryAllowlistPolicy注册、匹配、无匹配、多个匹配冲突、读写模式和afterWrite hook。
- 通用`dbx_file_*`工具在测试policy与`db-wiki`policy下使用同一输入/输出契约。
- db-wiki open/close/list/search/read/parse的目录提取、scope生命周期、glob、深度、编码、分段、continuation、hash变化和大文件边界。
- 文本/JSON/YAML/XML/CSV 适配器的语法、结构、截断和错误恢复。
- XLS/XLSX 工作表、行列窗口、日期/数字/公式视图和资源预算；DOCX 段落/表格/ZIP 安全在 M3 验证。
- `.doc`、加密 Office、宏和不支持格式返回稳定错误码，不回退为二进制文本。
- Manifest 确定性生成、稳定排序、hash 校验和漂移分类。
- 中文、英文、snake_case 和别名检索。
- FTS5 排序和结构过滤。
- 路径穿越、符号链接和重解析点阻断。
- expected hash 冲突。
- 绝对根路径、末级目录名、相对路径、符号链接/重解析点和scope逃逸校验。
- Evidence id/Citation id 完整性、`outputSchema` 和 Token Budget 截断。
- scope文件/Wiki提示词注入、危险嵌入和越权工具指令隔离。
- ResearchSession Evidence Ledger 去重、Open Questions、nextAction 和预算耗尽状态。
- 会话总结只能输出有 Citation 的事实，未引用内容进入 `provisional` 待确认项。
- 自动写入的expected hash冲突、格式失败、原子替换、审计和Manifest/索引刷新。
- secret/PII 检测及查询轨迹留存清理。
- 自动更新状态机和部分失败报告。
- Schema Drift 分类。
- Overlay 优先级和 `local-conflict` 展示。
- Rust/TypeScript 模型一致性。

### 18.2 集成测试

- 同一提示词存在多个`db-wiki`路径时要求用户明确，不自动选择。
- 普通绝对目录通过共享安全校验后以`policyId=generic-read-only`建立scope，可读但写入返回`FILE_SCOPE_READ_ONLY`；有效`db-wiki`目录匹配`policyId=db-wiki`并启用Wiki语义与写入工具。
- 两个并发scope中同名文件不串用证据或写入目标。
- feature flag关闭或scope未建立时，现有Ask/Agent、数据库工具和CLI Provider命令保持不变。
- 当前MiniMax配置在同一DBX会话中完成“解析目录—搜索文件—解析内容—读取关联文件—验证Schema—生成SQL”的多轮工具链。
- V1 M1/M2 不依赖安装 Codex/OpenCode，也不修改其 CLI Provider 工作目录和权限。
- Prompt模板提供目录后Agent只能在建立的scope内继续探索，文件内容不能触发切换根目录。
- 外部MCP调用相同`open_scope`和目录校验服务，不能绕过scope直接传目标绝对文件路径。
- Wiki缺字段时通过Schema工具补充，并按用户任务自动写入scope。
- SQL 错误产生候选但不修改 Wiki。
- scope内自动写入成功后刷新Manifest和索引；scope外、hash冲突和不支持格式写入失败关闭。
- 文本文件外部变化导致 expected hash 失效；普通 patch 不可作用于 XLSX/DOCX 二进制文件。
- Manifest 漂移时可只读扫描但禁止无告警 Evidence 和写入，修复后恢复。
- DBX重启后派生索引可恢复，旧scope不可恢复并须重新open_scope。
- DBX重启后旧scope不可恢复、不能继续使用。
- “总结以上会话并更新Wiki”只写入带Citation的结构化知识，不落完整聊天和原始文件内容。

### 18.3 验收指标

- 精确表名/字段名检索 Top-1 命中率 100%。
- 业务问题基准集 Top-5 表召回率不低于 90%。
- MiniMax Agent场景基准集从提示词取得`db-wiki`绝对路径，无需预先给出精确文件名，能在scope内通过至少两轮搜索/读取/解析获得充分证据并给出可追溯SQL或明确停止原因。
- M1 支持 `md/txt/sql/json/yaml/xml/csv/tsv/xls/xlsx` 读取；`.doc` 明确失败，不能误报已解析；DOCX 按 M3 验收。
- 已成功建立scope的会话中，所有AI SQL均包含事实级可追溯证据或明确的缺口告警；未提供目录按兼容流程单独验收。
- 所有 SQL 标识符均通过实时 Schema 或已验证 Wiki 校验。
- 数据库未确认时不会发生数据库写入；scope内Wiki文件允许自动写入。
- scope之间不发生Wiki、索引、轨迹和反馈串扰。
- 密码、Token、PII 和完整查询结果不会进入 Wiki。
- Wiki内容不能改变系统提示词、数据库权限和scope根目录。
- 系统只访问提示词明确指定的scope目录，不读取其父目录；普通目录和源码目录只读，只有命中写白名单的scope可修改。
- M2只在当前scope内经expected hash、格式和原子写入校验更新文件，不开发前端审批交互。
- V1 M1/M2 在未安装 OpenCode/Codex 的环境中仍可完整工作。
- V1文件工具schema不包含`wiki`专用字段；替换为测试policy后仍能完成通用目录读写，证明后续场景无需复制工具实现。
- Manifest 可由 Markdown 确定性生成，索引可由版本化 Wiki 源文件完整重建。

## 19. 兼容、回滚与风险

### 19.1 兼容

- 提示词未提供有效绝对目录或feature flag关闭时，DBX行为与当前版本一致。
- 现有 Database Docs、注释、HTML 导出和 DBML 不受影响。
- 新增 MCP 工具，不修改现有工具输入。
- Wiki文件能力按当前会话提示词建立scope，不绑定项目或连接。
- 通用只读`dbx_file_*`与Wiki只读取证工具同时提供给Ask和Agent；写入、Manifest同步和会话知识回写只提供给Agent，不新增平行Provider或替换现有Agent loop。
- V1 M1/M2 不修改各 CLI Provider；OpenCode/Codex 委托属于 M4 可选扩展。
- scope建立后按10.2失败关闭，不因检索或范围错误静默切换其他目录。

### 19.2 回滚

- 禁用Agent Files/Knowledge feature flag即移除`dbx_file_*`和`dbx_wiki_*`扩展工具，恢复原Agent工具集，无需回滚数据库查询或CLI Provider代码。
- 删除派生索引不会丢失 Wiki。
- 自动写入前记录原文件hash和备份引用。
- 功能开关关闭、显式close或DBX重启时关闭scope；单次Agent请求结束不关闭，已写入Wiki文件不回滚。

### 19.3 主要风险

| 风险 | 缓解 |
|---|---|
| Wiki 与实时库漂移 | 每次 SQL 生成前做目标表的实时校验 |
| 上下文过大 | 表/字段级检索和 Evidence Token Budget |
| 错误知识污染 | 来源、可信度、expected hash、审计和可通过版本控制回滚 |
| 多scope串扰 | 不透明scopeId、规范化根路径绑定、独立索引状态和每次工具调用路径校验 |
| UI 与 MCP 逻辑漂移 | 核心服务单一实现 |
| 敏感数据写入 | 白名单、扫描和结果不落盘 |
| 并发覆盖 | expected hash、原子保存和冲突提示 |
| 模型把任意目录伪装成Wiki | 绝对路径规范化、末级目录名、重解析点和scope校验 |
| Wiki提示词注入 | 结构化抽取、不可信数据边界、危险内容隔离和安全告警 |
| scope逃逸 | open_scope后只接受scopeId+相对路径，每次操作重新验证最终路径 |
| Agent 无限探索或成本失控 | 复用现有回合上限，并增加工具调用、上下文和停止原因预算 |
| 对现有 Agent/Provider 侵入过大 | 附加式 AgentFunctionExtension、严格集成点清单、optional context、feature flag 和未启用回归测试 |
| Office 解析资源耗尽 | 文件大小、工作表/行列、ZIP 条目、压缩比、时间和内存预算 |
| Office更新破坏格式或公式 | M1/M2只读；M3结构化更新、保真度检查、无法保证时拒绝覆盖 |
| 错把不支持格式当文本 | 扩展名/MIME/签名联合检测和稳定 `FORMAT_*` 错误码 |

## 20. 外部参考

1. MCP Roots：项目目录边界、用户授权和路径验证。
   <https://modelcontextprotocol.io/specification/2024-11-05/client/roots>
2. MCP Tools：工具契约和用户控制。
   <https://modelcontextprotocol.io/specification/2025-06-18/server/tools>
3. SQLite FTS5：本地全文检索、BM25、snippet 和 highlight。
   <https://www.sqlite.org/fts5.html>
4. RASL：把大规模 Schema 和元数据拆成可定向检索的语义单元。
   <https://arxiv.org/abs/2507.23104>
5. CSR-RAG：企业 Text-to-SQL 的上下文、结构和关系检索。
   <https://arxiv.org/abs/2601.06564>
6. OpenMetadata Data Discovery：资产、字段、术语、关系和版本发现。
   <https://docs.open-metadata.org/v1.12.x/how-to-guides/data-discovery>
7. OpenMetadata Metadata Ingestion：元数据、使用、血缘、画像和质量工作流。
   <https://docs.open-metadata.org/connectors/ingestion/workflows>
8. OpenMetadata Lineage：表级和字段级血缘。
   <https://docs.open-metadata.org/v1.12.x/how-to-guides/data-lineage>

## 21. 已决策与 V1 后续

### 21.1 已决策

- 扩展 DBX 现有 Database Docs，不另起平行文档 UI。
- Wiki按提示词指定目录维护，不硬编码机器路径，也不持久化项目映射。
- Markdown 是业务知识权威源，索引是可重建派生物。
- DBX 实时 Schema 是物理结构权威源。
- DBX 内置 Agent 和 MCP 共享 `dbx-core` Wiki 服务。
- V1 M1/M2主路径固定为当前MiniMax等API模型使用现有内置Agent loop；不依赖OpenCode/Codex。
- 通用文件能力通过附加式`AgentFunctionExtension`和`DirectoryAllowlistPolicy`注册表实现，只修改工具组合和扩展分派，不修改原AI命令输入、数据库工具契约或Agent回合算法。
- 通用文件工具不硬编码业务目录；V1提供任意安全绝对目录的只读fallback，并注册`db-wiki`读写policy，Wiki Evidence/Manifest/索引作为该policy的语义扩展和afterWrite hooks。
- `db-wiki`绝对目录从提示词解析，不引入projectId、Workspace配置、目录选择UI或持久化目录映射。
- scope校验通过后允许自动读取支持格式的文件；只有命中写白名单的scope允许自动写入且不需要前端审批，写入仍强制expected hash、格式、安全、原子替换、审计和索引刷新。
- M1 支持文本、结构化文本、CSV/TSV、XLS/XLSX 读取；M2 支持文本/Wiki安全更新；M3 支持 DOCX 读取和受限 XLSX 结构化更新；旧 `.doc` 不原生支持。
- Prompt模板可以提供`db-wiki`绝对目录，但文件内容不能改变已建立scope的根目录。
- 用户要求总结会话时，Agent从ResearchSession/Evidence Ledger生成知识并自动写入scope；原始聊天和无Citation推断不直接写入权威章节。
- 第一阶段采用精确匹配、FTS5 和关系扩展，不引入向量数据库。
- `.dbx-wiki/manifest.json`是可确定性生成的非敏感目录清单；全文索引和运行时scope状态不写入Wiki目录。
- Wiki目录白名单以末级目录名`db-wiki`和规范化真实路径为准；scope与数据库连接权限相互独立。
- Evidence Pack 使用版本化 schema，事实通过稳定 id 关联 Citation，并显式报告预算截断、缺口和运行态冲突。
- 已审核 Wiki 是业务语义首要证据；实时 Schema 是物理结构权威；Local Annotation 冲突时单独展示，不静默覆盖 Wiki。
- Wiki内容始终作为不可信数据，不能改变系统指令、数据库权限或scope根目录。
- 查询轨迹和反馈只保存脱敏结构化摘要，默认保留 30 天，不保存完整 SQL 和结果集。

### 21.2 非阻塞后续项

- Web版DBX是否允许使用服务器本地绝对`db-wiki`目录，还是只支持上传/同步后的Wiki包。
- Wiki自动写入是否可选生成Git分支或提交；V1只写当前工作树。
- 是否在 M4 实现 OpenCode/Codex `AgentExecutionBackend` 和 MiniMax 配置桥接。
- 是否通过显式外部转换器支持旧 `.doc`，以及是否增加 DOCX 原样写入能力。
