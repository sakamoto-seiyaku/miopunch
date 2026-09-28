# Cross-platform 客户端壳（Door 1）选型调研（LocalAPI-only，排除 Electron）

## 背景：我们仓库“已经有什么”

本调研只讨论“客户端壳（GUI/安装/托管）”，不扩展打洞/协议语义。

**硬约束（本调研前提）**：客户端与 daemon 的交互 **只走 `LocalAPI`**（IPC：unix socket / Windows named pipe），不依赖 `HTTP panel` 的 loopback HTTP。

- `LocalAPI`：`internal/localapi`
  - 传输：
    - Linux：unix socket（system：`/run/miopunch/localapi.sock`；user：`$XDG_RUNTIME_DIR/miopunch/localapi.sock`）
    - Android（D1b，后续）：unix socket（路径策略 TBD；可能需要显式指定 `--localapi unix:<path>`）
    - Windows：named pipe（`\\\\.\\pipe\\miopunch\\localapi-<operator_sid>`，DACL 仅允许 `{LocalSystem}+{operator user}`）
  - 语义：HTTP/JSON + SSE + WS（shell attach 需要 WS subprotocol：`miopunch.sh.v0`）
  - 安全边界：固定 `Host`（`internal/poc/localapi.go`：`local-miopunch.localapi`）+ OS 权限（socket mode / pipe DACL）
- `HTTP panel`：`internal/http_panel`
  - 说明：它仍可作为“浏览器调试/演示面板”，但 **不作为 Door 1 客户端的技术基线**。

## 约束与评估维度

- 硬约束：
  - 排除 `Electron`。
  - 客户端对 daemon **必须走 LocalAPI（IPC）**，不新增“loopback HTTP 作为主通道”。
- 目标：
  - “功能即可”：把已验证的 POC 能力收束成可用客户端。
  - 最终覆盖：`Windows / Linux / Android`（Android 端以“控制端”为主，常驻语义另行评估）。
  - 阶段策略（与 `docs/roadmap.md` 对齐）：先做桌面端 `D1a`（Wails 覆盖 `Linux/Windows`），Android 作为 `D1b` 后续推进。
- 评估维度（用于快速淘汰）：
  - **LocalAPI 适配成本**：能否稳定支持 unix socket + Windows named pipe（以及 Android 的 socket 路径策略）
  - **交互式 shell**：能否承载 `sh_attach` 的 WS 二进制字节流（或是否允许先用“外部终端/外部 CLI”兜底）
  - **发行与依赖**：安装包体积、运行时依赖、更新策略、daemon 托管与权限提示（operator/ACL）
  - **空载开销**：冷启动时间、idle RAM

## 方案分型（结合 LocalAPI 约束）

### 方案 A：Go 原生 UI（UI 进程直连 LocalAPI）

做法：

- 客户端（GUI）用 Go 写，直接复用 `internal/localapi.Client` 通过 unix socket / named pipe 调用：
  - `GET /api/v0/status|peers|tasks`
  - `GET /api/v0/events`（SSE）
  - `POST /api/v0/tasks`（invite/join/approve/ping/sh_ls/sh_attach/revoke_member）
  - `GET /api/v0/tasks/{id}/ws`（WS：`miopunch.sh.v0`）

候选：

- `Fyne` / `Gio`（Go 跨平台 UI）

优点：

- LocalAPI 适配成本最低（Go 侧已实现 unix socket / npipe dialer）。
- 不引入“第二门语言工具链”。

主要风险：

- GUI 内的 terminal/TTY 体验：如果要“内嵌交互式 shell”，需要 terminal 组件；否则可先用“拉起外部终端执行 `miopunch sh <peer>`”做兜底（该 CLI 本身也是 LocalAPI client）。

### 方案 B：WebView UI + Go 后端（特例：允许 Wails）

做法：

- UI 用 Web 技术（MD3/xterm/QR 都容易复用），但 **不直接访问 LocalAPI**（浏览器 JS 无法直连 unix socket / named pipe）。
- 由内置 Go 后端负责：
  - 作为 LocalAPI client 连接 daemon
  - 把 status/tasks/events/sh_attach 等能力通过框架桥接给前端（RPC/event bridge）

候选：

- **Wails（特例纳入）**：Go + system WebView

优点：

- 复用现有 Web UI 资产成本最低（尤其是 xterm.js 的 shell 交互）。
- 后端用 Go，LocalAPI client 复用直接。

主要限制：

- 通常只覆盖桌面端；Android 覆盖需要单独路径（不保证“一套壳全平台”）。
- Wails v2 的 `AssetServer` 对 WebSockets / SSE（Windows response streaming）存在能力边界：若要最大化复用 `xterm.js`（WS）与事件流刷新，需要选择“Wails runtime events + loopback-only WS”等桥接方式（细节见 `docs/decisions/door-1-client-shell-charter.md`）。

### 方案 C：Flutter / Compose 等（非 WebView 渲染）

做法：

- UI 跨平台，但必须解决 **LocalAPI over IPC**：
  - 要么在 UI 语言侧实现 unix socket + named pipe 的 HTTP/SSE/WS
  - 要么内嵌一个 Go sidecar/库，由它负责 LocalAPI client，再通过平台通道/FFI 暴露给 UI

优点：

- UI 体系成熟，Material 3 生态更完整。

主要风险（结合当前 repo）：

- LocalAPI 的 IPC + SSE + WS 子集一次性做齐，工程投入可能明显高于方案 A/B。

### 方案 D：Browser-first

- 不适用：浏览器无法直接访问 LocalAPI 的 unix socket / named pipe。

## 推荐结论（当前阶段）

- 若要求 “一套客户端覆盖 `Windows/Linux/Android` 且严格 LocalAPI-only”，**优先评估方案 A（Go 原生 UI）**。
- **Wails 作为桌面端特例候选（方案 B）**：当“内嵌 xterm shell”优先级更高、且 Android 可接受另行方案时再启用。
- 当前 roadmap 偏好：`D1a` 桌面端优先落地 Wails；移动端（Android）待选型明确后推进（必要时引入 `Flutter`）。

## 下一步建议（可执行的评估清单）

对候选做最小 demo（只做“LocalAPI client + 基础渲染”，不做 fancy UI）：

1. 连接与权限提示：复用 `cmd/miopunch/localapi_client.go` 的探测顺序（system → user）
2. 状态页：`status + peers + tasks`
3. 事件流：接入 `GET /api/v0/events`（SSE），确保任务推进可实时刷新
4. shell：
   - v0（兜底）：拉起外部终端执行 `miopunch sh <peer>`（仍是 LocalAPI）
   - v1（增强）：实现 `sh_attach` 的 WS 字节流承载与窗口 resize 控制帧
