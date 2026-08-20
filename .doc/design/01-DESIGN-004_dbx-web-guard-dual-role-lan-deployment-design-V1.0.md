---
title: "DBX Web Guard 双角色访问控制与局域网部署设计"
doc_id: "01-DESIGN-004"
version: "V1.0"
status: "Review"
created_date: "2026-08-20"
last_updated: "2026-08-20"
maintainer: "DBX DB-Wiki Project"
constraint_level: "Normative"
review_cycle: "On-demand"
related_docs:
  - "01-DESIGN-001"
tags:
  - "DBX"
  - "DBX-Web"
  - "Guard"
  - "RBAC"
  - "Nginx"
  - "Windows"
  - "LAN"
  - "AI-Agent"
---

# DBX Web Guard 双角色访问控制与局域网部署设计

## 1. 文档定位与状态

本文定义 DBX Web 在 Windows 内网环境中的非 Docker 部署、Nginx `/dbx/` 入口、独立 `dbx-web-guard` 双角色认证、Viewer 界面收敛、API 强制授权、开机自启、升级与回滚方案。

本文状态为 `Review`。它是后续实现、部署和验收的评审基线，但在实现与验收完成前不代表当前机器已具备 Guard、双角色、Nginx `/dbx/` 或自动启动能力。

权威边界如下：

1. DB-Wiki、AI SQL 证据、Agent 文件工具和数据库权限继续以 `01-DESIGN-001` 及 DBX 当前实现为准。
2. 本文不重新定义 DBX 的 SQL 风险分类、连接白名单、MCP 策略或 Agent 工具权限。
3. 本文只在 DBX Web 外层增加认证、角色授权、界面注入和部署编排，不把角色逻辑侵入 `dbx-core`、Desktop 或现有 Web API 处理器。
4. CSS/JS 隐藏只改善 Viewer 界面，不是安全边界；所有受限操作必须由 Guard 在服务端返回 403。
5. 本文不记录任何明文密码。已确定的上游固定密码只在部署时写入 Windows 加密凭据存储。

## 2. 用户目标

本方案满足以下目标：

1. PC 和移动端浏览器通过局域网访问 DBX AI Agent。
2. 统一入口为 `http://<server-ip>:82/dbx/`；后续启用 HTTPS 时路径保持 `/dbx/`。
3. 保留 `dbx-web` 当前单密码认证，并在外层增加管理员密码和普通用户密码。
4. 管理员登录后可见并可使用完整 DBX Web 功能。
5. 普通用户登录后不显示右上角“检查更新、主题、GitHub、设置”等管理入口，且不能绕过界面直接调用对应 API。
6. 不增加 `/guard/session` 接口；角色由服务端会话决定，前端仅接收非敏感的 UI 角色提示。
7. Guard、DBX Web 和 Nginx 随 Windows 启动，由 Windows 服务管理，不建立 Nginx 子进程依赖。
8. 后端固定部署在 `D:\dbx-web\server`，Guard 独立部署，Nginx 继续位于 `D:\nginx`，不使用 Docker。
9. 尽量不改 DBX 原源码；Guard 作为独立扩展构建和发布。
10. Web 与已安装 Desktop 使用同一份 DBX 配置、数据源、AI 配置和策略数据，同时明确并发与升级边界。

## 3. 已确认现状（As-Is）

### 3.1 DBX Web 源码能力

当前 `crates/dbx-web` 已具备：

- 复用 `dbx_core::agent_loop::run_agent_loop` 的 AI Agent SSE 接口。
- 数据源、Schema、查询、AI 配置、会话、Database Docs、MCP 策略和应用设置接口。
- `DBX_DATA_DIR`、`DBX_STATIC_DIR`、`DBX_PORT` 配置。
- `DBX_PUBLIC_BASE_PATH` 上下文路径，已包含 `/dbx` 规范化、Router 嵌套、入口重定向和 Cookie Path 支持。
- 单密码认证：密码使用 Argon2 哈希存入 `dbx.db`，登录后生成进程内 `dbx_session`。
- 登录失败 5 次后锁定 60 秒。
- API 认证中间件；静态资源本身允许匿名加载。

当前 DBX Web 不具备：

- `admin` / `viewer` 角色模型。
- 按角色区分 API 权限。
- 按角色输出不同的工具栏和设置界面。
- 独立 Guard 进程或 Guard 会话接口。

### 3.2 当前 Windows 部署

当前后端目录为：

```text
D:\dbx-web\server\
├── dbx-web.exe
├── start-dbx-web.ps1
├── data\
│   └── dbx.db
├── logs\
└── static\
```

当前启动脚本仍使用端口 `4224`，尚未设置 `/dbx` 上下文路径。当前 Nginx 监听 `82`，已有 `/api/` 到其他服务的代理规则，但尚未配置 `/dbx/`。

因此本文描述的目标入口、端口调整、Guard、角色授权、Windows 服务和防火墙规则均为待实施项。

### 3.3 当前部署约束

- `D:\dbx-web\server\data\dbx.db` 必须跨升级保留。
- 当前 DBX Web 进程固定绑定 `0.0.0.0:<port>`，没有单独的监听地址环境变量。
- Desktop 与 Web 均支持通过 `DBX_DATA_DIR` 指定数据目录。
- DBX Storage 使用 SQLite 单连接和 10 秒 busy timeout；没有证据表明当前默认打开 WAL。
- 已安装 Desktop 与 Web 若同时指向同一 `dbx.db`，SQLite 能进行文件锁，但配置写入、历史写入和启动迁移仍可能竞争，必须进行同版本、启动顺序和并发验收。

## 4. 核心架构决策

### 4.1 独立 Guard，不修改 DBX 角色模型

采用以下链路：

