---
title: "Agent 局部文件回写与多格式安全编辑设计"
doc_id: "01-DESIGN-002"
version: "V1.1"
status: "Review"
created_date: "2026-08-19"
last_updated: "2026-08-19"
maintainer: "DBX DB-Wiki Project"
constraint_level: "Normative"
review_cycle: "On-demand"
related_docs:
  - "01-DESIGN-001"
tags:
  - "DBX"
  - "Agent"
  - "Function-Call"
  - "Extension"
  - "File-Edit"
  - "Writeback"
  - "DB-Wiki"
---

# Agent 局部文件回写与多格式安全编辑设计

## 1. 文档定位与权威边界

本文定义在不侵入 DBX 主流程的前提下，通过现有 function-call 扩展口为 Agent 增加局部文件回写能力。

本文是 `01-DESIGN-001` 的下位专项设计，遵守以下权威顺序：

1. scope、目录权限、M1-M4边界、AI SQL证据和数据库权限以 `01-DESIGN-001` 为准。
2. 本文只定义新增局部编辑function tool的扩展实现，不重定义现有工具。
3. 本文与上位设计冲突时无条件以上位设计为准。
4. 本文状态为 `Review`；评审通过后才能升级为 `Active`。

本文的目标不是重构DBX文件系统，而是在已有Agent文件扩展内增加一个可关闭、可回滚、零主流程行为变化的局部编辑工具。

## 2. V1.1相对V1.0的主要变化

V1.1针对评审问题和主版本稳定要求做出以下调整：

- 从“重构统一FileMutationEngine”收敛为“AgentFileService内的附加function-call工具”。
- 不修改 `agent_loop.rs`、Provider、数据库工具、Storage、Tauri和Desktop。
- 不修改现有 `dbx_file_write/read/stat/parse` 的名称、输入和输出契约。
- 不要求现存 `dbx_file_write` 新增 `full_replace` 参数。
- 新工具默认开启，用户本地无需额外配置即可编辑；仍保留显式关闭能力用于主版本回退。
- `expected_hash`明确取自现有 `dbx_file_stat`，绕开当前read对BOM文件的hash语义差异。
- 不宣称任意外部进程下的强CAS；只定义DBX扩展内可证明的并发边界。
- 不要求修改FunctionExtension执行上下文，因此不虚构session级审计。
- 使用scope内的内部mutation receipt和backup reference满足可追踪与回滚引用，不新增数据库表。
- hook失败采用目标hash核对和Manifest确定性重建，不修改现有policy接口。
- JSON/YAML/XML/CSV/TSV在本版本仍使用文本局部补丁＋完整格式验证，不增加多套结构化工具。
- XLS/XLSX/DOC/DOCX保持只读；Office写回另行设计，不进入本版本。

### 2.1 当前源码状态（As-Is）

本文是待实现设计，不代表局部编辑已经落地。当前源码中：

- 没有注册 `dbx_file_edit`；
- 没有 `DBX_AGENT_FILE_EDIT_TOOLS` 开关；
- `dbx_file_write`仍要求模型提交完整最终 `content`；
- 服务校验expected hash和格式后，把完整content写入临时文件并rename覆盖目标；
- `dbx_wiki_update_from_session`仍复用相同完整写入路径。

因此在V1.1实现完成之前，Agent所谓“修改文件”仍是完整内容替换，不是局部patch。

## 3. 当前可用扩展口

### 3.1 Function Call扩展

当前Agent工具链已经提供：

```text
Agent Loop
  -> AgentFunctionRegistry
  -> AgentFunctionExtension
  -> AgentFileService.definitions/handles/execute
  -> ToolCall / ToolResult
```

该口子能够：

- 追加新的function tool定义；
- 按工具名分派执行；
- 复用现有Agent/Ask模式对read-only属性的过滤；
- 继续复用现有Provider的OpenAI-compatible function calling；
- 在 `DBX_AGENT_FILE_TOOLS` 关闭时整体退出。

### 3.2 AgentFileService scope

`AgentFileService`已经持有：

- `PolicyRegistry`；
- 进程内scope registry；
- `scope_id -> FileDirectoryScope`解析；
- `scope.resolve_relative`路径约束；
- 文件stat/read/parse/write和Wiki Manifest同步。

新增局部编辑必须作为 `AgentFileService` 的内部扩展执行，禁止另建scope registry。

### 3.3 DirectoryAllowlistPolicy

现有policy已经决定：

- scope是read-only还是read-write；
- 允许读写的扩展名；
- 写后执行的afterWrite hook。

局部编辑复用现有 `access_mode`、`allows_extension` 和 `after_write`，V1.1不修改trait，也不增加新的目录写权限。

