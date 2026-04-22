## Why

当前 POC 已具备 `miopunch up` + LocalAPI v0（task + SSE + WS + report）这一套“可解释性与执行闭环”，但交互主要依赖 CLI。为了降低演示/排障的心智负担，并把已冻结的 task/report/diagnosis 能力收束为“可点击的本机 UI”，需要一个最小的本机 HTTP 面板来展示状态、触发有限写操作并承载浏览器端 shell attach。

## What Changes

- 新增可选的本机 HTTP 面板（默认关闭），显式开启后仅监听 `127.0.0.1`（默认 `127.0.0.1:27400`）。
- 面板提供最小静态页面（单页、固定 Tab/卡片），通过 SSE 实时刷新（不做轮询）。
- 面板写操作白名单：仅允许创建 `invite` / `join` / `sh_attach` 三类 task；其它写操作一律拒绝并提示“请用 CLI”。
- 面板复用既有 task 状态 / 诊断输出 / Markdown report：不再定义第二套导出/脱敏/留存语义。
- Shell：浏览器端使用 WebSocket attach（`Sec-WebSocket-Protocol: miopunch.sh.v0`）+ 终端渲染器（`xterm.js`）完成最小交互。

## Capabilities

### New Capabilities
- `miopunch-poc-http-panel-v0`: 定义本机 HTTP 面板的 listener 约束（loopback-only）、静态 UI 形态、SSE 刷新、写操作白名单、以及写操作/WS 的最小同源安全边界。

### Modified Capabilities
- (none)

## Impact

- Affected code (expected):
  - `cmd/miopunch`：`miopunch up` 在启用面板时启动额外的 loopback HTTP listener，并在 stdout 打印一次面板 URL。
  - `internal/*`：新增 `http_panel`（或等价）模块；引入/内置静态资源打包（`go:embed`）；面板 handler 复用 task/SSE/WS/report 的既有实现或薄封装。
- Dependencies (expected):
  - 浏览器端终端渲染库（例如 `xterm.js`）与极小二维码库（本地打包/内置；不依赖外网 CDN）。
- Non-impact:
  - 不修改 LocalAPI v0 的“IPC-only”契约；面板是额外的 loopback HTTP listener。
  - 不引入登录体系/账户体系；POC 安全边界以 loopback + 同源校验 + 写白名单为主。