```text
LAN Browser
    │  http(s)://<server>/dbx/
    ▼
Nginx :82 / :443
    │  preserve /dbx prefix
    ▼
dbx-web-guard :4226
    ├── dual-password authentication
    ├── guard session and role
    ├── upstream DBX session mapping
    ├── viewer API authorization
    ├── HTML CSS/JS injection
    └── audit / health
    │  preserve /dbx prefix
    ▼
dbx-web :4225
    ├── native single-password authentication
    ├── existing DBX APIs
    ├── existing SQL and MCP policies
    └── existing AI Agent loop
```

理由：

- DBX 原登录只有一个密码和一类会话，直接在原模块增加角色会扩大源码侵入面。
- Nginx 的静态 location 规则不足以安全表达登录、会话和细粒度 API 权限。
- Guard 可以在不改变 DBX 请求和响应契约的前提下拦截认证、保存角色、代理 SSE，并在升级失败时独立回滚。

### 4.2 Nginx 不管理 Guard 子进程

Nginx、Guard 和 DBX Web 都由 Windows 服务管理。Nginx 只做代理，不使用脚本拉起或监护 Guard。

目标依赖顺序：

```text
DBXWebBackend -> DBXWebGuard -> DBXNginx
```

任一内部服务未就绪时，外部入口返回明确的 502/503，而不是绕过 Guard 访问后端。

### 4.3 保留 DBX 原密码

不关闭 DBX Web 原密码。Guard 不是用两个角色密码直接登录 DBX，而是维护一个仅服务端使用的上游服务凭据。

这样即使本机误开放了后端端口，DBX 仍有原生密码保护；同时 Guard 可为每个用户创建独立上游会话，保留 DBX 的会话临时凭据隔离。

### 4.4 无 `/guard/session`

Guard 不提供 `/guard/session`、`/api/guard/session` 或等价角色查询接口。

服务端授权只读取 HttpOnly Guard Session。前端界面注入通过通用 bootstrap 脚本和一个非敏感 UI Role Cookie 完成。伪造 UI Role Cookie 最多改变按钮是否显示，不能提升服务端权限。

### 4.5 Viewer 安全以 API 403 为准

Viewer 的隐藏元素升级后可能因 DOM 结构或选择器变化重新显示。此时：

- 点击受限按钮必须得到 403。
- 直接调用受限 API 必须得到 403。
- 伪造 UI Role Cookie、删除 CSS、禁用 JS 或手工构造请求都不能提升权限。

## 5. 三类密码与两层会话

### 5.1 凭据分类

| 凭据 | 使用方向 | 保存方式 | 浏览器可见 | 作用 |
|---|---|---|---|---|
| DBX 上游服务密码 | Guard → DBX Web | Windows DPAPI 加密文件，NTFS 限权 | 否 | 调用 DBX 原 `/api/auth/login` |
| Guard 管理员密码 | Browser → Guard | Argon2id 哈希 | 仅用户输入时 | 建立 `admin` 会话 |
| Guard 普通用户密码 | Browser → Guard | Argon2id 哈希 | 仅用户输入时 | 建立 `viewer` 会话 |

上游服务密码已确定为固定值，并与当前 DBX Web 原登录密码保持一致。该值不得出现在 Git、Markdown、TOML、PowerShell、Windows 服务 XML、环境变量、进程命令行或日志中。

### 5.2 是否需要预配置上游密码

需要。安装 Guard 时必须先执行一次交互式凭据写入：

```text
dbx-web-guard credential set-upstream
```

命令从控制台安全输入密码，完成以下步骤：

1. 调用本机 DBX Web `/dbx/api/auth/login` 验证密码。
2. 验证成功后使用 Windows DPAPI LocalMachine 范围加密。
3. 写入 Guard secrets 目录。
4. 设置仅 Windows 服务身份、SYSTEM 和 Administrators 可读的 ACL。
5. 不回显密码，不把密码写入命令历史。

若未配置、解密失败或与 DBX 当前密码不一致，Guard 的 readiness 为失败，对业务请求返回 503 `WEB_GUARD_UPSTREAM_AUTH_UNAVAILABLE`。

### 5.3 登录映射

登录流程：

```text
1. Browser POST /dbx/api/auth/login { password }
2. Guard 分别校验 admin_hash 和 viewer_hash
3. 命中一个且仅命中一个角色
4. Guard 使用加密保存的上游密码调用 DBX 原登录接口
5. DBX 返回一个新的 dbx_session
6. Guard 生成随机 guard_session
7. 内存保存：guard_session -> role + upstream_dbx_session + timestamps
8. Browser 只收到 guard_session，不收到 upstream dbx_session
```

管理员与普通用户都使用同一上游服务密码，但每次 Guard 登录都创建不同的 DBX 上游 Session。禁止全局共享一个上游 Session。

### 5.4 Session Cookie

Guard Session Cookie：

```text
Name: dbx_guard_session
Path: /dbx
HttpOnly: true
SameSite: Strict
Secure: HTTPS 时必须为 true
Max-Age: 不写入，由 Guard 管理服务端过期
```

UI Role Cookie：

```text
Name: dbx_guard_ui
Value: admin | viewer
Path: /dbx
HttpOnly: false
SameSite: Strict
Secure: HTTPS 时必须为 true
```

`dbx_guard_ui` 只用于界面；服务端不得从该 Cookie 判断权限。

默认 Session 策略：

- 空闲超时：8 小时。
- 绝对超时：24 小时。
- Guard 重启：全部 Guard Session 失效。
- DBX Web 重启：上游 Session 失效；Guard 使用服务凭据重新登录并重建上游 Session。
- 重新建立上游 Session 后，`save_password=false` 的临时数据源凭据不恢复，用户需要重新输入。

### 5.5 Cookie 与 Header 清洗

Guard 转发前必须：