### 3.4 可复用确定性能力

本版本允许复用：

- `file_sha256`：原始文件字节hash；
- `validate_text_format`：结构化文本完整格式校验；
- `atomic_write`：同目录临时文件、flush和replace；
- `sync_db_wiki_manifest`：Manifest确定性重建；
- `ToolCall.id`：本次function-call关联标识。

其中private helper只允许做 `pub(super)` 可见性调整，不改变现有实现语义和调用结果。

### 3.5 当前不存在的扩展上下文

当前 `AgentFunctionExtension.execute` 只接收 `ToolCall`，没有session id、用户消息授权证明、Provider名称或持久化审计服务。

V1.1因此明确：

- 不修改Agent Loop来注入这些上下文；
- 不把scope id或tool call id伪装成session id；
- 不宣称完成上位设计中的持久session审计；
- 只输出和保存局部mutation receipt；
- 如未来DBX主版本提供稳定的ExecutionContext扩展，再做独立兼容升级。

## 4. 主版本稳定约束

### 4.1 禁止修改的主流程

本版本禁止修改：

```text
crates/dbx-core/src/agent_loop.rs
crates/dbx-core/src/agent_events.rs
crates/dbx-core/src/agent_tools.rs
crates/dbx-core/src/ai.rs
Provider请求、流式解析和reasoning replay
数据库SQL权限与执行逻辑
crates/dbx-core/src/storage.rs及数据库迁移
src-tauri与apps/desktop
crates/dbx-mcp对外协议
现有tool的输入、输出和错误语义
```

### 4.2 允许修改的扩展面

允许变更严格限定为：

```text
crates/dbx-core/src/agent_files/
├── local_edit/                  # 全部新增主体代码
│   ├── mod.rs
│   ├── contract.rs
│   ├── feature_gate.rs
│   ├── snapshot.rs
│   ├── matcher.rs
│   ├── fidelity.rs
│   ├── safety.rs
│   ├── receipt.rs
│   └── writer.rs
├── mod.rs                       # 仅注册新增模块
├── tool_catalog.rs              # 仅在feature开启时追加新tool
├── file_tools.rs                # 仅增加新增tool分派
└── file_write.rs                # 最多调整helper为pub(super)
```

测试只允许增加或修改：

```text
crates/dbx-core/src/agent_files/mod.rs
crates/dbx-core/src/agent_files/local_edit/* tests
```

如实现需要越过上述边界，必须停止并先修订设计，不得以“顺便重构”为由扩大范围。

### 4.3 零影响开关

新增扩展使用独立开关：

```text
DBX_AGENT_FILE_EDIT_TOOLS=0|false|off  -> 不注册局部编辑tool
环境变量缺失、为空或其他值            -> 注册局部编辑tool（默认）
```

规则：

1. 默认开启：环境变量缺失、为空或不是显式关闭值时注册局部编辑tool。
2. `DBX_AGENT_FILE_TOOLS`关闭时，子开关无效。
3. 只有 `0|false|off`（忽略大小写和首尾空白）会关闭局部编辑tool。
4. 关闭状态下不得改变现有tool列表、描述、系统提示词和执行行为。
5. 默认开启只增加function tool及其条件化description，不修改Agent Loop和现有tool Schema。
6. feature gate解析放在 `agent_files/local_edit/feature_gate.rs`。
7. 单元测试通过显式构造选项覆盖开启/关闭状态，不依赖并发不安全的进程环境变量修改。

## 5. 目标与非目标

### 5.1 目标

1. 通过新增function call让模型提交局部差量，而不是完整文件。
2. 复用现有scope、policy、path、format和Manifest能力。
3. 保护BOM、换行、Unicode和末尾换行。
4. 使用原始字节hash和唯一旧内容锚点双重校验。
5. 关闭feature后主版本行为完全不变。
6. 写入失败或响应丢失时，可通过mutation fingerprint和receipt识别是否已提交。
7. 为正文写入保存有界backup reference，不新增DBX Storage表。
8. 精确报告当前能保证和不能保证的并发边界。

### 5.2 非目标

- 不改变现有 `dbx_file_write`。
- 不改变 `dbx_wiki_update_from_session`。
- 不改变 `dbx_file_read` 的hash或返回结构。
- 不开发JSON Pointer、XPath、YAML Path或CSV主键编辑工具。
- 不写XLS、XLSX、DOC、DOCX。
- 不实现任意跨进程写入的强CAS。
- 不增加数据库持久化审计和session上下文。
- 不改造MCP、Codex或OpenCode Provider。
- 不开放文件删除、移动、重命名和scope外写入。

