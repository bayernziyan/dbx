---
title: "Agent 局部文件回写与多格式安全编辑设计"
doc_id: "01-DESIGN-002"
version: "V1.0"
status: "Archived"
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
  - "File-Edit"
  - "Writeback"
  - "Patch"
  - "DB-Wiki"
---

# Agent 局部文件回写与多格式安全编辑设计

> 本版本已被 `01-DESIGN-002 V1.1` 取代，仅保留为历史评审基线，不再作为实现权威。

## 1. 文档定位

本文定义 DBX 内置 Agent、后续 MCP 客户端以及可选 Coding Agent 对 scope 内文件执行局部回写时的统一工具契约、格式策略、并发保护、审计结果和迁移路径。

本文是 `01-DESIGN-001` 的下位专项设计：

- `01-DESIGN-001` 继续负责 DB-Wiki、AI SQL 证据闭环、目录 scope、访问策略和 M1-M4 总体边界。
- 本文负责 M2 文本安全回写、M3 结构化 Office 回写以及 M4 可选 Codex/OpenCode 补丁兼容的具体协议。
- 两份文档发生冲突时，目录权限、阶段边界和安全原则以 `01-DESIGN-001` 为准；局部编辑的工具输入、提交事务和格式适配以本文为准。

本文不授权扩大可写目录。当前只有命中已注册写策略的 scope 可以调用写工具，V1 仍只有末级目录名为 `db-wiki` 的目录获得读写能力。

## 2. 当前实现与问题确认

### 2.1 当前 `dbx_file_write` 是完整内容替换

当前工具 Schema 强制要求：

```json
{
  "scope_id": "scope-id",
  "path": "tables/ecl_task.md",
  "content": "完整最终文件内容",
  "expected_hash": "读取时取得的文件 SHA-256"
}
```

执行链路为：

```text
function_call.content
  -> WriteRequest.content
  -> 格式校验
  -> 临时文件 write_all(content)
  -> flush/sync
  -> rename 覆盖目标
  -> Manifest hook
```

因此它在 function-call 契约层是“模型提交完整最终内容”，不是“模型提交局部变化”。底层采用临时文件原子替换本身是正确方向，问题在于差量计算被交给了模型。

`dbx_wiki_update_from_session` 当前也直接复用同一完整写入方法，存在相同问题。

### 2.2 已有安全能力

当前实现已经具备：

- scope 与相对路径约束；
- 目录读写白名单；
- 扩展名白名单；
- 现存文件 `expected_hash` 校验；
- 新文件 `expected_missing=true` 校验；
- JSON、YAML、XML、CSV、TSV 写前格式校验；
- 同目录临时文件与原子替换；
- 写后 Manifest 同步 hook；
- 写入前后 hash 审计。

本文要求复用这些能力，不建立第二套路径、安全或 Manifest 实现。

### 2.3 完整内容回写的主要风险

1. `dbx_file_read` 单次最多返回 500 行，而更新需要完整 `content`；大文件必须由模型分页读取并重组。
2. 分页行结果不携带原始行终止符，读取还会剥离 UTF-8 BOM；模型重组后可能改变 BOM、CRLF/LF、混合换行和末尾换行。
3. 模型可能遗漏未读取段落、重复拼接分页边界或无意格式化无关区域，`expected_hash` 无法识别这类逻辑性误删。
4. `expected_hash` 在当前内容读取后校验，但校验与最终替换之间没有统一的文件级提交锁和提交前 unchanged 复核，存在 TOCTOU 窗口。
5. Manifest hook 在文件替换后运行；hook 失败时，调用可能表现为失败，但正文文件实际上已经改变。
6. JSON/YAML/XML/CSV 等结构化格式虽然会被整体解析，但让模型手工生成完整文件仍会扩大非目标变化面。

## 3. 外部 Agent 模式与结论

### 3.1 Codex

Codex 的 `apply_patch` 使用结构化 diff：新增、更新、删除由文件操作和带上下文的 hunk 描述。更新位置主要由旧内容和周边上下文确定，行号用于展示或辅助定位，不作为唯一权威锚点。

参考：

- <https://developers.openai.com/api/docs/guides/latest-model?model=gpt-5.2>
- <https://github.com/openai/codex/blob/main/codex-rs/prompts/templates/apply_patch_tool_instructions.md>

### 3.2 OpenCode

OpenCode 明确区分：

- `edit`：使用 `oldString -> newString` 对现存文件做精确局部替换；
- `apply_patch`：应用上下文 patch；
- `write`：创建新文件或有意完整覆盖现存文件。

其当前补丁提交实现使用 `writeIfUnchanged`，在宿主侧基于已读取底稿进行条件写入。

参考：