1. 删除浏览器传入的 `dbx_session`。
2. 删除浏览器传入的 `X-DBX-Guard-*`、`X-Forwarded-*`。
3. 只注入当前 Guard Session 对应的上游 `dbx_session`。
4. 由 Guard 重新设置 `X-Forwarded-For`、`X-Forwarded-Proto`、`X-Forwarded-Host`。
5. 不向浏览器转发 DBX 的 `Set-Cookie: dbx_session=...`。

### 5.6 登录失败与密码冲突

- 管理员和普通用户密码必须不同；配置时若哈希校验发现相同，拒绝保存。
- 登录失败统一返回 401，不说明是否接近管理员密码或普通用户密码。
- 按客户端 IP 和全局两个维度限速。
- 连续失败阈值默认 5 次，锁定 60 秒；参数可配置。
- 日志只记录角色命中成功或认证失败，不记录密码和密码哈希。

## 6. 认证端点兼容协议

Guard 拦截 DBX 已有认证路径，使原前端无需修改：

| 路径 | Guard 行为 |
|---|---|
| `POST /dbx/api/auth/login` | 校验双角色密码，创建独立上游 Session 和 Guard Session |
| `GET /dbx/api/auth/check` | 返回 DBX 兼容字段 `authenticated/required/setup_required` |
| `POST /dbx/api/auth/logout` | 调用上游 logout，清除两类 Cookie 和映射 |
| `POST /dbx/api/auth/setup` | Guard 已初始化后固定返回 403 |
| `POST /dbx/api/auth/change-password` | 仅 admin 可调用，修改当前管理员 Guard 密码，不修改上游密码 |

兼容响应：

```json
{
  "authenticated": true,
  "required": true,
  "setup_required": false
}
```

登录响应继续返回：

```json
{
  "ok": true
}
```

角色不放入 API 响应，避免引入前端契约依赖。

## 7. 密码配置与轮换

### 7.1 初始配置

Guard 未完成以下三项配置时不得进入 ready：

```text
dbx-web-guard credential set-upstream
dbx-web-guard password set --role admin
dbx-web-guard password set --role viewer
```

所有命令必须交互输入；禁止支持 `--password <plaintext>` 参数。

### 7.2 管理员密码修改

支持两条路径：

- DBX 设置页原“修改密码”表单由 Guard 拦截，用于修改当前管理员 Guard 密码。
- 本机管理员可执行 `dbx-web-guard password set --role admin` 进行紧急重置。

修改后当前 admin Session 默认继续有效，其他 admin Session 全部失效。

### 7.3 普通用户密码修改

Viewer 不显示设置页，也不能调用修改密码 API。普通用户密码只能由本机管理员执行：

```text
dbx-web-guard password set --role viewer
```

修改后全部 Viewer Session 立即失效。

### 7.4 上游密码轮换

上游密码不是管理员登录密码。轮换必须使用协调命令：

```text
dbx-web-guard credential rotate-upstream
```

协调顺序：

1. 使用旧加密凭据登录 DBX。
2. 调用 DBX 原修改密码 API提交新值。
3. 使用新值重新登录验证。
4. 新值加密落盘并原子替换旧凭据。
5. 全部上游 Session 重新建立。

若步骤 2 成功但步骤 4 失败，命令必须明确返回部分成功并要求立即恢复；不得继续把 Guard 标记为 ready。

## 8. 角色模型

### 8.1 Admin

Admin 可以：

- 使用全部 DBX Web 页面和工具栏入口。
- 查看和修改设置。
- 检查更新、打开 GitHub、切换主题。
- 管理数据源、AI 配置、MCP 策略、白名单、驱动和同步配置。
- 使用 DBX 原有查询和 Agent 能力。

Admin 仍受 DBX 原有 SQL 权限、生产保护、危险 SQL 确认和数据源权限约束。Guard 不扩大这些权限。

### 8.2 Viewer

Viewer 可以：

- 登录 Web。
- 查看管理员已配置的数据源和允许的数据库对象。
- 打开查询、Database Docs、历史、AI Agent 和普通业务页面。
- 调用为日常使用所需的读取、连接、查询、AI SSE 和用户会话接口。
- 使用 DBX 已有 SQL 权限允许的能力。

Viewer 不可以：

- 打开或修改全局设置。
- 检查 DBX 或驱动更新。
- 打开 GitHub 外链。
- 修改主题或界面全局配置。
- 新增、编辑、删除、导入或导出数据源配置。
- 修改 AI Provider、API Key、Prompt 全局配置、MCP 策略或白名单。
- 安装、升级、删除驱动、插件或运行时。
- 修改 Web/Guard 密码。
- 调用云同步、配置导入或其他全局管理接口。

### 8.3 Guard 与数据库权限的边界

Guard 只负责 Web 管理面角色，不重新解析 SQL，也不替代 DBX 的 SQL 策略。Viewer 是否能执行 DML/DDL，继续由 DBX 数据源权限、连接只读配置、MCP/Agent 策略和危险 SQL 确认决定。

若需要“Viewer 无条件只读数据库”，应在独立版本中复用 DBX 的 SQL 分类器实现服务端强制策略；不得仅通过拦截 `POST /query/execute` 或字符串匹配 SQL 实现。

## 9. API 授权策略

### 9.1 策略原则

1. Admin 默认允许，仍受基础认证和请求安全校验。
2. Viewer 采用显式策略组，不按 HTTP POST 一刀切，因为读取查询和 AI SSE 同样使用 POST。
3. 未分类的新 API 对 Viewer 默认拒绝。
4. 每次升级根据路由快照生成差异报告；新路由没有明确归类时 Guard readiness 可正常，但 Viewer 调用返回 403 并产生审计告警。
5. WebSocket、SSE 和普通 HTTP 使用同一角色判断。

### 9.2 Viewer 必须拒绝的管理域

至少包含：