## 6. 总体方案

```text
Model function_call: dbx_file_edit
        │
        ▼
AgentFileService dispatch
        │
        ▼
local_edit extension
  ├── resolve existing scope
  ├── reuse policy and extension allowlist
  ├── load raw snapshot
  ├── verify raw expected_hash
  ├── match exact anchors
  ├── apply bounded non-overlapping edits
  ├── preserve BOM/EOL/EOF newline
  ├── validate final text format
  ├── scan newly introduced sensitive content
  ├── create backup reference
  ├── recheck raw target hash
  ├── call existing atomic write helper
  ├── run existing afterWrite
  ├── reconcile db-wiki Manifest when needed
  └── persist bounded mutation receipt
```

模型协议是局部编辑；文件系统提交仍然是宿主在内存中生成最终字节后执行原子替换。

## 7. 新增function tool契约

### 7.1 工具名称

```text
dbx_file_edit
```

该名称是新增契约，不与现有tool重名。输入契约版本固定为V1；未来发生破坏性变化时新增 `dbx_file_edit_v2`，不原位改变V1。

### 7.2 调用前置流程

Agent必须按以下顺序调用：

```text
dbx_file_open_scope
  -> dbx_file_read/search取得旧内容锚点
  -> dbx_file_stat取得原始字节contentHash
  -> dbx_file_edit
```

重要约束：

- `expected_hash`必须使用 `dbx_file_stat.contentHash`。
- 不使用 `dbx_file_read.contentHash`作为编辑锁，因为当前read对UTF-8 BOM文件会剥离BOM后计算hash。
- `old_text`必须来自当前请求中的read/search结果，不得凭记忆生成。