- <https://opencode.ai/docs/tools>
- <https://github.com/anomalyco/opencode/blob/dev/packages/core/src/tool/apply-patch.ts>
- <https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/tool/edit.ts>

### 3.3 对 DBX 的结论

1. Agent 默认输入应是局部差量，不应是完整最终文件。
2. 行号可以保留为人类可读位置和近邻提示，但不能单独决定修改目标。
3. 权威冲突保护必须同时包含“完整底稿 hash”和“局部旧内容锚点”。
4. DBX 当前主路径是 JSON function calling，需要优先提供模型容易稳定生成的结构化参数；自由文本 patch 作为 M4 兼容能力，不作为 M2 唯一入口。
5. 宿主最终仍可生成完整新字节并原子替换文件；“局部编辑”描述的是模型协议和变更计算方式，不要求对目标文件执行原地字节写入。

## 4. 目标与非目标

### 4.1 目标

1. 现存文本文件默认通过局部编辑更新。
2. 单次调用可安全提交同一文件内多个不重叠编辑。
3. 修改位置同时受完整 `expected_hash`、唯一内容锚点和可选近邻行号约束。
4. 自动保留原文件 BOM、编码、换行风格和末尾换行。
5. 完整文件只在宿主内存中合成，写前继续执行格式与安全校验。
6. 同一路径的并发写入串行化，并在提交前复核底稿未变化。
7. 返回可审计的 changed ranges、有限 diff、hash 和 hook 状态。
8. 按格式选择文本补丁、结构化文档操作或包级编辑，不把二进制 Office 文件当文本处理。
9. Native Agent、MCP 和未来 CLI Provider 复用同一核心编辑引擎。

### 4.2 非目标

- 不开放 scope 外文件写入。
- 不开放删除文件、移动或重命名；这些仍属于后续独立能力。
- 不把 Git patch、Git commit 或分支管理并入 M2。
- 不在 M2 写入 XLS、XLSX、DOC、DOCX。
- 不承诺对任意 YAML/XML 进行语义级重排而完全保留注释和样式。
- 不把文件行号当作乐观锁或唯一定位依据。
- 不允许模型绕过格式验证、敏感信息检测或 Manifest hook。

## 5. 核心设计决策

### 5.1 新增 `dbx_file_edit`

现存文件默认使用新工具 `dbx_file_edit`。`dbx_file_write` 保留用于新建文件和有意完整覆盖，避免改变已有创建语义。

### 5.2 内容锚点优先，行号只作提示

每个编辑必须提供旧内容或插入锚点。`near_line` 只帮助错误诊断和缩小搜索窗口；后端仍要求锚点在底稿中唯一匹配。

不支持“只传第 42 行，把它改成某内容”。原因是：

- 多个编辑会引起后续行号偏移；
- 外部修改可能插入或删除行；
- CSV 字段、XML 文本和 SQL 字符串可能跨物理行；
- 单独行号无法证明模型看过并理解旧内容。

### 5.3 双层冲突保护

```text
文件级：expected_hash == 当前完整文件 SHA-256
局部级：old_text/anchor 在底稿中唯一匹配
```

任一条件不满足均失败关闭，不执行模糊覆盖。

### 5.4 模型提交差量，宿主合成全量

模型只提交 edits。DBX 在受控内存中读取完整底稿、应用 edits、验证最终文档，再通过原子替换提交完整新字节。

### 5.5 格式能力按阶段开放

- M2：UTF-8 文本及结构化文本局部编辑。
- M2 后续：JSON、CSV/TSV 结构化操作。
- M3：XLSX 工作簿结构化编辑；DOCX 保持只读取证，写入另行决策。
- M4：可选 V4A/`apply_patch` 兼容适配。

## 6. 工具体系

```text
dbx_file_read
  └── 返回 hash、行窗口和文本保真元数据

dbx_file_edit
  └── 现存 UTF-8 文本文件的默认局部编辑

dbx_file_write
  └── 新建文件；显式完整覆盖

dbx_file_json_edit          （M2 后续）
  └── JSON Pointer / JSON Patch 语义更新

dbx_file_table_edit         （M2 后续）
  └── CSV/TSV 行列语义更新

dbx_file_workbook_edit      （M3）
  └── XLSX sheet/cell/range/table 更新

dbx_file_apply_patch        （M4 可选）
  └── Codex/OpenCode patch 兼容入口
```

所有工具最终调用统一的：

```text
FileMutationEngine
  ├── scope/path/policy validation
  ├── SnapshotLoader
  ├── MutationAdapter
  ├── FormatValidator
  ├── SensitiveContentGuard
  ├── ConditionalAtomicWriter
  ├── AuditBuilder
  └── afterWrite hooks
```