```text
/api/auth/setup
/api/auth/change-password
/api/update/**
/api/changelog
/api/app-settings/** 写方法
/api/cloud-sync/**
/api/agents/** 安装、升级、删除、导入、运行时变更
/api/jdbc/** 安装、导入、删除、卸载
/api/connection/** 配置保存、复制、删除、MCP 注册变更
/api/mcp-policy/** 写方法
/api/ai/configs/** 写方法
/api/prompt-templates/** 写方法
/api/layout/** 写方法
```

准确列表以 Guard 的版本化策略文件和 DBX 路由快照测试为准，不能只依赖本文示例。

### 9.3 403 契约

Viewer 命中受限接口时返回：

```http
HTTP/1.1 403 Forbidden
Content-Type: application/json
Cache-Control: no-store
```

```json
{
  "code": "WEB_GUARD_VIEWER_FORBIDDEN",
  "error": "This operation requires an administrator session."
}
```

响应不得暴露策略表达式、管理员密码状态、上游凭据或内部路径。

## 10. Viewer UI 注入

### 10.1 注入方式

Guard 对 `text/html` 的入口文档注入：

```html
<link rel="stylesheet" href="/dbx/__guard/viewer.css">
<script defer src="/dbx/__guard/bootstrap.js"></script>
```

静态 Guard 资源由 Guard 自身提供，不从 DBX `static/` 目录读取。资源带版本号和内容哈希缓存标识。

`bootstrap.js` 始终加载，并读取 `dbx_guard_ui`：

- `admin`：不增加 Viewer 限制 class。
- `viewer`：为根节点添加 `dbx-guard-viewer`，启动 `MutationObserver` 处理延迟渲染元素。
- 未登录：不显示主应用管理入口；登录成功后 UI Role Cookie 变化由 bootstrap 检测并立即应用。

### 10.2 必须隐藏的入口

Viewer 至少隐藏：

- 右上角检查更新。
- 主题切换。
- GitHub。
- 设置。
- 设置页、设置快捷键和从侧栏/上下文菜单进入设置的入口。
- 欢迎页 GitHub 链接。
- 与上述管理能力对应的二级入口。

### 10.3 交互防护

`bootstrap.js` 还应：

- 拦截已知设置快捷键。
- Viewer 访问设置页路由或动态打开设置面板时立即关闭并提示无权限。
- 捕获 403 `WEB_GUARD_VIEWER_FORBIDDEN`，显示统一提示。
- 不修改请求为 admin，不保存或读取 HttpOnly Session。

### 10.4 前端升级失效策略

每个 Guard 发布版本记录其验证过的 DBX 前端版本和选择器签名。启动不因单个选择器失效而停止，但必须：

1. 记录 `UI_POLICY_SELECTOR_MISS`。
2. 在健康详情中标记 `ui_policy=degraded`。
3. API 授权继续生效。
4. 发布验收必须重新检查四个目标按钮和所有二级入口。

## 11. 移动端支持边界

当前 DBX 主界面存在最小宽高和桌面工作区布局。Guard 外层不重写完整前端，因此 V1 移动端支持范围定义为：

- 登录。
- 浏览已配置连接。
- 打开 AI 面板。
- 提交问题、查看 SSE 流式回答和工具过程。
- 查看基础查询结果和错误提示。

V1 不承诺在 360px 宽度上完整操作 ER 图、复杂表格编辑、驱动管理和多窗格式化工作区。

Guard 注入的移动端 CSS 可以取消页面级固定最小宽度、优化 AI 面板为全屏，并隐藏非核心栏位；不能通过脆弱 CSS 强行重排全部 DBX 组件。

验收视口至少包括：

- 360×800。
- 390×844。
- 768×1024。
- 1366×768。
- 1920×1080。

## 12. `/dbx/` 子路径设计

### 12.1 DBX Web

DBX Web 启动参数：

```text
DBX_PORT=4225
DBX_PUBLIC_BASE_PATH=/dbx
DBX_DATA_DIR=<configured shared data directory>
```

Guard 模式下 DBX Web 只提供上游 API，不负责局域网静态入口；`DBX_STATIC_DIR` 可不设置。为本机诊断保留后端静态预览时，也只能指向经过版本校验的只读副本，且 4225 仍不得对局域网开放。路径和端口写入本机配置，由启动器读取；不得硬编码进源码。

### 12.2 Guard

Guard 配置：

```text
listen = 127.0.0.1:4226
public_base_path = /dbx
upstream = http://127.0.0.1:4225
static_dir = <configured frontend directory>
```

Guard 从配置的前端目录提供入口和静态资源，并完成 HTML 注入；API、SSE 和 WebSocket 请求向上游转发且保留 `/dbx`，因为 DBX Web 自身已按 `/dbx` 挂载。Nginx 不可直接绕过 Guard 读取该静态目录，否则 Viewer 注入无法保证。

### 12.3 Nginx

Nginx 在现有 `server` 中增加：

```nginx
location = /dbx {
    return 301 /dbx/;
}

location /dbx/ {
    proxy_pass http://127.0.0.1:4226;
    proxy_http_version 1.1;

    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection $connection_upgrade;

    proxy_buffering off;
    proxy_cache off;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_pass` 后不加尾随 `/`，确保 `/dbx` 前缀不被剥离。`$connection_upgrade` 需要在 `http` 块定义 `map`；若当前 Nginx 不使用 WebSocket，可先显式设置 `Connection ""`，但 SSE 仍必须关闭 buffering。

### 12.4 对外地址

目标局域网入口：

```text
http://<server-ip>:82/dbx/
```

生产长期使用建议启用内网 HTTPS：

```text
https://<server-name>/dbx/
```

## 13. 端口与防火墙

