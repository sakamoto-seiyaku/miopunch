## Context

POC 当前已具备：

- `miopunch up` 常驻进程；
- LocalAPI v0（IPC-only）：`/api/v0/*` 资源与 task、SSE（snapshot-first）、`sh_attach` WebSocket（`miopunch.sh.v0`）；
- 统一的可解释性输出（`stage/reason_code/exit_code` + facts/suggestions）与 task Markdown report。

但实际演示/排障仍以 CLI 为主；对“只想看状态/跟随 join 进度/一键进入 shell”的场景，CLI 的门槛与可视化表达不足。POC-07 的目标是在不改变既有 LocalAPI 契约与输出口径的前提下，提供一个仅本机可访问的最小 HTTP 面板，把“读取状态 + 少量写操作 + shell attach”做成可点击 UI。

关键约束（POC 口径）：

- 面板默认关闭；启用后只监听 `127.0.0.1`（固定默认 `127.0.0.1:27400`）。
- 面板只做“本机辅助 UI”，不替代 CLI；写操作严格白名单。
- 实时刷新只用 SSE（不做轮询 fallback）。
- 前端交付为内置静态资源（`go:embed`），不引入 SPA 框架，不依赖外网 CDN。
- 安全边界以 loopback + 同源校验 + 写白名单为主；`ui_token` 仅预留扩展点（后置）。

## Goals / Non-Goals

**Goals:**

- 提供本机面板页面（单页，固定 Tab）：`Status / Invite / Join / Shell`。
- 统一 SSE 驱动刷新：状态/任务列表/任务阶段机推进可实时更新。
- 面板只允许创建 `invite` / `join` / `sh_attach` 三类 task，并把其阶段/诊断/报告以卡片形式呈现。
- Shell tab 通过 `sh_attach` WebSocket + `xterm.js` 提供最小可用交互（连接、resize、断线提示）。
- 复用既有 task Markdown report，并提供“下载/复制”入口（不再定义第二套导出语义）。

**Non-Goals:**

- 不开放 `approve/ping/revoke/sh_ls` 等其它写操作（面板提示“请用 CLI”）。
- 不提供登录体系、用户体系、权限管理、多 operator 支持。
- 不支持非 loopback 监听；不在 POC 引入 `ui_token`/CSRF token 的强制要求。
- 不实现“轮询模式”作为 SSE 不可用时的 fallback。
- 不新增 report 的脱敏/留存规则（沿用既有 report/export 能力）。

## Decisions

1. **进程模型：与 `miopunch up` 同进程启动**
   - 面板作为 `miopunch up` 的可选子服务；启用后与 LocalAPI（IPC-only）并行监听。
   - 共享同一套 task 管理与事件流（避免“UI 再造一个状态机/输出口径”）。

2. **监听与路由：面板 HTTP listener + `/api/v0` 复用**
   - 面板 listener 仅允许绑定到 loopback（`127.0.0.1`）；默认端口 `27400`，端口占用则直接报错并提示用户改配置。
   - 面板页面走 `/` 与静态资源路径（例如 `/assets/...`），API 统一走 `/api/v0/...`（与 LocalAPI v0 路径对齐，减少重复协议设计）。
   - 面板 API 的读路径（status/peers/tasks/events/report）复用既有 task/event/report 结构；写路径仅允许创建白名单 task kind。

3. **前端交付：内置静态资源（KISS）**
   - 静态资源通过 `go:embed` 随二进制分发，避免外部依赖与部署步骤。
   - 终端渲染采用 `xterm.js`（本地打包/内置其产物，不依赖 CDN）；只实现最薄 glue（WS 连接 + resize + 断线提示）。
   - 二维码（如 invite code）由前端 JS 生成（内置极小 qrcode 库；不依赖外网）。

4. **写操作安全边界：同源校验 + 白名单**
   - 对 `POST /api/v0/tasks` 与 `GET /api/v0/tasks/<task_id>/ws` 强制同源校验（基于 `Origin`/`Referer` 与 configured `listen_addr` 的 host:port）。
   - 兼容性：当 `listen_addr` 为 `127.0.0.1:<port>` 时，同源校验同时接受 `http://localhost:<port>`（等价 origin），便于用户通过 `localhost` 打开面板。
   - 仅允许创建 `invite/join/sh_attach` 三类 task；其它 kind 返回可操作错误（提示使用 CLI）。
   - `ui_token` 作为后置扩展点：仅当未来显式支持非 loopback 监听时引入强制 token。

## Risks / Trade-offs

- **[Risk] 浏览器同源/Origin 细节差异** → Mitigation：仅对写操作与 WS 强制校验；错误提示明确要求“从面板页面发起请求”，并记录实际 `Origin/Referer` 便于排障。
- **[Risk] 内置前端资源引入体积与许可证管理成本** → Mitigation：选用最小依赖集合、固定版本、把第三方 license 归档到仓库既有许可证体系（`LICENSES/`）。
- **[Risk] `/api/v0` 在 TCP(loopback) 与 IPC(LocalAPI) 同时存在会引起概念混淆** → Mitigation：文档与输出明确区分“LocalAPI=IPC-only；HTTP 面板=loopback-only”，并避免把面板暴露为通用 API（写白名单 + 同源校验）。