## 7. `dbx_file_edit` 契约

### 7.1 输入

```json
{
  "scope_id": "wiki-scope-id",
  "path": "tables/ecl_task.md",
  "expected_hash": "sha256-of-current-file",
  "edits": [
    {
      "op": "replace",
      "old_text": "status: ready",
      "new_text": "status: done",
      "expected_occurrences": 1,
      "near_line": 42
    }
  ]
}
```

字段规则：

| 字段 | 必填 | 规则 |
|---|---:|---|
| `scope_id` | 是 | 必须对应仍存活且可写的 scope |
| `path` | 是 | scope 内相对路径，目标必须已存在且是允许写入的文本格式 |
| `expected_hash` | 是 | 必须匹配本次提交底稿的完整 SHA-256 |
| `edits` | 是 | 1-50 个编辑，按底稿解析，不得重叠 |
| `op` | 是 | `replace`、`delete`、`insert_before`、`insert_after` |
| `old_text` | 是 | `replace/delete` 的旧内容，或 insert 的锚点文本 |
| `new_text` | 条件必填 | `replace/insert_*` 必填；`delete`省略或为空 |
| `expected_occurrences` | 否 | V1 只能为 1，默认 1 |
| `near_line` | 否 | 仅作提示和错误报告，不改变唯一匹配要求 |

### 7.2 操作语义

#### replace

```text
old_text -> new_text
```

#### delete

删除唯一匹配的 `old_text`。

#### insert_before

在唯一匹配的锚点前插入 `new_text`，锚点本身保留。

#### insert_after

在唯一匹配的锚点后插入 `new_text`，锚点本身保留。

### 7.3 匹配规则

1. 先基于原始底稿解析所有锚点，不边修改边搜索。
2. 工具输入中的 `\n` 按目标文件主换行风格转换后匹配；混合换行文件必须采用精确模式，不自动统一全文件。
3. 每个锚点必须唯一；零匹配返回 `FILE_EDIT_ANCHOR_NOT_FOUND`，多匹配返回 `FILE_EDIT_ANCHOR_AMBIGUOUS`。
4. `near_line`存在时，返回最接近候选的位置帮助重新读取，但不能自动选择候选。
5. 编辑范围不得重叠；插入点与删除/替换范围冲突时返回 `FILE_EDIT_OVERLAP`。
6. 所有范围确认后按字节偏移倒序应用，保证前序编辑不改变后序定位。
7. 最终内容与底稿相同返回 `FILE_EDIT_NO_CHANGE`，不触发写入和 hooks。

### 7.4 资源预算

- 目标文本文件继续受 2 MiB 上限约束；扩大上限必须单独评估。
- 单次最多 50 个 edits。
- 单个 `old_text` 默认不超过 64 KiB。
- 单个 `new_text` 默认不超过 256 KiB。
- 单次新增总字节数默认不超过 512 KiB。
- 返回 diff 默认最多 200 行或 32 KiB，超出时标记 `diffTruncated=true`。

### 7.5 输出

```json
{
  "scopeId": "wiki-scope-id",
  "path": "tables/ecl_task.md",
  "writeApplied": true,
  "previousHash": "...",
  "contentHash": "...",
  "changedRanges": [
    {
      "oldStartLine": 42,
      "oldEndLine": 42,
      "newStartLine": 42,
      "newEndLine": 42,
      "operation": "replace"
    }
  ],
  "additions": 1,
  "deletions": 1,
  "diff": "@@ ...",
  "diffTruncated": false,
  "hooks": [
    { "name": "sync-manifest", "status": "succeeded" },
    { "name": "reindex-on-next-search", "status": "scheduled" }
  ]
}
```

结果中的行号是基于提交前后内容计算的审计信息，不是下一次编辑可复用的锁；下一次编辑必须重新读取并取得新 hash。

## 8. `dbx_file_read` 增强

为支持模型生成精确差量，读取结果新增但不破坏现有字段：

```json
{
  "contentHash": "...",
  "encoding": "utf-8",
  "bom": "utf-8",
  "lineEnding": "crlf",
  "mixedLineEndings": false,
  "endsWithNewline": true,
  "windowStartLine": 40,
  "windowEndLine": 60,
  "windowHash": "...",
  "text": "保留窗口内原始行终止符的精确文本片段",
  "lines": []
}
```

规则：

1. 保留现有 `lines`、`lineCount`、`continuationLine`，兼容当前调用方。
2. `text` 是所选窗口的精确文本片段，用于直接构造 `old_text`。
3. `windowHash` 对精确窗口字节计算，仅用于审计和诊断；文件级并发控制仍使用 `contentHash`。
4. BOM 不放进 `text`，但通过 `bom` 明确返回并由编辑引擎自动保留。
5. 混合换行时返回 `mixedLineEndings=true`，编辑引擎不对全文件做换行归一化。