| 端口 | 组件 | 监听 | 局域网访问 |
|---|---|---|---|
| 82 或 443 | Nginx | LAN | 允许 Private/Domain 的 LocalSubnet |
| 4226 | Guard | `127.0.0.1` | 禁止 |
| 4225 | DBX Web | 当前代码为 `0.0.0.0` | Windows 防火墙显式禁止 |
| 4224 | Desktop/旧预览 | 按当前使用 | 不作为 Web 公网入口 |

防火墙原则：

1. 只允许局域网访问 Nginx 对外端口。
2. 4225、4226 不开放给 LocalSubnet。
3. 不使用“隐藏 URL”作为访问控制。
4. Guard 故障时不得临时把 Nginx 直接切到 4225。

## 14. DBX 配置、数据源与白名单共享

### 14.1 共享目标

以下持久数据需要 Desktop 和 Web 一致：

- 数据源定义与保存的凭据。
- AI Provider 配置、默认模型和 Prompt 模板。
- MCP 全局策略、连接白名单和 Agent 最大轮次等设置。
- Database Docs 注释与相关持久配置。

以下数据不要求跨进程即时共享：

- 进程内连接池。
- Guard Session 和 DBX Web Session。
- `save_password=false` 的临时连接密码。
- 正在执行的查询、SSE 流和临时上传文件。

### 14.2 V1 共享方式

V1 将 `D:\dbx-web\server\data` 作为本机 DBX 共享数据目录。实际路径从本机部署配置读取；代码和仓库文档不把它作为不可变常量。

- DBX Web 服务设置 `DBX_DATA_DIR` 指向该目录。
- Desktop 必须通过受控启动器或机器级配置设置相同的 `DBX_DATA_DIR`。
- Guard 不直接打开 `dbx.db`，只通过 DBX Web API工作。

### 14.3 并发限制

共享同一 SQLite 文件是本方案风险最高的部分，必须满足：

1. Desktop 与 Web 使用完全相同的 DBX 版本和数据库 Schema。
2. 升级时先停止 Desktop 和 DBX Web，再做离线备份和迁移。
3. DBX Web 先启动并完成 Storage 初始化，Desktop 后启动。
4. 不同时进行批量配置导入、云同步和另一端的大量设置保存。
5. 验收必须覆盖 Desktop/Web 并发读取、交替保存设置和锁超时。

如果出现持续 `database is locked`、迁移竞争或跨进程缓存不一致，立即回退到两个独立数据目录；随后通过独立、可审计的配置同步工具实现共享，不能用运行中的 SQLite 文件直接复制覆盖。

### 14.4 升级一致性

Desktop、DBX Web 和共享 `dbx.db` 作为一个版本单元升级。禁止只升级其中一个可执行文件后长期共用同一数据库。

## 15. 部署目录

目标布局：

```text
D:\dbx-web\
├── server\
│   ├── dbx-web.exe
│   ├── start-dbx-web.ps1
│   ├── config\
│   │   └── server.toml
│   ├── data\
│   │   └── dbx.db
│   ├── static\
│   └── logs\
├── guard\
│   ├── dbx-web-guard.exe
│   ├── config\
│   │   └── guard.toml
│   ├── secrets\
│   │   └── upstream.dpapi
│   ├── state\
│   │   └── credentials.db
│   └── logs\
├── service\
│   ├── dbx-web-service.xml
│   ├── dbx-web-guard-service.xml
│   └── nginx-service.xml
└── backup\

D:\nginx\
├── nginx.exe
├── conf\
│   └── nginx.conf
├── dbx\
│   ├── index.html
│   └── assets\
└── logs\
```

`D:\nginx\dbx` 是前端发布目录，但由 Guard 读取并返回，Nginx 的 `/dbx/` location 仍统一代理到 Guard。`credentials.db` 只保存 Guard 角色密码哈希、版本和轮换时间，不保存明文。Guard 日志、状态和密钥目录必须加入源码仓库 ignore，并设置 NTFS ACL。

## 16. Guard 配置

示例仅展示非机密字段：

```toml
[server]
listen = "127.0.0.1:4226"
public_base_path = "/dbx"

[upstream]
base_url = "http://127.0.0.1:4225"
credential_file = "../secrets/upstream.dpapi"
connect_timeout_seconds = 5
request_timeout_seconds = 300
sse_timeout_seconds = 3600

[static]
directory = "<configured frontend directory>"
index_file = "index.html"

[session]
idle_timeout_minutes = 480
absolute_timeout_minutes = 1440
cookie_name = "dbx_guard_session"
ui_cookie_name = "dbx_guard_ui"

[security]
allowed_origins = ["http://<server-ip>:82"]
trust_proxy = true
viewer_policy = "viewer-policy-v1"

[ui]
dbx_version = "<package-version>"
selector_policy = "viewer-ui-v1"
```

安装脚本负责把机器地址和目录写入本机配置；源码不包含机器特定地址和路径默认值。

## 17. Guard 实现边界

### 17.1 独立扩展工程

Guard 使用独立 Rust 工程构建，建议路径：

```text
extensions/dbx-web-guard/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── auth.rs
│   ├── credential.rs
│   ├── session.rs
│   ├── policy.rs
│   ├── proxy.rs
│   ├── inject.rs
│   ├── health.rs
│   └── audit.rs
├── assets/
│   ├── bootstrap.js
│   └── viewer.css
└── tests/
```

该工程使用自己的 Cargo workspace 边界，不修改 DBX 根 workspace 成员，不依赖 `dbx-core`，不直接访问 DBX SQLite。

### 17.2 允许修改

- 新增独立 Guard 源码和测试。
- 新增本机部署模板、服务模板和构建脚本。
- 修改 Nginx 本机配置。
- 修改 `D:\dbx-web` 下启动和部署配置。
- 在项目文档中记录部署规范。

### 17.3 禁止修改

默认禁止为了 Guard 修改：

