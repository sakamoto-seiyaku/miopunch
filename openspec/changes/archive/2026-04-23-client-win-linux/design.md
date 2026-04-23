## Context

`miopunch` 的 Alpha/POC 已具备可用的最小闭环（daemon `up`、`LocalAPI`、tasks/events/report、`sh_attach`），但当前仍主要以 CLI 形态交付。对普通用户而言，缺少桌面客户端壳会带来两类摩擦：

- **交互摩擦**：join/ping/sh/report 等都需要 CLI；遇到权限/daemon 未运行/版本不匹配时缺少统一的“下一步动作”提示。
- **交付摩擦**：Windows/Linux 上的“安装 + 后台托管 + 权限提示 + 日志”缺少一个统一且可预测的默认路径。

本 change 聚焦 Door 1 的桌面端子阶段（`D1a`）：用 Wails 快速覆盖 `Windows / Linux`，并复用现有 `internal/http_panel/assets/` 的 `MD3 + xterm.js` 资产，但不把 HTTP panel 作为主通道；客户端与 daemon 的交互只走 `LocalAPI`（IPC）。

关键约束与事实基线：

- **LocalAPI-only**：unix socket / Windows named pipe；固定 `Host=local-miopunch.localapi`。
- **排除 Electron**：允许 system WebView（Wails）。
- **Wails 能力边界**：AssetServer 不支持 WebSockets；Windows 上 response body streaming 不支持（影响 SSE）。
- **现成能力**：`miopunch install-system-daemon` / `uninstall-system-daemon` 已覆盖 Linux/Windows（service + stable binary path + operator 权限）。
- **交付形态**：安装器/发行包必须交付两个可执行文件：`miopunch`（daemon/CLI）+ `miopunch-desktop`（GUI）。

## Goals / Non-Goals

**Goals:**

- 交付 `miopunch-desktop`（Wails）覆盖 Windows/Linux，实现：
  - 连接 daemon（默认探测 system→user；可 override）
  - 状态/任务列表 + 实时事件刷新
  - 关键任务触发（以 LocalAPI tasks 为事实源）
  - 内嵌 `sh_attach` 终端（复用 `xterm.js`）
  - report 导出（UI 选路径 → Go 写文件）
- 交付 Windows(NSIS) 与 Linux(.deb) 的最小 packaging 契约：
  - 安装阶段调用 `miopunch install-system-daemon`（fail-fast）
  - 卸载阶段 best-effort 调用 `miopunch uninstall-system-daemon`
  - 明确默认路径（stable binary、GUI、desktop entry、logs）
- 失败可解释：对齐 `reason_code/stage/facts/suggestions` 的既有口径，UI 默认展示“下一步动作”，详情可展开。

**Non-Goals:**

- 不覆盖 Android（Door 1 `D1b` 另行 change）。
- 不承诺自动更新、复杂托盘/通知集成（v0/v1 只做最小可用交付）。
- 不修改 `LocalAPI`/task/report 的需求契约（本 change 复用既有语义；若实现中发现缺口，应单独开 change 修改相关 spec）。
- 不一次性解决所有 Linux 发行版的发行策略（首发 `.deb`；`.rpm`/多发行版矩阵后续收口）。

## Decisions

### 1) 桌面壳选型：Wails（system WebView），排除 Electron

- **选择**：Wails 作为 `miopunch-desktop` 的桌面壳（system WebView）。
- **备选**：
  - Go 原生 UI（Fyne/Gio）：LocalAPI 适配成本低，但内嵌终端与 UI 资产复用成本高。
  - Electron：生态成熟但体积/内存/安全面与本项目阶段目标不匹配（显式排除）。
- **原因**：复用现有 `MD3 + xterm.js` UI 资产成本最低；Go 后端可直接复用 `internal/localapi.Client`，快速出结果。

### 2) 数据通道：Wails bindings + runtime events（避开 AssetServer SSE/WS）

- **选择**：前端不直接通过 HTTP(SSE/WS) 调后端；改为：
  - 前端调用 Go bindings 获取 snapshot（status/tasks/...）
  - Go 侧消费 LocalAPI events（SSE body）并以 runtime events 向前端推送
- **备选**：用 Wails AssetServer 透传 SSE/WS
- **原因**：Wails 能力边界导致 SSE/WS 在 Windows/AssetServer 上不可行或不稳定；bindings/events 是兼容路径且更容易做错误分型与诊断。

### 3) 内嵌终端：loopback-only WebSocket + token（桥接到 LocalAPI WS）

- **选择**：为 `xterm.js` 继续使用 WebSocket；由 `miopunch-desktop` 内置一个仅监听 `127.0.0.1` 的 WS server（随机端口），并用随机 token 做最小防护。
- **备选**：
  - 全改为 bindings/events 承载字节流：实现复杂、容易引入性能与可靠性问题。
  - 放弃内嵌终端改外部终端：与“桌面端必须内嵌”的体验目标冲突。