## 9. `dbx_file_write` 收敛策略

### 9.1 目标语义

```text
新文件：content + expected_missing=true
现存文件完整覆盖：content + expected_hash + full_replace=true
现存文件默认更新：使用 dbx_file_edit
```

新增字段：

```json
{
  "full_replace": true,
  "reason": "用户要求重建整个生成文件"
}
```

`reason`进入审计但不作为权限判断依据。

### 9.2 兼容迁移

阶段 A：

- 新增 `dbx_file_edit`；
- 修改工具描述，要求现存文件默认使用 edit；
- 保留旧的完整覆盖调用，返回 `warnings=["FULL_REPLACE_DEPRECATED_FOR_EXISTING_FILE"]`。

阶段 B：

- 观察 Native Agent 与 MCP 调用结果；
- 更新测试和 Prompt；
- 现存文件完整覆盖必须显式传 `full_replace=true`。

阶段 C：

- 对大型现存文件或非生成文件，可由 policy 禁止完整覆盖；
- 生成型文件可以由独立 policy 明确允许 full replace。

## 10. `dbx_wiki_update_from_session` 调整

该工具不能继续无条件委托完整 `write`：

- 新建会话总结文件：使用 `content + expected_missing=true`。
- 更新现有 Wiki：使用 `expected_hash + edits`，委托 `FileMutationEngine`。
- 明确重建整份机器生成文档：必须 `full_replace=true`，并返回完整覆盖告警。
- 会话总结仍只能写入有 Citation 的事实；文件编辑能力不放宽知识权威规则。

## 11. 条件原子提交

### 11.1 提交流程

```text
1. 验证 scope、policy、相对路径和扩展名
2. 获取规范化目标路径对应的进程内写锁
3. 在锁内读取原始字节、文件标识和元数据
4. 验证 expected_hash
5. 解码并记录 BOM、换行和末尾换行
6. 在同一底稿上解析所有 edits
7. 检查唯一性、不重叠和资源预算
8. 应用 edits，恢复原始文本属性
9. 执行格式、安全和敏感内容校验
10. 写入同目录唯一临时文件并 sync_all
11. 替换前重新读取目标摘要，执行 write-if-unchanged
12. 原子替换目标文件
13. 计算最终 hash 和 changed ranges
14. 执行 afterWrite hooks
15. 写审计并返回结构化结果
```

### 11.2 并发边界

- 进程内同一规范化路径使用独占锁；不同文件可以并行。
- 文件锁与 scope registry 分离，避免一个 scope 阻塞另一个 scope 的无关文件。
- 提交前必须重新验证目标仍是步骤 3 的底稿；不允许仅依赖步骤 4 的早期校验。
- 外部程序在最终复核前修改文件时返回 `FILE_HASH_CONFLICT`。
- 平台无法提供真正文件系统 CAS 时，必须记录该边界；不得把“临时文件 rename”单独描述成完整并发安全。

### 11.3 hook 部分失败

正文提交和 afterWrite hook 分开报告：

```json
{
  "writeApplied": true,
  "contentHash": "...",
  "hooks": [
    {
      "name": "sync-manifest",
      "status": "failed",
      "errorCode": "MANIFEST_WRITE_FAILED"
    }
  ],
  "status": "written_with_hook_failure"
}
```

禁止在正文已写入后只返回一个无上下文的普通错误，使 Agent误以为文件没有改变。Agent收到该状态后只重试失败 hook，不得重复应用 edits。

## 12. 文本保真规则

### 12.1 编码

- M2写入继续只支持 UTF-8、UTF-8 BOM。
- 无 BOM 文件保持无 BOM；UTF-8 BOM 文件自动保留 BOM。
- 非 UTF-8 文件返回 `FILE_ENCODING_WRITE_UNSUPPORTED`，不得猜测编码。

### 12.2 换行

- 统一 CRLF 文件的新增文本转换为 CRLF。
- 统一 LF 文件的新增文本保持 LF。
- 混合换行文件只修改目标范围，不统一无关行。
- `new_text`未显式包含末尾换行时，不改变文件级 `endsWithNewline`，除非编辑目标覆盖文件末尾。

### 12.3 Unicode

- 不自动执行 NFC/NFD 归一化。
- hash基于实际最终字节，不基于字符归一化结果。
- changed ranges按 Unicode 标量和行号报告，不暴露不稳定的模型字节偏移。

## 13. 各格式回写策略