```text
crates/dbx-core/
crates/dbx-web/src/auth.rs
crates/dbx-web/src/routes/
apps/desktop/src/
src-tauri/
DBX SQL 权限和 Agent Loop
```

如果未来 DBX 上游删除 `/dbx` 支持或认证契约发生不兼容变化，必须先修订本文并评审，不允许在部署现场做不可追踪补丁。

### 17.4 DBX Web 源码零修改验收

以当前工作区为实施基线时，`dbx-web` 已具备本方案需要的 `/dbx` 上下文路径、原单密码登录、Cookie Path 和 AI Agent API，因此 Guard 实施不得修改现有 DBX Web 源码。

交付验收必须证明：

- `crates/dbx-web/` 没有因 Guard 产生源码差异。
- `crates/dbx-core/`、`apps/desktop/` 和 `src-tauri/` 没有因 Guard 产生源码差异。
- DBX Web 的变化仅限启动配置、端口、数据目录和部署文件。
- 双角色、DPAPI、Session 映射、UI 注入和 403 策略全部位于独立 Guard。

若实施发现当前发布二进制不包含已确认的 `/dbx` 能力，应重新构建当前工作区的 DBX Web，而不是为 Guard 临时修改认证或业务源码。

## 18. 代理与流式传输

Guard 必须支持：

- 普通 HTTP 请求和响应流式转发。
- 从配置目录安全提供 DBX 前端静态资源，拒绝路径穿越和目录列表。
- SSE：不缓存、不压缩重组、不等待完整响应。
- WebSocket Upgrade 透传。
- 大文件上传的大小和超时限制。
- 客户端断开后取消上游请求。
- 不在日志记录请求正文、SQL 全文、AI Prompt、密码或 Token。

对 `text/html` 仅注入入口文档；不得修改 JSON、SSE、JavaScript bundle、下载文件或压缩响应。

## 19. CSRF、Origin 与浏览器安全

Guard 对带副作用的方法执行：

1. 校验 `Origin` 属于配置 allowlist。
2. Origin 缺失时按浏览器导航/非浏览器客户端策略处理，不默认放行跨站请求。
3. 校验 Guard Session。
4. 应用角色策略。
5. 限制 Content-Type 和正文大小。

HTTPS 部署时增加：

- `Secure` Cookie。
- HSTS。
- `Content-Security-Policy`，允许 DBX 自身资源和 Guard 注入资源。
- `X-Content-Type-Options: nosniff`。
- `Referrer-Policy: same-origin`。
- `frame-ancestors 'none'`。

HTTP 内网阶段必须记录为过渡状态；密码会话不应跨不可信无线网络使用。

## 20. 健康检查

健康端点仅允许本机访问：

```text
GET /__guard/health/live
GET /__guard/health/ready
```

它们不返回 Session 或角色信息。

`live` 只表示进程存活。`ready` 需要同时满足：

- Guard 配置有效。
- 两个角色哈希存在且不同。
- 上游 DPAPI 凭据可解密。
- DBX `/dbx/api/auth/login` 可用且上游密码有效。
- Viewer policy 已加载。
- 注入资产完整性校验通过。

Nginx 外部 `/dbx/` 不直接暴露健康详情。

## 21. Windows 服务与开机自启

### 21.1 服务模型

使用 WinSW 或等价 Windows Service Wrapper，把三个进程注册为独立服务：

| 服务 | 启动类型 | 失败恢复 | 依赖 |
|---|---|---|---|
| `DBXWebBackend` | Automatic | 5 秒后重启，限次后告警 | 无 |
| `DBXWebGuard` | Automatic (Delayed) | 5 秒后重启 | `DBXWebBackend` |
| `DBXNginx` | Automatic (Delayed) | 5 秒后重启 | `DBXWebGuard` |

若不引入 Service Wrapper，允许使用计划任务作为过渡方案，但正式验收仍要求进程退出后能自动恢复且日志、工作目录和依赖顺序稳定。

### 21.2 服务身份

- DBX Web 和 Guard 使用受限本地服务身份。
- Guard 身份必须能读取 `secrets/upstream.dpapi`，不能写 Nginx 配置和 DBX 可执行文件。
- Nginx 身份只能读取配置、静态文件和写日志。
- 普通局域网用户没有部署目录文件权限。

### 21.3 启动顺序

```text
1. DBX Web 打开共享 dbx.db 并完成 migration
2. DBX Web /dbx/api/auth/check 可达
3. Guard 验证上游服务密码并进入 ready
4. Nginx 开始对外提供 /dbx/
```

Guard 不因 DBX 短暂未启动而退出；采用有上限的指数退避重试。Nginx 不直接回退到 DBX 后端。

## 22. 日志与审计

Guard 审计至少记录：

- 时间、请求 ID、客户端 IP 哈希或受控地址。
- 登录成功角色、登录失败、锁定。
- Session 创建、续期、过期和注销。
- Viewer 403 的策略 ID、方法和规范路径。
- 上游重新登录、上游不可用。
- UI 选择器降级。
- 配置或密码轮换操作的结果。

禁止记录：

- 三类密码及其哈希。
- Cookie、Session Token。
- API Key、数据库密码、Authorization Header。
- 完整 SQL、AI Prompt、模型响应和数据库结果。

默认日志滚动：单文件 20 MiB，保留 14 天，总量上限 500 MiB。

## 23. 构建与打包

### 23.1 独立构建

Guard 独立构建：

```text
cargo build --release --manifest-path extensions/dbx-web-guard/Cargo.toml
```

DBX Web 继续使用当前仓库 Rust 构建。前端使用 `/dbx/` 基础路径生成后复制到配置的 `D:\nginx\dbx` 发布目录，由 Guard 读取并提供给浏览器。

### 23.2 发布包

本机发布包至少包含：

