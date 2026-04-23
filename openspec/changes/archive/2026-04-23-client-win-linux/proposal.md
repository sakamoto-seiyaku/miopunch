## Why

目前 `miopunch` 的 Alpha/POC 能力（daemon `up`、`LocalAPI`、tasks/events/report、`sh_attach`）已经可用，但缺少面向日常使用的“桌面客户端壳”：GUI、安装包交付、后台服务托管与权限提示。该缺口导致普通用户必须依赖 CLI 才能完成 join/ping/sh/report 等操作，且在 Windows/Linux 上的安装与排障路径不够一致、不可预期。

## What Changes

- 新增桌面 GUI 可执行文件：`miopunch-desktop`（Wails），覆盖 `Windows / Linux`。
- 桌面 GUI 与 daemon 的交互 **只走 `LocalAPI`（IPC）**（unix socket / Windows named pipe），不引入 loopback HTTP 作为主通道。
- GUI 复用现有 Web UI 资产（`internal/http_panel/assets/`），以 `MD3` 风格为基线；终端交互最大化复用 `xterm.js`。
- Bridge 机制：使用 Wails bindings + runtime events；为 `sh_attach` 提供 loopback-only WebSocket + token 的内部桥接，以复用现有 WS 终端语义。
- 提供 Windows(NSIS) 与 Linux(.deb) 的最小交付形态：安装器/发行包交付 `miopunch` + `miopunch-desktop`，并通过调用 `miopunch install-system-daemon`/`uninstall-system-daemon` 复用既有 service 逻辑（安装语义 fail-fast，卸载 best-effort）。
- 明确连接失败与权限不足的 UX 口径（reason_code 分型 + “下一步动作”建议），并提供 LocalAPI override（等价 `--localapi`，默认隐藏）用于排障。
- 非目标：本 change 不覆盖 Android（后续 Door 1 `D1b` 单独推进）。

## Capabilities

### New Capabilities

- `miopunch-desktop-gui-v0`: Windows/Linux 桌面 GUI 的连接、任务驱动、事件刷新、内嵌终端与报告导出等最小能力契约（LocalAPI-only）。
- `miopunch-desktop-packaging-v0`: Windows(NSIS) 与 Linux(.deb) 的交付/安装/卸载/Repair 语义、路径约定、权限提示与日志落点契约。

### Modified Capabilities

<!-- None (this change reuses existing POC LocalAPI/task/report semantics without changing requirements). -->

## Impact

- 新增代码与结构：
  - 新增 `cmd/miopunch-desktop/`（Wails app 入口）与最薄的 Go bridge 层。
  - 新增 Windows installer（NSIS）与 Linux `.deb` 打包脚手架（后续可扩展 `nfpm`/CI）。
- 依赖与运行时：
  - Windows：依赖 WebView2 Runtime（策略：`-webview2 embed`）。
  - Linux：依赖 WebKitGTK/GTK（提供 WebKitGTK 4.0 与 4.1 两个 `.deb` 变体）。
- 运维/排障口径：
  - 新增 installer log 与 runtime log 的默认路径约定；GUI 展示并提供复制/导出能力。