| 格式 | M2/M3策略 | 定位方式 | 写后验证 | 保真重点 |
|---|---|---|---|---|
| Markdown | M2文本局部编辑 | 唯一旧文本/标题上下文 | 文本与可选 Front Matter解析 | 标题层级、代码块、链接、换行 |
| TXT | M2文本局部编辑 | 唯一旧文本 | UTF-8与资源预算 | BOM、换行、末尾空行 |
| SQL | M2文本局部编辑 | 唯一语句/注释上下文 | SQL安全扫描；不承诺完整方言解析 | 分隔符、注释、字符串字面量 |
| JSON | M2文本补丁；后续JSON Pointer | 唯一文本或JSON Pointer | 完整 JSON 解析 | 缩进、键顺序、末尾换行 |
| YAML/YML | M2文本局部编辑 | 唯一键块上下文 | 完整 YAML 解析 | 注释、锚点、alias、样式 |
| XML | M2文本局部编辑 | 唯一元素上下文；后续XPath | 完整 XML 解析 | namespace、注释、属性与空白 |
| CSV/TSV | M2文本补丁；后续表格语义编辑 | 唯一行块或主键列 | CSV Reader完整解析 | 分隔符、引号、字段内换行、BOM |
| XLS | 只读 | 不适用 | 不适用 | 旧二进制格式不写 |
| XLSX | M3结构化工作簿编辑 | sheet/cell/range/table | ZIP、workbook关系、公式与重开验证 | 未改sheet、样式、公式、合并单元格 |
| DOCX | M3只读取证 | 不适用 | 不适用 | 写入未获授权 |
| DOC | 不支持，要求转换 | 不适用 | 不适用 | 旧二进制格式不解析/不写 |

### 13.1 Markdown

V1不强制 Markdown AST 重写，避免格式化无关区域。局部编辑完成后至少检查：

- Front Matter（存在时）仍可解析；
- fenced code block没有因编辑产生明显未闭合；
- `SUMMARY.md`写入后Manifest仍能确定性生成。

未来可以增加 `replace_section` 语义操作：以完整标题路径为选择器，后端把它转换为受 hash保护的文本 edit。

### 13.2 SQL

SQL 文件属于知识和案例，不等同于立即执行：

- 局部写入继续受文件安全扫描约束；
- 写文件不得提升数据库执行权限；
- 不自动格式化整份 SQL；
- 字符串字面量、存储过程和方言分隔符使行级替换风险较高，应使用更大上下文锚点。

### 13.3 JSON

第一阶段沿用文本局部编辑并对最终内容完整解析。第二阶段增加 `dbx_file_json_edit`：

```json
{
  "scope_id": "...",
  "path": "config.json",
  "expected_hash": "...",
  "operations": [
    { "op": "replace", "pointer": "/documents/0/kind", "value": "table" }
  ]
}
```

结构化编辑必须：

- 使用 RFC 6901 JSON Pointer；
- 支持 `add/replace/remove/test` 的安全子集；
- 默认带 `test`或旧值条件；
- 尽量保留原缩进、键顺序、BOM和末尾换行；
- JSONC 不属于当前 `.json` 契约，不得把注释文件当标准 JSON 回写。

### 13.4 YAML/YML

普通数据模型解析再完整序列化会丢失注释、锚点样式和格式。M2只允许文本局部补丁＋完整语法验证。

只有引入可保留 concrete syntax tree 的 round-trip 库并完成 fixture 验证后，才能开放 YAML Path 语义编辑。无法保证保真时继续使用文本锚点或拒绝写入。

### 13.5 XML

M2使用文本局部补丁并完整解析。后续 XPath 编辑必须显式处理 namespace，不得仅按标签名匹配。若序列化器会重排属性、前缀或空白，则不能用于默认写回。

### 13.6 CSV/TSV

物理行不等于逻辑记录，因为带引号字段可以包含换行。后续 `dbx_file_table_edit` 使用：

```json
{
  "scope_id": "...",
  "path": "tables/status.csv",
  "expected_hash": "...",
  "key_columns": ["code"],
  "operations": [
    {
      "op": "update_row",
      "key": { "code": "1" },
      "expected": { "label": "Ready" },
      "set": { "label": "Done" }
    }
  ]
}
```

必须校验：

- 表头唯一；
- key 命中唯一记录；
- expected旧值一致；
- 原分隔符、引号策略、BOM和换行尽量保留；
- 写后重新解析记录数与字段数。

### 13.7 XLSX

XLSX 是 ZIP/XML 包，禁止传入 `dbx_file_edit` 或 `dbx_file_write`。M3新增 `dbx_file_workbook_edit`：

```json
{
  "scope_id": "...",
  "path": "metadata.xlsx",
  "expected_hash": "...",
  "operations": [
    {
      "op": "set_cell",
      "sheet": "tables",
      "cell": "D12",
      "expected_value": "Ready",
      "value": "Done"
    }
  ]
}
```