```text
dbx-web.exe
dbx-web-guard.exe
frontend static assets
non-secret config templates
Windows service templates
install/update/rollback/verify scripts
artifact-manifest.json
```

`artifact-manifest.json` 记录版本、构建时间、SHA-256 和兼容的 DBX/Guard/UI policy 版本，不包含机密。

### 23.3 加速构建

- Guard 与 DBX Web 分开构建；只改 Guard 时不重编 Tauri Desktop。
- Rust 使用增量缓存和稳定 target cache。
- 前端未变化时复用经过 SHA-256 验证的静态产物。
- Desktop EXE/MSI/NSIS 与 Web/Guard 发布分开，只有共享核心或前端变化时才触发完整 Tauri 打包。
- 发布阶段始终重新计算最终文件哈希，不能复用历史哈希或时间戳。

## 24. 安装与迁移

### 24.1 首次安装

1. 核对 DBX Desktop、DBX Web 版本一致。
2. 停止旧 DBX Web；确认 Desktop 不在写配置。
3. 离线备份 `data`。
4. 部署 DBX Web、Guard、静态资源和服务模板。
5. 写入非机密配置。
6. 交互配置上游服务密码、管理员密码、Viewer 密码。
7. 配置 Desktop 使用共享 `DBX_DATA_DIR`。
8. 配置 Nginx `/dbx/`。
9. 配置防火墙。
10. 注册并启动 Windows 服务。
11. 完成 Admin、Viewer、移动端和直接端口绕过验收。

### 24.2 现有数据迁移

- 保留现有 `D:\dbx-web\server\data\dbx.db`。
- 不用在线文件复制覆盖正在运行的 SQLite。
- 若从 Desktop 默认目录迁移，以停止双方后的离线副本为基线，并在启用共享目录前保留源备份。
- 迁移后校验文件 hash、SQLite integrity、连接数量、AI 配置数量和策略记录；不打印敏感字段。

## 25. 升级流程

1. 进入维护窗口，阻止新登录。
2. 停止 Nginx、Guard、DBX Web，并确认 Desktop 已退出。
3. 对共享 data 做离线备份。
4. 把新文件复制到同目录临时文件，核对 hash 后原子替换。
5. 先启动 DBX Web并完成 migration。
6. 启动 Guard，验证上游认证和 policy。
7. 启动 Nginx。
8. 运行 Admin/Viewer/API/SSE/移动端 smoke。
9. 观察稳定窗口后才删除旧二进制备份。

升级不得删除 `data/`、`guard/secrets/`、`guard/state/` 和本机配置。

## 26. 回滚

### 26.1 Guard 回滚

- 停止 Nginx 和 Guard。
- 恢复上一版 Guard EXE、策略和注入资产。
- 不回滚 DBX 数据。
- 验证上游凭据仍可解密和登录。

### 26.2 DBX Web 回滚

只有数据库 Schema 与旧版兼容时才能单独回滚可执行文件。若 migration 不可逆，必须同时恢复升级前离线 data 备份。

### 26.3 紧急旁路

不允许把 Nginx 临时直连 4225。紧急恢复只能：

- 修复或回滚 Guard；或
- 由本机管理员在服务器本机访问受原密码保护的后端进行诊断。

局域网防火墙规则不得因 Guard 故障而放开。

## 27. 测试设计

### 27.1 Guard 单元测试

- admin/viewer 哈希命中和密码相同拒绝。
- 登录失败限速与统一错误。
- Guard Session 超时和绝对超时。
- Cookie Path `/dbx`、HttpOnly、SameSite、Secure。
- 客户端 Cookie/Header 清洗。
- Viewer policy allow/deny/default-deny。
- 403 JSON 契约。
- Origin/CSRF 校验。
- DPAPI 加密、解密和 ACL 错误。
- HTML 注入仅发生在入口 HTML。
- SSE 不缓冲。

### 27.2 集成测试

- 一个 admin 和两个 viewer 同时登录，三者获得不同上游 Session。
- 伪造 `dbx_guard_ui=admin` 后 Viewer API 仍为 403。
- 浏览器直接提交伪造 `dbx_session` 被剥离。
- DBX 重启后 Guard 重建上游 Session。
- Guard 重启后全部用户重新登录。
- 上游密码错误时 readiness 失败且不泄密。
- Nginx `/dbx` 重定向和 `/dbx/` 资源/API/SSE 正常。
- 前端升级导致选择器失效时 API 仍拒绝。
- Desktop 与 Web 共享 data 的交替保存和锁竞争测试。

### 27.3 安全测试

- 未认证访问 API 返回 401。
- Viewer 访问管理 API 返回 403。
- admin 可访问管理 API。
- 4225/4226 从局域网不可达。
- 82/443 仅 LocalSubnet 可达。
- 跨 Origin 副作用请求拒绝。
- 日志扫描不包含密码、Cookie、Token 或完整配置 JSON。

### 27.4 UI 验收

Viewer：

- 检查更新、主题、GitHub、设置不显示。
- 快捷键和二级入口不能打开设置。
- 强制显示 DOM 后，调用仍为 403。
- AI Agent 可正常流式使用。

Admin：

- 四个入口全部显示并可操作。
- 设置页可打开。
- 原修改密码表单修改的是 Guard admin 密码。
- Viewer 密码可由本机管理命令修改。

## 28. 验收标准