- **原因**：最大化复用现有 `sh_attach` 的 WS 语义与 `xterm.js` 代码，降低协议漂移风险。

### 4) UI 资产策略：静态复用现有 `internal/http_panel/assets/`

- **选择**：以现有静态 UI 为基线，改造其 transport 层（从 fetch/EventSource 切到 bindings/events；WS 指向本地 bridge）。
- **备选**：引入全新前端工程（Vite/React 等）
- **原因**：v0 目标是最快收口可用性，避免引入额外工具链与构建复杂度；保留后续替换空间。

### 5) Packaging/Installer：只 orchestration，不重写 service/权限逻辑

- **选择**：
  - Windows(NSIS)：复制 `miopunch.exe` + `miopunch-desktop.exe`，调用 `miopunch install-system-daemon`（fail-fast）；卸载 best-effort 调用 `miopunch uninstall-system-daemon`。
  - Linux(.deb)：安装时 postinst 调用 `miopunch install-system-daemon`（fail-fast）；best-effort 加组（若无法推断 operator 用户则不阻塞），但始终输出手动加组指引；卸载时 prerm 调用 `miopunch uninstall-system-daemon`。
- **备选**：在 NSIS/maintainer scripts 中自己实现 systemd/service/ACL/stable-binary 逻辑
- **原因**：避免能力重复与口径分叉；把“事实源”收敛在 `miopunch` 二进制中，installer 只负责调用与记录日志。

### 6) 日志与排障：约定路径 + UI 仅复制/导出

- **选择**：
  - installer log：Linux `/var/log/miopunch/install.log`；Windows `%ProgramData%\\miopunch\\install.log`
  - GUI runtime log：Linux `~/.config/miopunch/logs/miopunch-desktop.log`；Windows `%LocalAppData%\\miopunch\\logs\\miopunch-desktop.log`
  - daemon runtime log：system log 目录（Linux `/var/log/miopunch`；Windows `%ProgramData%\\miopunch\\...`）
  - UI 只展示路径并提供复制；安装器提供导出 installer log 到用户指定路径
- **原因**：遵循用户在各平台上的常见习惯与权限边界；避免 GUI 试图“自动打开”带来的权限/体验不一致。

## Risks / Trade-offs

- [WebView2 Runtime 依赖] 首次运行可能需要联网安装 runtime → **Mitigation**：策略固定为 `-webview2 embed`（内嵌 bootstrapper；不捆绑 fixed runtime）；无法安装则明确提示并退出。
- [Linux WebKitGTK 依赖差异] 不同发行版依赖口径不一致 → **Mitigation**：首发即提供 WebKitGTK 4.0/4.1 两个 `.deb` 变体（4.1 使用 `-tags webkit2_41`）；后续用矩阵验证收口。
- [Wails 能力边界导致桥接复杂] bindings/events + loopback WS 会引入额外桥接层 → **Mitigation**：把桥接层做成最薄、可单测的 Go 包；协议尽量复用既有 `LocalAPI` 与 `shellproto`。
- [Maintainer scripts 下 operator 用户推断不可靠] 可能无法自动把桌面用户加入 `miopunch-operators` → **Mitigation**：best-effort 加组但不阻塞安装；始终输出手动修复步骤，并在 GUI `forbidden` 场景提示同样的“下一步动作”。
- [Loopback WS 被本机其它进程利用] → **Mitigation**：仅监听 `127.0.0.1` + 随机端口 + 随机 token（用户无感）+ 最小 Origin/Host 校验。
- [发行流程/CI 未固化] `.deb`/NSIS 产物如何在 CI 可复现仍需工程投入 → **Mitigation**：本 change 先固化契约与脚手架；CI 矩阵后续按证据补齐。

## Migration Plan

- `D1a(v0)`：先把 `miopunch-desktop` 在开发机上跑通（连接 + tasks/events + 内嵌终端 + report 导出）。
- `D1a(v1)`：补齐 Windows(NSIS) 与 Linux(.deb) 的最小交付；把安装/卸载/Repair 的语义与日志落点固化成 spec 并验证。
- 回滚策略：
  - Windows：卸载器 best-effort 调用 `miopunch uninstall-system-daemon` 并移除二进制与快捷方式（state 保留/按既有命令口径）。
  - Linux：`apt remove`/`apt purge` 分离（按 packaging spec 约定）。

## Open Questions

- `.rpm` 是否进入近期主线交付？是否需要多发行版依赖口径与 CI 矩阵？
- 是否需要为 `install-system-daemon` 增加“显式 operator 用户”参数以提升 packaging 场景的可控性？
- 终端桥接的 token/Origin 校验最小集合是否需要单独写安全 spec？