处理流程：

1. 复制为同目录临时工作簿；
2. 校验 workbook、sheet、cell/range和旧值；
3. 只修改目标XML部件；
4. 保留未修改sheet、关系、样式、共享字符串和公式；
5. 重建ZIP后重新打开并验证；
6. 对关键未修改部件计算摘要，发现非目标变化时拒绝提交；
7. 条件原子替换原文件。

宏、外部链接、嵌入对象、密码保护或无法保真的工作簿失败关闭。

### 13.8 DOCX与旧Office格式

- DOCX在M3继续只读取证，不提供通用文本写回。
- 若未来开放DOCX写入，必须使用段落、run、表格单元格和关系级操作，不能对解压XML做任意字符串替换。
- `.doc`、`.xls`旧二进制格式不原生写入，返回稳定转换要求。

## 14. 格式、安全与审计

### 14.1 最终内容验证

局部编辑不是绕过完整验证：

- JSON、YAML、XML、CSV、TSV继续整体解析；
- Markdown执行Front Matter与基础结构检查；
- SQL执行现有敏感信息和危险内容规则；
- Office执行包结构、资源预算和保真检查；
- 任意格式都执行 secret/PII 规则和允许内容策略。

### 14.2 审计记录

审计至少包含：

- scope id、policy id、规范化相对路径；
- tool name与编辑类型；
- previous hash、content hash；
- 操作数量、changed ranges、增删行数；
- `full_replace`标记与reason；
- 格式验证结果；
- hook逐项状态；
- 时间、会话id和调用来源。

默认不持久化完整旧内容、新内容或完整 diff。需要保留 diff 时必须有大小上限和敏感内容清洗。

### 14.3 主要错误码

| 错误码 | 含义 | Agent下一步 |
|---|---|---|
| `FILE_EXPECTED_HASH_REQUIRED` | 缺少底稿hash | 重新read/stat |
| `FILE_HASH_CONFLICT` | 文件已变化 | 重新读取并重建edits |
| `FILE_EDIT_ANCHOR_NOT_FOUND` | 旧内容不存在 | 读取目标窗口 |
| `FILE_EDIT_ANCHOR_AMBIGUOUS` | 锚点不唯一 | 提供更大上下文 |
| `FILE_EDIT_OVERLAP` | 多个编辑范围冲突 | 合并或拆分操作 |
| `FILE_EDIT_NO_CHANGE` | 结果无变化 | 停止重复写入 |
| `FILE_EDIT_LIMIT_EXCEEDED` | edits或内容超预算 | 拆分调用 |
| `FILE_FULL_REPLACE_CONFIRMATION_REQUIRED` | 现存文件完整覆盖未显式确认 | 改用edit或传full_replace |
| `FILE_ENCODING_WRITE_UNSUPPORTED` | 非UTF-8文本 | 转换格式或只读 |
| `FORMAT_*_INVALID` | 最终文档无效 | 修正局部变更 |
| `FILE_WRITE_RACE` | 提交前底稿变化 | 重新读取 |
| `FILE_WRITE_HOOK_FAILED` | 正文已写但hook失败 | 只重试hook |

错误返回必须包含 `writeApplied`。在任何可能发生部分成功的阶段，Agent必须据此决定是否允许重试。

## 15. Agent调用策略

工具描述和系统提示应明确：

1. 更新现存文本文件默认先read/search，再调用 `dbx_file_edit`。
2. `old_text`必须从本次read结果取得，不得凭记忆构造。
3. 行号只用于 `near_line`，必须同时提交旧内容锚点。
4. 锚点不唯一时扩大读取窗口，不使用 `replace_all`。
5. hash冲突后重新读取，不把旧 edits 原样重试。
6. 新建文件使用 `dbx_file_write + expected_missing=true`。
7. 只有用户明确要求重建整个文件或目标是确定性生成物时才完整覆盖。
8. Office文件必须调用对应结构化工具；无对应工具时保持只读。

`dbx_file_edit`不得标记为parallel-safe；同一Agent回合内依赖read取得的 hash和锚点，调用顺序必须保留。

## 16. 代码改造范围

### 16.1 `agent_files`新增模块

```text
crates/dbx-core/src/agent_files/
├── file_edit.rs
├── file_snapshot.rs
├── mutation_engine.rs
├── conditional_writer.rs
├── text_fidelity.rs
└── diff.rs
```

职责：