1. 局域网可通过 `http://<server-ip>:82/dbx/` 访问，根路径和旧 `/api/` 不受影响。
2. 4225、4226 不能从另一台局域网机器直接访问。
3. Admin 与 Viewer 密码不同且均不明文落盘。
4. 固定上游服务密码已通过交互命令加密配置，并能登录 DBX。
5. 每个 Guard Session 对应独立 DBX 上游 Session。
6. 浏览器永远拿不到上游 `dbx_session`。
7. Viewer 四个指定入口和设置二级入口全部隐藏。
8. Viewer 直接调用受限 API稳定返回 403。
9. 伪造 UI Cookie、禁用 CSS/JS 或修改 DOM 不提升权限。
10. Admin 使用全部原有功能，SQL 权限不被 Guard 放宽。
11. SSE AI Agent 连续运行 30 分钟无代理缓冲或超时。
12. DBX Web、Guard、Nginx 开机自动启动并按依赖恢复。
13. 服务重启、密码错误和 Guard 故障均不产生后端旁路。
14. Desktop/Web 共享配置通过并发和版本一致性测试；不通过时启用独立目录回退。
15. PC 和定义范围内的移动端 AI Agent 使用通过。
16. 日志和发布物不包含明文密码、Token、数据库凭据或完整 AI 配置。
17. `crates/dbx-web/`、`crates/dbx-core/`、`apps/desktop/` 和 `src-tauri/` 没有因 Guard 产生源码修改。

## 29. 分阶段实施

### G0：基线与可回退部署

- 固化端口、目录和配置模板。
- DBX Web 切换到 4225 和 `/dbx`。
- Nginx 增加 `/dbx/`，暂不对局域网放行。
- 完成备份与回滚脚本。

退出条件：本机通过 Nginx 可以访问原单密码 DBX Web。

### G1：Guard 认证与上游会话

- 实现三类凭据、双角色登录、DPAPI 和一对一上游 Session。
- 拦截 DBX auth 接口。
- 完成 Cookie/Header 清洗、限速和健康检查。

退出条件：Admin/Viewer 登录、注销、DBX 重启恢复和无上游 Cookie 泄漏通过。

### G2：Viewer API 策略

- 建立 DBX 路由快照。
- 实现 Viewer allow/deny/default-deny。
- 完成 403 契约、审计和 CSRF。

退出条件：管理接口绕过测试全部失败关闭，日常 AI/查询流程可用。

### G3：UI 与移动端

- 注入 bootstrap、Viewer CSS 和移动端 AI 布局。
- 覆盖四个指定入口、快捷键和二级入口。
- 增加选择器签名和降级告警。

退出条件：五类视口和 DOM 绕过测试通过。

### G4：共享数据与服务化

- 配置 Desktop/Web 共享数据目录。
- 完成 SQLite 并发测试。
- 注册三个 Windows 服务及依赖。
- 配置防火墙和局域网入口。

退出条件：重启机器后自动恢复，配置共享与绕过测试通过。

### G5：发布验收

- 生成 artifact manifest、hash 和回滚包。
- 执行完整验收清单。
- Review 通过后把本文升级为 Active。

## 30. 风险与缓解

| 风险 | 缓解 |
|---|---|
| Guard 变成单点故障 | Windows 服务恢复、健康检查、独立回滚；禁止后端旁路 |
| 两角色共用上游 Session 泄漏临时凭据 | 每个 Guard Session 独立换取 DBX Session |
| 上游固定密码写入脚本或日志 | 交互输入、DPAPI、ACL、日志脱敏 |
| Viewer CSS 选择器随前端升级失效 | 版本化选择器、降级告警、API 403 为真实边界 |
| 伪造 UI Role Cookie | 服务端只信 HttpOnly Guard Session |
| Nginx 剥离 `/dbx` 导致资源/API 404 | `proxy_pass` 无尾斜杠，端到端子路径测试 |
| SSE 被代理缓存或超时 | Guard 流式透传，Nginx buffering off，长 read timeout |
| DBX 后端端口被局域网绕过 | 4225 防火墙拒绝，Nginx 只代理 Guard |
| Desktop/Web 同时写 SQLite 锁竞争 | 同版本、顺序启动、维护窗口、并发验收和独立目录回退 |
| Admin 设置页修改了错误密码层 | Guard 拦截 change-password 并明确只改 admin；上游用专用协调命令 |
| 新 DBX API 未纳入 Viewer 策略 | 路由快照差异、Viewer 默认拒绝、升级验收 |
| 移动端被误认为完整桌面等价 | 明确 V1 AI 使用范围和视口验收，复杂编辑不作虚假承诺 |

## 31. 已决策事项

- 局域网通过 Nginx `/dbx/` 访问。
- Nginx、Guard、DBX Web 是三个独立 Windows 服务。
- Guard 不作为 Nginx 子进程。
- Guard 不增加 `/guard/session`。
- Guard 接管浏览器登录兼容路径，保留 DBX 原单密码认证。
- 上游服务密码固定并在部署时预配置，使用 DPAPI 加密，不在文档和代码记录明文。
- Admin 和 Viewer 使用两个不同的 Guard 密码。
- 每个 Guard Session 创建独立 DBX 上游 Session。
- Viewer UI 使用 CSS/JS 注入收敛，API 403 是唯一权限边界。
- Guard 作为独立扩展工程，不修改 DBX 原认证、路由、Agent 和数据库权限源码。
- Web 后端部署目录保持 `D:\dbx-web\server`，不使用 Docker。
- V1 共享 DBX 数据目录，但必须通过并发验收并保留独立目录回退方案。

## 32. 实施前置检查表

- [ ] Guard 独立工程目录和构建边界评审通过。
- [ ] Admin、Viewer 初始密码已由用户分别确定。
- [ ] 上游服务密码已通过交互命令验证和加密保存。
- [ ] Nginx 当前 82 端口其他 location 回归清单已建立。
- [ ] 4225/4226 防火墙规则草案已评审。
- [ ] DBX Web 与 Desktop 版本一致。
- [ ] 共享 `dbx.db` 已离线备份。
- [ ] Viewer 路由策略已从当前源码生成快照。
- [ ] UI 选择器已基于当前发布静态文件验证。
- [ ] Windows 服务身份和目录 ACL 已确定。
- [ ] 回滚包和恢复命令已在测试目录演练。
