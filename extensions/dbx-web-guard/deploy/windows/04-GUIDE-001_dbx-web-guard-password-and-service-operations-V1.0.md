---
title: "DBX Web Guard 密码配置与服务启动手册"
doc_id: "04-GUIDE-001"
version: "V1.0"
status: "Active"
created_date: "2026-08-20"
last_updated: "2026-08-20"
maintainer: "DBX DB-Wiki Project"
constraint_level: "Instructional"
review_cycle: "On-demand"
related_docs:
  - "01-DESIGN-004"
tags:
  - "DBX-Web"
  - "Guard"
  - "Windows"
  - "Operations"
---

# DBX Web Guard 密码配置与服务启动手册

## 1. 最终端口与访问地址

| 组件 | 端口 | 访问范围 |
|---|---:|---|
| Nginx | 4223 | 局域网入口 |
| DBX Web | 4225 | 内部后端，不允许局域网直连 |
| DBX Web Guard | 4226 | 仅 `127.0.0.1` |

最终浏览器地址：

```text
http://192.168.10.170:4223/
```

不使用 `/dbx` 路径。旧版 DBX Web 的 `4224` 端口应在切换时关闭。

## 2. 目录结构

```text
D:\dbx-web\server\
  dbx-web.exe
  data\dbx.db
  logs\

D:\dbx-web\guard\
  dbx-web-guard.exe
  start-dbx-web-guard.ps1
  config\guard.toml
  secrets\upstream.dpapi
  state\credentials.db
  logs\

D:\nginx\
  nginx.exe
  conf\nginx.conf
  dbx\
```

## 3. 三类密码

| 密码 | 用途 | 保存位置 |
|---|---|---|
| Guard 管理员密码 | 浏览器管理员登录 | `D:\dbx-web\guard\state\credentials.db`，Argon2 哈希 |
| Guard 普通用户密码 | 浏览器普通用户登录 | `D:\dbx-web\guard\state\credentials.db`，Argon2 哈希 |
| DBX 上游密码 | Guard 登录内部 DBX Web | `D:\dbx-web\guard\secrets\upstream.dpapi`，DPAPI 加密 |

禁止把任何密码写入 `guard.toml`、PowerShell 脚本、Windows 服务参数或本文档。

管理员密码与普通用户密码必须不同。上游密码必须与 DBX Web 当前登录密码一致。

## 4. 首次配置密码

打开 PowerShell，依次执行。程序会关闭控制台回显，并要求输入两次。

### 4.1 管理员密码

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" `
  --config "D:\dbx-web\guard\config\guard.toml" `
  password set --role admin
```

### 4.2 普通用户密码

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" `
  --config "D:\dbx-web\guard\config\guard.toml" `
  password set --role viewer
```

### 4.3 DBX 上游密码

配置上游密码前，DBX Web 必须已在 `4225` 启动，并能访问 `/api/auth/login`。

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" `
  --config "D:\dbx-web\guard\config\guard.toml" `
  credential set-upstream
```

Guard 会先登录 DBX Web 验证密码，成功后才写入 DPAPI 文件。

## 5. 修改密码

管理员或普通用户密码重置仍使用 `password set` 命令。修改后对应角色的旧会话失效。

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" --config "D:\dbx-web\guard\config\guard.toml" password set --role admin
& "D:\dbx-web\guard\dbx-web-guard.exe" --config "D:\dbx-web\guard\config\guard.toml" password set --role viewer
```

DBX Web 原密码发生变化后，必须重新执行：

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" --config "D:\dbx-web\guard\config\guard.toml" credential set-upstream
```

## 6. 启动前配置检查

`D:\dbx-web\guard\config\guard.toml` 必须包含：

```toml
[server]
listen = "127.0.0.1:4226"
public_base_path = "/"

[upstream]
base_url = "http://127.0.0.1:4225"

[security]
allowed_origins = ["http://192.168.10.170:4223", "http://127.0.0.1:4223"]
```

Viewer 策略中的路径必须以 `/api/` 开头，不能残留 `/dbx/api/`。

执行完整检查：

```powershell
& "D:\dbx-web\guard\dbx-web-guard.exe" `
  --config "D:\dbx-web\guard\config\guard.toml" `
  check
```