- `file_snapshot.rs`：原始字节、hash、BOM、换行、文件标识。
- `file_edit.rs`：JSON输入模型、锚点解析、范围冲突检测。
- `mutation_engine.rs`：统一编辑事务编排。
- `conditional_writer.rs`：路径锁、提交前复核、临时文件和原子替换。
- `text_fidelity.rs`：编码、BOM、换行和尾部换行保真。
- `diff.rs`：changed ranges与有界diff。

### 16.2 修改现有模块

| 文件 | 修改 |
|---|---|
| `tool_catalog.rs` | 注册 `dbx_file_edit`；增强write描述；后续注册结构化工具 |
| `file_tools.rs` | 分派edit；read返回保真元数据；session update选择edit/write |
| `file_write.rs` | 抽取公共validator与atomic writer；保留Manifest生成 |
| `audit.rs` | 增加changed ranges、fullReplace、hook状态和部分成功 |
| `db_wiki_policy.rs` | 可选区分局部编辑与完整覆盖能力 |
| `mod.rs` tests | 增加局部编辑、保真和竞态测试 |
| `wiki/write.rs` | 委托统一mutation engine，不复制写逻辑 |

### 16.3 禁止事项

- 不在 `agent_loop.rs` 内实现 patch算法。
- 不在Desktop或MCP适配层复制文件编辑逻辑。
- 不用 PowerShell、Python、sed等shell命令实现Agent正式回写。
- 不允许结构化格式绕过最终解析。
- 不为了局部编辑扩大目录白名单。

## 17. 分阶段实施

### P0：安全基线修复

- 抽取FileSnapshot和ConditionalAtomicWriter。
- 增加规范化路径级写锁。
- 提交前复核unchanged。
- 保留BOM、换行和末尾换行。
- hook返回结构化部分成功。

退出条件：现有完整写入在并发、文本保真和hook失败上有明确结果，行为测试通过。

### P1：文本局部编辑

- 注册 `dbx_file_edit`。
- 实现四种基础操作、唯一锚点、重叠检测和倒序应用。
- 增强read返回精确窗口文本和保真元数据。
- 更新Agent提示，现存文件默认edit。
- `dbx_wiki_update_from_session`支持edits。

退出条件：Markdown/TXT/SQL/JSON/YAML/XML/CSV/TSV可以仅提交局部差量，未修改区域字节保持一致，写后完整格式验证通过。

### P2：结构化文本编辑

- 增加JSON Pointer安全子集。
- 增加CSV/TSV key-column行列编辑。
- 根据round-trip能力决定是否增加YAML Path和XPath；不满足保真要求则保持文本补丁。

退出条件：结构化操作可以验证旧值、唯一对象和非目标区域保真。

### P3：M3工作簿编辑

- 实现XLSX结构化工具和临时包验证。
- 覆盖公式、样式、合并单元格、共享字符串及危险包场景。
- DOCX继续只读，除非新增独立Active设计授权。

退出条件：简单工作簿可安全更新；复杂或不可保真工作簿稳定拒绝，原文件不变。

### P4：M4 Coding Agent兼容

- 增加可选 `dbx_file_apply_patch`。
- 解析受限V4A patch语法并转换为统一MutationPlan。
- CLI Provider仍必须通过scope和policy，不直接获得目录外shell写权限。

退出条件：Codex/OpenCode风格patch与Native function-call edit产生相同安全结果和审计结构。

## 18. 测试设计

### 18.1 基础编辑

- replace/delete/insert_before/insert_after成功。
- 锚点零匹配、多匹配和更大上下文重试。
- 多编辑不重叠成功、重叠失败。
- edits基于同一底稿解析，倒序应用后位置正确。
- no-change不触发写入和hook。

### 18.2 冲突与并发

- 缺少expected hash失败。
- 外部修改导致hash冲突。
- hash校验后、rename前发生修改时条件写失败。
- 同一路径并发串行化，不同路径可并行。
- 失败后临时文件清理。

### 18.3 文本保真

- UTF-8无BOM保持无BOM。
- UTF-8 BOM保持BOM。
- LF、CRLF和末尾换行分别保持。
- 混合换行只改变目标范围。
- 中文、emoji、组合字符和长行。
- 空文件、新建文件、仅一行文件和文件末尾编辑。

### 18.4 格式

- Markdown Front Matter与代码块。
- JSON格式错误拒绝且原文件不变。
- YAML注释和锚点在文本局部编辑后保留。
- XML namespace、注释和实体。
- CSV/TSV引号、逗号、Tab、字段内换行和BOM。
- SQL多语句、注释、存储过程分隔符和字符串。
- XLSX公式、样式、合并单元格、隐藏sheet及不支持包拒绝。

### 18.5 hook与审计

- Manifest同步成功。
- 正文成功、Manifest失败返回 `written_with_hook_failure`。
- 重试hook不重复正文编辑。
- changed ranges、hash、增删统计准确。
- diff截断和敏感内容清洗。