### 7.3 输入Schema

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
    "edits": {
      "type": "array",
      "minItems": 1,
      "maxItems": 20,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "properties": {
          "op": {
            "type": "string",
            "enum": ["replace", "delete", "insert_before", "insert_after"]
          },
          "old_text": { "type": "string", "minLength": 1, "maxLength": 65536 },
          "new_text": { "type": "string", "maxLength": 262144 },
          "near_line": { "type": "integer", "minimum": 1 }
        },
        "required": ["op", "old_text"]
      }
    }
  },
  "required": ["scope_id", "path", "expected_hash", "edits"]
}
```

条件规则由执行器二次校验：

- `replace/insert_before/insert_after`要求 `new_text` 存在；允许空字符串只用于replace为delete等价场景时返回明确告警。
- `delete`禁止非空 `new_text`。
- V1不支持 `replace_all`。
- V1每个锚点必须在完整底稿中唯一。

### 7.4 成功输出

```json
{
  "schemaVersion": 1,
  "toolCallId": "provider-tool-call-id",
  "mutationFingerprint": "sha256:...",
  "status": "applied",
  "writeApplied": true,
  "path": "tables/ecl_task.md",
  "previousHash": "...",
  "contentHash": "...",
  "backupRef": ".dbx-wiki/.backups/<fingerprint>.bak",
  "changedRanges": [
    {
      "operation": "replace",
      "oldStartLine": 42,
      "oldEndLine": 42,
      "newStartLine": 42,
      "newEndLine": 42
    }
  ],
  "additions": 1,
  "deletions": 1,
  "manifestStatus": "succeeded",
  "receiptRef": ".dbx-wiki/.mutations/<fingerprint>.receipt"
}
```

### 7.5 状态枚举

| status | `is_error` | 含义 |
|---|---:|---|
| `applied` | false | 正文和Manifest处理成功 |
| `no_change` | false | 结果与当前内容一致，未写入 |
| `replayed` | false | 相同fingerprint此前已成功，返回既有receipt |
| `applied_manifest_reconciled` | false | 正文成功，首次hook状态异常，Manifest重建后成功 |
| `applied_receipt_failed` | true | 正文和Manifest已完成，但receipt持久化失败 |
| `applied_hook_unknown` | true | 正文hash已命中目标，但非db-wiki hook状态无法确认 |
| `conflict` | true | raw hash不匹配 |
| `rejected` | true | 锚点、格式、安全或资源预算失败 |

所有错误结果仍返回JSON内容和 `schemaVersion=1`。为了不改变现有通用错误包装，`dbx_file_edit`在 `AgentFileService.execute` 中走一个局部直接返回 `ToolResult` 的分派分支；其他tool继续使用现有包装逻辑。

## 8. 编辑匹配与应用规则

### 8.1 权威选择器

```text
文件级：expected_hash
局部级：old_text唯一匹配
辅助信息：near_line
```

`near_line`不参与授权，只用于：

- 错误消息中报告最近候选；
- 在唯一匹配失败时提示Agent扩大上下文；
- changed ranges展示。

### 8.2 匹配规则

1. 对原始底稿一次性解析全部锚点。
2. 每个 `old_text` 必须全文件唯一。
3. 零匹配返回 `FILE_EDIT_ANCHOR_NOT_FOUND`。
4. 多匹配返回 `FILE_EDIT_ANCHOR_AMBIGUOUS`。
5. 不进行Levenshtein、缩进模糊匹配或自动猜测。
6. 多个编辑范围不得重叠。
7. 两个insert不能使用同一字节位置。
8. 范围确认后按起始字节偏移倒序应用。
9. 新内容与底稿完全相同时返回 `no_change`，不写backup、正文、hook和receipt。

### 8.3 换行匹配

- 统一LF文件：输入 `\n`按LF匹配和写入。
- 统一CRLF文件：输入 `\n`在扩展内转换为CRLF后匹配和写入。
- 不允许输入裸 `\r`。
- 混合换行文件只允许不含换行的单行锚点和单行替换。
- 混合换行文件的多行编辑返回 `FILE_EDIT_MIXED_EOL_MULTILINE_UNSUPPORTED`。

该限制优先保护非目标区域，不通过全文件格式化“修复”混合换行。

## 9. Raw Snapshot与hash语义

### 9.1 原始字节hash

`dbx_file_edit.expected_hash`定义为：

```text
SHA-256(目标文件完整原始字节)
```

它包含：

- UTF-8 BOM；
- 原始CRLF/LF；
- 所有末尾换行和空字节；
- 实际Unicode编码字节。

这与 `dbx_file_stat.contentHash` 和 `file_sha256` 一致。

### 9.2 read hash兼容边界

V1.1不修正也不改变现有 `dbx_file_read.contentHash`，避免破坏当前调用方。

工具描述必须明确要求编辑锁来自stat。后续主版本若统一read/stat hash，应通过独立兼容变更处理，不夹带在局部编辑扩展中。

### 9.3 Snapshot内容

扩展内部Snapshot包含：

```text
raw bytes
raw SHA-256
UTF-8 BOM present
decoded text without BOM
EOL kind: lf | crlf | mixed | none
endsWithNewline
file length
best-effort modified timestamp
```

V1.1只支持UTF-8和UTF-8 BOM。其他编码返回 `FILE_ENCODING_WRITE_UNSUPPORTED`。

## 10. 文本保真

1. 无BOM文件保持无BOM。
2. UTF-8 BOM文件写回时恢复原BOM。
3. LF/CRLF文件保持原主换行。
4. 混合换行只允许单行编辑，未修改区域保持原始字节。
5. 不执行NFC/NFD归一化。
6. 不自动格式化完整文件。
7. 不改变文件末尾换行，除非编辑锚点覆盖文件末尾。
8. changed ranges按行号报告；内部仍使用字节范围应用编辑。

“未修改区域保持原始字节”定义为：从原底稿扣除全部旧编辑范围、从新内容扣除对应新范围后，剩余前缀和间隔字节逐段相同。不能使用全文件hash比较这一性质。

## 11. 格式处理

### 11.1 本版本支持

| 格式 | 编辑方式 | 最终校验 | 额外限制 |
|---|---|---|---|
| Markdown | 精确文本锚点 | Front Matter可选校验、基础fence检查 | 不做整文AST重排 |
| TXT | 精确文本锚点 | UTF-8与资源预算 | 无 |
| SQL | 精确文本锚点 | 现有内容安全规则＋基础检查 | 不执行SQL、不改变数据库权限 |
| JSON | 精确文本锚点 | 完整JSON解析 | 不支持JSONC |
| YAML/YML | 精确文本锚点 | 完整YAML解析 | 不做对象反序列化后重写 |
| XML | 精确文本锚点 | 完整XML解析 | 不重排namespace、属性和空白 |
| CSV/TSV | 精确文本锚点 | 完整CSV Reader解析 | 多行字段必须用完整唯一上下文 |

### 11.2 本版本不写

| 格式 | 行为 |
|---|---|
| XLS | 保持只读 |
| XLSX | 保持只读；M3另立扩展设计 |
| DOC | 返回转换要求 |
| DOCX | 保持只读取证；写回另立扩展设计 |
| 其他二进制 | 拒绝 |

### 11.3 不增加结构化tool的理由

JSON Pointer、YAML Path、XPath、CSV主键编辑和XLSX工作簿修改会显著增加tool数量、Schema复杂度和格式保真风险。V1.1优先验证局部编辑function-call本身；结构化工具必须在独立评审中按格式逐个增加，不能作为当前改造的附带范围。

## 12. 格式与安全校验

### 12.1 复用格式校验

扩展调用现有 `validate_text_format` 校验最终decoded text。允许做的唯一改动是把该helper调整为 `pub(super)`；不改变现有 `dbx_file_write` 的调用顺序和错误码。

### 12.2 新增内容安全

当前仓库没有完整的Wiki secret/PII写入扫描实现。V1.1在扩展目录内增加最小 `safety.rs`，采用baseline-aware规则：

```text
scan(old text)
scan(new final text)
new findings = new scan - old scan
```

规则：

- 新引入私钥块、明确Token、带明文凭据的连接串：拒绝。
- 新引入疑似手机号、证件号、邮箱等PII：返回告警并拒绝自动权威写入。
- 原文件已有finding但本次没有新增：允许修改无关区域，同时在结果中返回existingFindingWarning。
- 扫描器不得把完整匹配内容写入错误和receipt，只记录类别与数量。
- 扫描规则和测试fixture全部放在local_edit扩展目录。

该最小扫描器只保护本新增tool，不宣称补齐整个DB-Wiki产品的全局安全闭环。

## 13. 写入、并发与保证边界

### 13.1 提交流程

```text
1. 从AgentFileService取得现有scope
2. 复用scope.resolve_relative解析目标
3. 验证scope为read-write且扩展名允许写
4. 读取raw snapshot
5. 验证expected_hash
6. 解析并应用全部edits
7. 执行文本保真、格式和新增敏感内容校验
8. 计算最终raw bytes与目标hash
9. 查询相同mutation fingerprint receipt
10. 创建有界backup reference
11. 替换前再次计算目标raw hash
12. hash仍一致时调用现有atomic_write helper
13. 核对最终目标hash
14. 调用现有after_write
15. 必要时确定性重建Manifest
16. 写mutation receipt并返回结果
```

步骤10、16写入的是db-wiki policy拥有的内部控制文件，不是模型可以指定路径的通用写入目标。它们只能位于固定的 `.dbx-wiki/.backups` 和 `.dbx-wiki/.mutations` 子目录，文件名只能由服务端fingerprint生成；模型输入不能覆盖内部路径、文件名、扩展名或内容结构。这与Manifest一样属于policy内部副作用，不扩大用户可寻址的写扩展名白名单。

### 13.2 扩展内并发

`local_edit`维护规范化目标路径级异步锁，仅串行化 `dbx_file_edit` 调用。

保证：

- 两个并发 `dbx_file_edit` 不会同时基于同一底稿成功提交。
- 后到调用会在锁内重新读取并验证hash。
- 不同目标文件可以并行。

不保证：

- 旧 `dbx_file_write` 会参与该锁；
- 任意外部编辑器会遵守该锁；
- 最后一次hash复核与rename之间不存在理论竞态。

因此验收不得使用“任意外部编辑永不覆盖”的绝对表述。

### 13.3 外部变化处理

- 在最后复核前可观察到的变化返回 `conflict`。
- replace后最终hash不等于预期时返回 `FILE_EDIT_FINAL_HASH_MISMATCH` 并保留backup reference。
- 残余微小竞态作为平台限制记录；不把同目录rename描述为CAS。
- 如未来主版本提供统一ConditionalWrite口子，扩展可以切换实现，但不得提前依赖。

## 14. Mutation fingerprint、receipt与重试

### 14.1 fingerprint

```text
SHA-256(
  policy_id
  + normalized relative path
  + expected_hash
  + canonical JSON edits
)
```

fingerprint不包含绝对路径，也不把原始内容直接暴露在文件名中。

### 14.2 receipt

receipt写入：

```text
<scope>/.dbx-wiki/.mutations/<fingerprint>.receipt
```

内容包括：

- schemaVersion；
- toolCallId；
- mutationFingerprint；
- relative path；
- previous/content hash；
- changed ranges和增删统计；
- backupRef；
- Manifest状态；
- 结果状态和写入时间。

不包括：

- 完整旧内容、新内容或diff；
- 绝对路径；
- session id；
- secret/PII匹配原文。

`.receipt`不属于文件读写白名单，现有search/parse不会把它当Wiki证据；Manifest生成继续跳过 `.dbx-wiki`。

### 14.3 replay

收到相同fingerprint时：

1. 读取receipt；
2. 核对当前目标hash是否等于receipt.contentHash；
3. 一致则返回 `replayed`，不重复写正文；
4. receipt显示Manifest未完成时，只执行Manifest重建；
5. 当前目标hash不同则返回冲突，不复用旧结果。

receipt不存在但同fingerprint backup存在时：

1. 校验backup raw hash等于调用中的expected_hash；
2. 在backup底稿上重新应用同一组edits，仅计算预期最终hash；
3. 当前目标hash等于预期最终hash时，判定正文此前已提交，补写receipt并返回 `replayed`；
4. 当前目标hash既不等于expected_hash也不等于预期最终hash时返回冲突；
5. 不允许仅凭backup存在就覆盖当前目标。

该机制覆盖function-call结果丢失后的重复调用，不依赖session上下文。

## 15. Backup reference与回滚材料

### 15.1 backup

目标存在且编辑会产生变化时，提交前把原始raw bytes写入：

```text
<scope>/.dbx-wiki/.backups/<fingerprint>.bak
```

要求：

- 同目录内部写入，继承scope文件权限；
- 写入成功后才允许替换正文；
- backup hash必须等于expected_hash；
- `.bak`不属于Agent读取和搜索白名单；
- receipt只保存相对backupRef和hash；
- 本版本不新增自动restore tool，防止扩大写能力。
- 创建内部目录前逐级执行与scope相同的symlink/reparse-point检查；内部路径发生逃逸或被替换时正文不得写入。

### 15.2 retention

- 每个目标最多保留20个backup。
- 默认保留7天。
- 清理只能删除 `.dbx-wiki/.backups` 下由合法fingerprint命名的 `.bak`。
- 清理失败不阻断当前正文提交，但写入receipt告警。
- 不删除没有对应receipt或hash无法核对的文件。

该方案不依赖Git和DBX Storage，同时满足上位设计的原文件hash与backup reference要求。

## 16. afterWrite与Manifest协调

V1.1不修改 `DirectoryAllowlistPolicy::after_write` 返回类型。

处理方式：

1. `atomic_write`成功后计算目标hash。
2. 调用现有 `after_write`。
3. 成功则记录 `manifestStatus=succeeded`。
4. 失败时重新读取目标；若hash等于预期，确认正文已提交。
5. policy为 `db-wiki` 时直接调用现有 `sync_db_wiki_manifest` 做确定性协调。
6. 协调成功返回 `applied_manifest_reconciled`。
7. 非db-wiki policy或协调仍失败，返回 `applied_hook_unknown`，receipt明确 `writeApplied=true`。

receipt自身写入失败时返回 `applied_receipt_failed`，同时携带 `writeApplied=true`、最终正文hash和backupRef；下一次相同调用按14.3使用backup重建receipt，禁止重复应用正文。

本版本不声称知道多个通用hook中的逐项成功状态，也不新增hook重试接口。

## 17. 最小代码改造清单

| 文件 | 允许改动 | 主版本影响 |
|---|---|---|
| `agent_files/mod.rs` | 声明 `local_edit` 模块 | 无运行语义变化 |
| `agent_files/tool_catalog.rs` | feature开启时追加 `dbx_file_edit`定义 | 关闭时输出必须字节级等价 |
| `agent_files/file_tools.rs` | 新增tool的局部分派与scope复用 | 不改现有分支 |
| `agent_files/file_write.rs` | helper改为 `pub(super)` | 不改现有调用和结果 |
| `agent_files/local_edit/*` | 新增全部实现 | 独立扩展目录 |
| `agent_files/mod.rs` tests | 增加开关关闭/开启快照 | 只测试扩展 |

明确不增加新crate、不改workspace成员、不改主版本依赖图。若新增diff库或格式库，必须先证明当前依赖无法满足并单独评审；V1.1默认使用现有依赖和小型确定性实现。

## 18. Agent工具选择策略

### 18.1 feature关闭

- 仅当 `DBX_AGENT_FILE_EDIT_TOOLS=0|false|off`，或父开关 `DBX_AGENT_FILE_TOOLS`关闭时进入该状态。
- 不暴露 `dbx_file_edit`。
- 现有系统提示词和tool description完全不变。
- 现有Agent仍按当前逻辑使用 `dbx_file_write`。

### 18.2 feature开启

- 这是环境变量缺失时的默认状态。
- 新增tool description明确：现存文本文件优先使用edit。
- `dbx_file_write`名称、Schema和实现不变。
- 允许在extension tool catalog中仅调整write描述，提示“新建或显式完整内容写入”；该描述变化只在子feature开启时发生。
- 不修改 `augment_system_prompt_with_file_tools`。

主版本稳定优先意味着旧 `dbx_file_write` 仍然可被模型调用，本扩展不能强制禁止完整回写。V1.1只通过新增tool及条件化description改善工具选择；是否进一步限制旧write属于上位工具契约变更，不在本文授权范围内。

### 18.3 推荐调用提示

```text
For an existing text file, read the target context, call dbx_file_stat for the
raw-byte contentHash, and prefer dbx_file_edit with exact unique old_text.
Use dbx_file_write only when the task intentionally supplies the complete file.
```

该提示放入function tool description，不注入Agent Loop系统提示。

## 19. 测试设计

### 19.1 零影响回归

- 子feature默认开启，默认工具快照包含新增 `dbx_file_edit`。
- 显式关闭时tool名称、顺序、描述和read-only标记与改造前快照一致。
- `DBX_AGENT_FILE_TOOLS`关闭时不暴露edit。
- 不修改 `agent_loop` 当前27项回归基线。
- 现有5项文件工具基线继续通过。

### 19.2 function contract

- JSON Schema包含 `additionalProperties=false`。
- 四种操作的条件字段校验。
- 成功、no_change、replayed和错误均返回 `schemaVersion=1`。
- ToolCall.id原样进入toolCallId。
- 破坏性V2不能替换V1工具契约。

### 19.3 hash与BOM

- `dbx_file_stat.contentHash`可用于无BOM文件edit。
- `dbx_file_stat.contentHash`可用于UTF-8 BOM文件edit。
- `dbx_file_read.contentHash`的当前兼容行为保持不变。
- backup hash、previousHash和raw expected_hash一致。
- 最终contentHash包含BOM和真实换行字节。

### 19.4 匹配与保真

- replace/delete/insert_before/insert_after。
- 锚点零匹配、多匹配和重叠。
- 两个insert同位置拒绝。
- LF、CRLF、无末尾换行和UTF-8 BOM保留。
- 混合换行单行编辑成功，多行编辑拒绝。
- 中文、emoji、组合字符和长行。
- 未修改前缀、间隔和后缀字节逐段一致。

### 19.5 格式

- Markdown Front Matter和fence。
- JSON最终解析失败时原文件不变。
- YAML注释与anchor未修改区不变。
- XML namespace、注释和实体未修改区不变。
- CSV/TSV字段内换行使用完整锚点。
- SQL文件写入不触发数据库执行。
- XLS/XLSX/DOC/DOCX稳定拒绝。

### 19.6 safety

- 新增私钥、Token和带凭据DSN拒绝。
- 原文件已有finding、编辑无关位置时允许并告警。
- 新增PII拒绝自动权威写入。
- 错误和receipt不包含匹配原文。

### 19.7 replay、backup与Manifest

- 相同fingerprint重复调用返回replayed。
- receipt存在但目标hash不同返回冲突。
- backup失败时正文不变。
- backup hash等于previousHash。
- afterWrite失败但正文hash正确时执行Manifest重建。
- Manifest重建成功返回applied_manifest_reconciled。
- hook未知时明确writeApplied=true。
- receipt写入失败后，相同调用可根据backup和目标hash恢复为replayed。
- retention只清理合法目录、后缀和fingerprint文件。

### 19.8 并发边界

- 两个同路径edit基于同hash时最多一个成功。
- 不同路径edit可以并行。
- 最终复核前的外部修改产生冲突。
- 记录复核后到rename之间的残余竞态，不设置不可证明的测试断言。

## 20. 分阶段交付

### E0：扩展骨架与零影响证明

- 新增local_edit模块和显式构造选项。
- 完成交付后的默认配置为开启；E0未完成执行器前不得向正式tool catalog注册空壳工具。
- 显式关闭状态tool快照与改造前完全一致。
- 不实现写入。

退出条件：compile、现有文件工具测试和Agent Loop focused tests通过。

### E1：文本edit核心

- 实现Snapshot、唯一锚点、四种操作、格式验证和BOM/EOL保真。
- 使用stat raw hash。
- 支持Markdown/TXT/SQL/JSON/YAML/XML/CSV/TSV。

退出条件：所有编辑、hash和格式fixture通过；只有此时才允许把默认开启行为接入正式tool catalog。

### E2：安全、backup与receipt

- 增加baseline-aware safety。
- 增加backup、fingerprint、receipt和replay。
- 增加Manifest协调状态。

退出条件：响应丢失重放、部分成功和回滚引用测试通过。

### E3：本地灰度

- 使用默认开启配置完成本地真实文件编辑验证。
- 同时验证显式关闭可以立即恢复旧工具集。
- 观察Agent选择edit/write、冲突、歧义和格式失败。
- 未通过本地验收前不发布构建产物。

退出条件：无主版本回归，未修改区域异常数为0。

### E4：发布与Active评审

默认开启是本文已决策行为，但只有满足全部验收、显式关闭回退有效并经过独立评审后，文档才能从Review升级为Active并发布对应构建。

## 21. 验收标准

1. 子feature默认开启并暴露新增tool；显式关闭时现有工具快照和行为完全不变。
2. 未修改Agent Loop、Provider、数据库工具、Storage、Tauri、Desktop和MCP。
3. 现存文本文件可通过新增function call提交局部差量。
4. expected_hash来自stat原始字节hash，BOM文件可正常编辑。
5. 唯一锚点和完整hash同时生效；不提供模糊匹配。
6. LF、CRLF、BOM、Unicode和末尾换行满足保真测试。
7. JSON/YAML/XML/CSV/TSV最终完整解析失败时正文不变。
8. 新引入明确secret/PII被扩展安全规则拒绝。
9. 相同fingerprint重放不会重复修改正文。
10. 每次实际变更生成backupRef和receiptRef。
11. 正文已写但Manifest首次失败时可确定性协调，并明确writeApplied。
12. 两个并发edit不会基于同一hash同时成功。
13. 不宣称阻断所有非协作外部写入；残余竞态在结果与发布说明中可见。
14. XLS/XLSX/DOC/DOCX保持只读。
15. 本版本不要求session审计，不修改主流程来获取session上下文。

## 22. 回滚与清理

### 22.1 功能回滚

设置子feature为关闭即可移除tool。关闭后：

- 不再向模型暴露edit；
- 不影响现有read/write/Wiki tools；
- 已写正文不自动回滚；
- backup和receipt按retention继续清理或由运维手动保留。

### 22.2 代码回滚

代码回滚只需移除：

- `agent_files/local_edit/`；
- `mod.rs`模块声明；
- tool catalog新增项；
- file_tools新增分支；
- helper可见性调整。

不需要回滚DB migration、Agent Loop、Provider或前端。

### 22.3 数据恢复边界

本版本生成backup reference但不提供Agent自动restore。需要恢复时必须由受信任运维流程核对：

- backup hash等于receipt.previousHash；
- 当前目标文件和目标恢复版本；
- scope与相对路径；
- 恢复后的Manifest同步。

## 23. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 新tool默认开启后改变模型选择 | 保持纯增量tool；用focused Agent回归验证，显式关闭可立即恢复旧工具集 |
| 模型继续选择旧write | 保持兼容并记录为已知边界；不宣称强制局部编辑 |
| read/stat hash语义不一致 | 明确只使用stat raw hash；不修改旧read契约 |
| 模糊匹配误改 | V1仅精确唯一匹配 |
| 混合换行被重排 | 多行编辑失败关闭 |
| 外部非协作写入竞态 | 最终复核＋明确残余边界，不承诺强CAS |
| 现有write与edit不共享锁 | 仅保证edit内部串行；灰度指标单独记录 |
| secret扫描误报 | baseline-aware，只阻断新增finding，fixture验证 |
| backup扩大敏感数据留存 | 不进入搜索/Manifest，短期有界保留，继承目录权限 |
| receipt污染Wiki证据 | 使用非白名单 `.receipt`，位于 `.dbx-wiki` |
| hook接口信息不足 | 目标hash核对＋db-wiki Manifest确定性协调 |
| 缺少session上下文 | 诚实记录toolCallId/scopeId，不侵入主流程 |
| 工具Schema未来变化 | V1固定；破坏性变化新增v2工具 |

## 24. 已决策与后续项

### 24.1 已决策

- 主路径采用现有function-call扩展，不修改Agent Loop。
- 新能力放在 `agent_files/local_edit`，现有主模块只做最小注册。
- 子feature默认开启；显式关闭时主版本零行为变化。
- 不修改现有 `dbx_file_write/read/stat/parse` 契约。
- expected_hash固定来自 `dbx_file_stat` 原始字节hash。
- V1只支持精确唯一文本锚点，不支持模糊匹配。
- V1支持当前文本写白名单，Office保持只读。
- 不声称任意外部写入强CAS。
- 不为获得session上下文修改Agent Loop。
- 使用scope内部backup和receipt，不增加Storage migration。
- 正文已写后的Manifest异常使用确定性重建协调。
- JSON/YAML/XML/CSV结构化语义工具不进入本版本。

### 24.2 后续独立评审项

- DBX主版本是否提供稳定的 `AgentFunctionExecutionContext`。
- 默认开启后的真实Agent工具选择指标和回退阈值。
- 是否统一 `dbx_file_read` 与stat的raw hash语义。
- 是否增加JSON Pointer或CSV key-column结构化工具。
- 是否为XLSX建立独立M3扩展设计。
- 是否把function-call扩展桥接到MCP或CLI Provider。
- 是否提供受控backup restore工具。
