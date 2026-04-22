# Cross-platform 客户端壳（Door 1）选型调研（排除 Electron）

## 背景：我们仓库“已经有什么”

本调研只讨论“客户端壳（GUI/安装/托管）”，不扩展打洞/协议语义。

- `miopunch up` 已提供两条本地接口（同一 `task.Manager`）：
  - `LocalAPI`：`internal/localapi`（Unix socket / Windows pipe），对外暴露 `status/peers/tasks/events + ws(sh_attach)`。
  - `HTTP panel`：`internal/http_panel`（loopback-only `127.0.0.1:<port>`），内置静态 UI（MD3 + xterm.js + QRCode）。
- `HTTP panel` UI 通过相对路径 `fetch("/api/v0/...")` 访问 API，且对 `POST /api/v0/tasks` 与 `GET /api/v0/tasks/{id}/ws` 做了 same-origin 校验（`Origin/Referer` 必须是 `http://127.0.0.1:<port>` 或 `http://localhost:<port>`）。
  - 这意味着：**客户端壳若复用现有 HTTP panel UI，应该直接加载面板 URL**（例如 `http://127.0.0.1:27400/`），而不是用 `file://`/自定义 scheme 承载前端资源再跨域调用 API。

关联实现入口：

- daemon 启动面板：`cmd/miopunch/up.go`（`--http_panel`、`--http_panel_listen_addr`）
- 面板 loopback-only：`internal/http_panel/listen.go`
- same-origin：`internal/http_panel/origin.go`

## 约束与目标

- 硬约束：`Electron` 直接排除。
- 目标：先把现有 POC 能力“糊成可用客户端”，不追求移动端原生体验，不追求 UI 极致精致。
- 优先关注：安装/更新、daemon 托管、跨平台可运行、空载内存与启动开销可控。

## 方案分型（结合当前实现）

### 方案 A：Browser-first（不做桌面壳）

做法：

- 继续沿用 `miopunch up --http_panel`，直接用系统浏览器打开面板 URL。

优点：

- 0 额外运行时 / 0 GUI 框架引入。
- 复用现有 MD3 UI、xterm、二维码能力。

缺点：

- 不像一个“独立客户端应用”（窗口/托盘/自启动/更新/daemon 托管需要另做）。

### 方案 B：System WebView 壳（推荐优先评估）

做法：

- 做一个很薄的跨平台窗口应用：负责启动/探测 `miopunch up --http_panel`，然后用系统 WebView 打开 `http://127.0.0.1:<port>/`。

为什么它和我们的 repo 更贴合：

- 我们已经有 `HTTP panel`（MD3 + xterm + QR），壳只需要“开一个窗口 + 管一下 daemon”。
- same-origin 规则天然满足：壳加载的就是面板 origin（`http://127.0.0.1:<port>`）。

候选（均非 Electron）：

- **Go 生态最薄壳**：`webview`/WebView2/WebKitGTK 这类“直接开 WebView 窗口”的库
  - 优先级高：不引入第二门语言工具链；可以和现有 Go 构建/发布流程融合。
- **Wails**（Go + 系统 WebView）：更偏“完整应用框架”，适合后续增加系统托盘、窗口管理、自动更新等。
- **Tauri**（Rust + 系统 WebView）：也可以，但会引入 Rust 工具链；对“仅做壳”来说不一定划算。

需要明确的现实成本（不夸大/不拍脑袋）：

- Windows：通常依赖 WebView2 runtime（系统/安装器侧要处理）。
- Linux：通常依赖 WebKitGTK（发行版差异、依赖体积与安装体验要评估）。

### 方案 C：Native-render（不用 WebView）

做法：

- 完整重写 GUI，用 `LocalAPI`（Unix socket / Windows pipe）作为唯一后端接口；或直接把 daemon 能力嵌入进 app。

候选：

- Flutter（Material 3 生态成熟）
- Compose Multiplatform（Material 3 语义原生，但发行包通常会更重）
- Avalonia/.NET、Qt
- Go 原生 UI（Fyne/Gio 等）

风险/成本（结合当前 repo）：

- 需要重做：任务列表/事件流（SSE）、二维码、尤其是 **交互式 shell attach**（当前用 xterm.js + WebSocket subprotocol `miopunch.sh.v0`）。
- “功能即可”的前提下，投入可能明显高于方案 B。

## 推荐结论（当前阶段）

- **优先走方案 B（System WebView 壳）**：最小化重写，最大复用现有 HTTP panel（MD3/xterm/QR 已齐）。
- 方案 C 作为“后续再讨论”的升级路径：只有当 WebView 壳在内存/发行体验上不可接受时，再考虑投入重写原生 UI。

## 下一步建议（可执行的评估清单）

对 2 个候选（“Go 最薄壳” vs “Wails”）各做一个最小 demo，仅实现：

1. 启动或复用后台 daemon（`miopunch up --http_panel`）
2. 打开面板 URL（`http://127.0.0.1:<port>/`）
3. 记录并对比指标：
   - 空载 RAM（启动后静置 30s）
   - 冷启动耗时（点击到窗口出现）
   - 安装包体积与依赖（Windows WebView2 / Linux WebKitGTK 的策略）