### 18.6 安全回归

- 只读scope不能edit。
- 路径穿越、绝对路径、symlink/reparse-point逃逸继续阻断。
- 非白名单扩展名继续阻断。
- XLSX/DOCX不能误入文本工具。
- feature flag关闭时不注册新增工具。
- 基础数据库工具与SQL权限行为不变。

## 19. 验收标准

1. Agent更新一个1000行Markdown时只提交目标段落和上下文，不提交完整文件。
2. 未修改区域与提交前字节完全相同；BOM、换行和尾部换行保持。
3. 锚点重复时不自动选择，返回稳定歧义错误。
4. 外部编辑发生后不能被旧hash或旧patch覆盖。
5. JSON/YAML/XML/CSV/TSV写回后完整解析通过，失败时目标文件不变。
6. 同一路径并发编辑不会丢失更新。
7. 正文已写而hook失败时返回明确部分成功，不允许Agent重复应用正文变更。
8. 审计包含旧/新hash、changed ranges、操作统计和hook状态，不保存完整敏感内容。
9. 新建文件和显式完整覆盖仍可用，普通现存文件默认走edit。
10. M2不写Office二进制；M3 XLSX无法保真时拒绝替换原文件。
11. Native Agent与MCP调用同一核心引擎并得到一致错误码和结果模型。
12. feature flag关闭后新增工具消失，原Ask/Agent和数据库能力无行为变化。

## 20. 兼容、回滚与发布

### 20.1 兼容

- `dbx_file_edit`是新增工具，不修改现有read/list/search参数。
- `dbx_file_read`只增加字段，不删除现有字段。
- `dbx_file_write`先以告警方式收敛，后续版本才强制 `full_replace=true`。
- MCP输出Schema必须版本化；旧客户端可以继续使用write过渡。
- CLI Provider兼容属于M4，不阻塞Native Agent的P0-P2。

### 20.2 回滚

- 关闭 `DBX_AGENT_FILE_TOOLS` 移除全部文件工具。
- 独立的局部编辑feature flag可临时退回旧write，但不得绕过P0条件写安全修复。
- P2/P3结构化适配器可以按工具单独禁用。
- Manifest和索引仍可由Wiki文件重建。
- 本设计不自动生成Git提交；文件级业务回滚继续依赖版本控制或明确备份策略。

### 20.3 灰度指标

- edit成功率；
- anchor not found/ambiguous比例；
- hash conflict比例；
- full replace调用比例；
- 格式验证失败率；
- hook部分失败率；
- 平均输入字节和完整写入相比的Token下降比例；
- 外部修改被条件写阻断次数；
- 未修改区域hash异常次数，目标必须为0。

## 21. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 模型提供过短锚点 | 强制唯一匹配；歧义时要求扩大上下文 |
| 行号漂移 | 行号只作提示；hash和内容锚点为权威 |
| 多编辑互相影响 | 基于同一底稿解析、检查重叠、倒序应用 |
| 文本属性被破坏 | FileSnapshot记录并恢复BOM、换行和尾部换行 |
| 外部并发覆盖 | 路径锁、expected hash、提交前unchanged复核 |
| 格式整体无效 | 在临时文件提交前对最终文档完整验证 |
| YAML/XML重序列化污染 | M2仅文本补丁；无round-trip保证不开放语义重写 |
| CSV物理行误判 | 结构化阶段使用CSV parser和key columns |
| Office包损坏 | 包级临时副本、重开验证、非目标部件摘要 |
| hook失败导致重复写 | 返回writeApplied和逐项hook状态，只重试hook |
| 工具数量增加影响模型选择 | 分阶段注册；工具描述明确默认路径；feature flag控制 |
| diff泄漏敏感内容 | 有界、清洗，持久审计默认不存完整diff |

## 22. 已决策事项

- 现存文本文件默认使用 `dbx_file_edit`。
- 行号不是权威修改定位；唯一旧内容锚点和完整hash共同决定修改。
- 宿主内部仍使用完整新字节的条件原子替换。
- 新建和显式完整覆盖继续使用 `dbx_file_write`。
- Native function calling优先使用JSON结构化edit，patch文本兼容延后到M4。
- M2支持文本及结构化文本的安全局部编辑，不写Office二进制。
- JSON和CSV/TSV优先增加专用结构化操作。
- YAML/XML没有round-trip保真能力前不做默认语义重序列化。
- M3只交付受限XLSX结构化回写；DOCX继续只读。
- 所有适配器必须汇聚到单一 `FileMutationEngine`，不在Desktop、MCP或CLI复制实现。
- 正文提交与hook结果必须分开报告，部分成功不得伪装成全失败。