成功输出：

```text
guard configuration is ready
```

## 7. 手工启动顺序

首次联调按以下顺序启动。

### 7.1 启动 DBX Web 后端

```powershell
& "E:\workspace\dbx-db-wiki-feature-wiki\extensions\dbx-web-guard\deploy\windows\start-dbx-web.ps1" `
  -ServerRoot "D:\dbx-web\server" `
  -Port 4225 `
  -PublicBasePath "/"
```

验证：

```powershell
Invoke-WebRequest "http://127.0.0.1:4225/api/auth/check" -UseBasicParsing
```

### 7.2 启动 Guard

另开 PowerShell：

```powershell
& "D:\dbx-web\guard\start-dbx-web-guard.ps1" -GuardRoot "D:\dbx-web\guard"
```

验证：

```powershell
Invoke-RestMethod "http://127.0.0.1:4226/__guard/health/live"
Invoke-RestMethod "http://127.0.0.1:4226/__guard/health/ready"
```

### 7.3 校验并启动 Nginx

```powershell
& "D:\nginx\nginx.exe" -p "D:\nginx" -t -c "conf/nginx.conf"
```

配置检查通过后：

```powershell
& "D:\nginx\nginx.exe" -p "D:\nginx" -c "conf/nginx.conf"
```

Nginx 已运行时使用：

```powershell
& "D:\nginx\nginx.exe" -p "D:\nginx" -s reload
```

## 8. Windows 服务启动

正式运行使用三个独立服务：

```text
DBXWebBackend -> DBXWebGuard -> DBXNginx
```

查看状态：

```powershell
Get-Service DBXWebBackend, DBXWebGuard, DBXNginx
```

启动：

```powershell
Start-Service DBXWebBackend
Start-Service DBXWebGuard
Start-Service DBXNginx
```

停止时使用相反顺序：

```powershell
Stop-Service DBXNginx
Stop-Service DBXWebGuard
Stop-Service DBXWebBackend
```

重启：

```powershell
Restart-Service DBXWebBackend
Restart-Service DBXWebGuard
Restart-Service DBXNginx
```

服务尚未注册时，使用仓库提供的 `New-DbxWebGuardServices.ps1` 生成并安装 WinSW 服务。该操作需要管理员 PowerShell 和本机 WinSW 可执行文件。

## 9. 最终验证

```powershell
Get-NetTCPConnection -State Listen | Where-Object LocalPort -in 4223,4224,4225,4226
```

预期：

- `4223`：Nginx 监听。
- `4224`：无旧版 DBX Web 监听。
- `4225`：DBX Web 后端监听，但防火墙禁止局域网直连。
- `4226`：Guard 仅监听 `127.0.0.1`。

浏览器验证：

```text
http://192.168.10.170:4223/
```

- 管理员密码登录：显示检查更新、主题、GitHub 和设置。
- 普通用户密码登录：上述入口不显示；直接调用管理 API 返回 403。

## 10. 常见错误

| 错误 | 原因与处理 |
|---|---|
| `both admin and viewer passwords must be configured` | 先配置两个角色密码 |
| `UPSTREAM_AUTH_FAILED` | 上游密码错误，或 DBX Web 未在 4225 启动 |
| `DPAPI credential storage is only supported on Windows` | 必须在 Windows 主机执行 |
| `WEB_GUARD_UPSTREAM_ERROR` | 检查 DBX Web 4225 和 Guard 日志 |
| `WEB_GUARD_VIEWER_FORBIDDEN` | 当前是普通用户会话，操作需要管理员 |
| Nginx 502 | Guard 未启动或未 ready |
| 页面资源 404 | 检查 `public_base_path="/"` 和静态目录 |

## 11. 日志

```text
D:\dbx-web\server\logs\stdout.log
D:\dbx-web\server\logs\stderr.log
D:\dbx-web\guard\logs\stdout.log
D:\dbx-web\guard\logs\stderr.log
D:\nginx\logs\error.log
```

排错时禁止打印 `credentials.db`、`upstream.dpapi`、Cookie、数据库密码或完整 AI 配置。
