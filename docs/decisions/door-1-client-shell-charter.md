# Door 1 客户端壳纲领（桌面优先，LocalAPI-only）

## 文档状态

- 当前 Door 1 Pro 桌面主线已由 `docs/decisions/door-1-pro-session-shell-charter.md` 补充并收口；若与本文件中的 installer-first / system-daemon 默认交付口径冲突，以新文档为准。
- 本文件继续保留为后续 privileged 路线的事实源，主要承接安装器、system service、root/管理员权限与未来虚拟组网相关能力。
- 本文档定义 Door 1（客户端壳）的目标、边界、约束与关键原则。
- 本文档不展开实现细节，不替代后续 OpenSpec change。
- Door 1 的“已选定 / 待定 / 已知折中与风险”统一收敛在本文档中，避免分散到多个 decision 文档造成口径漂移。
- 后续实现前，应基于本文档与 `docs/roadmap.md` 创建并收敛对应 change（例如：桌面端 Wails、移动端 Android）。

## 背景

- Alpha/POC（`POC-01..POC-07`）已按最初设计收口：我们已经具备“可用能力”的最小闭环（daemon `up`、`LocalAPI`、task/report、`sh_attach`）。
- 当前缺口不在“打洞语义本身”，而在“可用产品形态”：
  - GUI/安装/托管：降低 CLI 门槛，形成日常可用的客户端。
  - 平台交互：托盘、通知、权限提示、daemon 生命周期管理等。
  - 角色边界：operator/client 的职责与安全边界表达清晰。
- 经验结论：一次性追求“同一套壳覆盖桌面 + 移动端”风险较高、迭代速度慢；Door 1 采用“桌面优先、移动端后续”的分阶段推进策略。

## 仓库事实基线（我们“已经有什么”）

Door 1 的优先策略是“复用已验证的语义与口径”，客户端壳只补 GUI/包装/托管。

- daemon（POC 线）：
  - `miopunch up` 提供 `LocalAPI`（IPC：unix socket / Windows named pipe）+ tasks/events/report + `sh_attach`。
- LocalAPI 客户端实现（Go）：
  - `internal/localapi.Client` 已覆盖：HTTP/JSON、events（SSE body）、以及 `sh_attach` 的 WS（`miopunch.sh.v0`）。
  - CLI 侧已有可参考的连接探测与错误口径：`cmd/miopunch/localapi_client.go`。
- daemon “系统托管”能力（已存在）：
  - `miopunch install-system-daemon` / `uninstall-system-daemon` 已覆盖 Linux/Windows（system service + stable binary path）。
- 现成 Web UI 资产（可复用）：
  - `internal/http_panel/assets/` 已包含 `MD3` 视觉基线 + `xterm.js` 终端交互 + join/invite/shell/status 的最小页面结构与渲染逻辑。

## 术语与约定（减少歧义）

- `LocalAPI`：
  - daemon 对外的本机 IPC API（HTTP/JSON + SSE + WS）。
  - 所有请求必须携带固定 `Host`：`local-miopunch.localapi`（`internal/poc.LocalAPIHost`）。
- `system LocalAPI` / `user LocalAPI`（指监听端点，而非权限语义）：
  - Linux(system)：unix socket：`/run/miopunch/localapi.sock`（`internal/localapi/listen_unix.go`）。
  - Linux(user)：unix socket：`$XDG_RUNTIME_DIR/miopunch/localapi.sock`。
  - Windows：named pipe：`\\.\pipe\miopunch\localapi-<operator_sid>`（`internal/localapi/listen_windows.go`）。
- Linux `operator group`：
  - group 名称固定为：`miopunch-operators`（`internal/poc.LinuxOperatorGroup`）。
  - 目的：在 system daemon 模式下，给非 root 的 operator 提供最小权限的 LocalAPI 访问入口。
- system state/log 目录（用于讨论 remove/purge 行为）：
  - Linux(system state)：`/var/lib/miopunch`（`internal/pocstate/dirs.go`）。
  - Linux(system log)：`/var/log/miopunch`（约定；安装/卸载日志与 system daemon 日志默认落在该目录下）。
  - Windows(system log)：`%ProgramData%\\miopunch\\`（约定；安装器与 system daemon 日志默认落在该目录下）。
- user log 目录（前台/GUI）：
  - Linux：`os.UserConfigDir()/miopunch/logs/`（通常为 `~/.config/miopunch/logs/`）。
  - Windows：`%LocalAppData%\\miopunch\\logs\\`（避免 Roaming profile；对齐常见桌面应用习惯）。
- `install-system-daemon` / `uninstall-system-daemon`：
  - CLI 子命令，用于注册/启动/卸载 system service，并在安装时处理 stable binary 路径与 operator 权限。
  - Linux stable binary path：
    - 当前实现：`/usr/local/bin/miopunch`（`cmd/miopunch/system_daemon_linux.go`）。
    - `.deb` 交付目标：`/usr/bin/miopunch`（需在 Door 1 `D1a` 中对齐实现）。
  - Windows stable binary path：`%ProgramFiles%\\miopunch\\miopunch.exe`（`cmd/miopunch/system_daemon_windows.go`）。
- system service 名称：
  - Linux：`miopunch`（通常对应 `miopunch.service`）。
  - Windows：`miopunch`（Windows Service）。
- `miopunch` / `miopunch-desktop`：
  - `miopunch`：CLI + daemon（二进制本体）。
  - `miopunch-desktop`：桌面 GUI（二进制壳，Wails）。
  - 用户可见的应用名称仍为 `miopunch`（桌面入口/开始菜单的展示名），`miopunch-desktop` 仅是可执行文件名（避免与 `miopunch` CLI/daemon 冲突）。

## 核心决策

- LocalAPI-only：
  - 客户端与 daemon 的交互 **只走 `LocalAPI`（IPC）**：unix socket / Windows named pipe。
  - `HTTP panel` 仅作为“浏览器调试/演示面板”，不作为 Door 1 的主通道与技术基线。
- 排除 Electron：
  - 不采用 `Electron` 作为客户端壳技术路线。
  - 允许 system WebView（例如 Wails）作为桌面端实现手段（包体/内存/依赖由系统 WebView 承担）。
- 桌面优先（`D1a`）：
  - 先用 `Wails` 快速覆盖 `Linux / Windows` 桌面端（优先“可用、可解释、可回归”，而非“完美原生体验”）。
  - 前端直接复用现有 `MD3 + xterm.js` UI 资产（以 `internal/http_panel/assets/` 为起点，改动以“替换 transport/bridge 层”为主）。
  - 生命周期策略（桌面 v0）：**客户端只负责连接已运行 daemon**，不在 App 内做“启动/停止/安装服务”的交互（由安装器/包管理器负责）。
  - 发包与安装（桌面最终交付必须具备）：
    - Windows：提供安装器（建议 NSIS），安装过程需要管理员权限，用于把 daemon/服务安装到正确位置并启动后台服务。
    - Linux：提供发行包（例如 `deb/rpm`），安装过程需要 root 权限，用于把 daemon/服务安装到正确位置并启动后台服务；daemon 稳定路径以 `/usr/bin/miopunch` 为准（Debian policy 合规；需在 `D1a` 中对齐实现）。
- 移动端后续（`D1b`）：
  - Android 端以“控制端”为主：不实现被控端/agent、不承诺 daemon 常驻语义（另行评估）。
  - UI 美观度存在最低要求（尤其 Material 3 / dynamic color）；实现路径另行调研，必要时可引入/切换 `Flutter`。
- 交互式终端是核心能力：
  - 客户端必须支持 App 内交互式 `sh_attach`（承载 `LocalAPI` 的 WS 字节流与窗口 resize 控制帧）。
  - 桌面端优先内嵌（避免依赖外部终端）；移动端同样以“内嵌”为必备。
- D1a 的 bridge 约束（来自 Wails 能力边界）：
  - Wails `AssetServer` 不支持 WebSockets，且 Windows 上不支持 Response Body Streaming（影响 SSE）。因此：Door 1 不把“前端直接走 HTTP(SSE/WS) 调后端”作为默认实现路径；优先用 Wails runtime events/JS-binding 做数据流转，终端部分单独走 loopback-only WebSocket 以最大化复用 `xterm.js` 交互逻辑。
- LocalAPI 连接优先级（桌面 v0 默认）：
  - 默认探测顺序为 system→user（与当前 CLI 口径一致）。
  - 保留“显式指定地址”的能力（等价于 CLI 的 `--localapi`）。
- report 的落地方式（桌面 v0）：
  - UI 侧选择保存路径；bridge 拉取 task report 并写文件；不依赖浏览器“下载”语义。
- 原则：安装器/发行包不重复实现 daemon 托管逻辑：
  - 安装/卸载/Repair 只调用 `miopunch install-system-daemon` / `miopunch uninstall-system-daemon`（单一事实源），并把其输出作为用户可见诊断与 installer log 的主要内容。
  - NSIS/postinst/prerm 脚本不在脚本层重新实现“加组/ACL/stable binary copy/service 注册”等逻辑；若语义需要调整，应改 Go 代码/命令输出口径。

## 交付物与发布形态（D1a）

Door 1 的桌面端不是“单一可执行文件 all-in-one”（会被 WebView2 / WebKitGTK / service 权限等现实牵制），而是以“安装包交付”为主。

- 交付物分两层：
  - 裸二进制：`miopunch`（现状 CLI/daemon，可供高级用户与调试使用）。
  - 安装包（面向普通用户）：包含至少两个可执行文件并完成安装与托管：
    - `miopunch`：daemon + CLI（用于安装/升级/调试；安装包内部用于注册系统服务）
    - `miopunch-desktop`：桌面 GUI（Wails）
- 约束：`miopunch-desktop` 单独分发不能工作；安装包必须同时交付 daemon（`miopunch`）并完成 system service 注册/启动。
- 安装包文件形态（每个平台一个“可分发的安装文件”）：
  - Windows：单一安装器（建议 NSIS `.exe`）。
  - Linux：单一 `.deb`（后续需要时再补 `.rpm`）。
  - 用户入口：安装完成后应提供桌面入口（Windows Start Menu shortcut / Linux `.desktop` entry），展示名为 `miopunch`，其 Exec 目标指向 `miopunch-desktop`。
- Linux 上安装包可能存在多个格式（例如 `.deb` / `.rpm`），但“安装完成后文件位置与服务语义”应保持一致。
  - `D1a(v1)` 首发优先交付 `.deb`（`.rpm` 作为后续补齐项）。

## 目标

### `D1a`（桌面端：Wails，Linux/Windows）

- 最小可用闭环（GUI 驱动 POC 能力）：
  - 能连接本机 daemon（LocalAPI 探测/选择）。
  - 能查看：`status / peers / tasks / report`。
  - 能实时刷新：bridge 订阅 `LocalAPI events` 并推送到 UI（Wails runtime events），驱动 UI 状态推进。
  - 能发起关键任务：`invite / join / approve / ping / sh_ls / sh_attach / revoke_member`（按已实现能力取子集也可，但需明确最小清单）。
- 内嵌 `sh_attach` 终端：
  - 能正确承载字节流、处理用户输入、支持窗口 resize。
  - 终端体验以“可用”为第一优先级（不追求完整 VT 特性一次到位）。
- 体验基线：
  - UI 风格倾向 `MD3`（与 HTTP panel 的视觉口径一致即可，不追求深度主题系统）。
  - 失败可解释：复用 task/report 的 reason/stage/建议动作，不另造第二套诊断口径。

### `D1a` 实施拆分（与现有代码的配合点）

为避免 Door 1 变成“UI 自己又实现一套状态机/协议”，D1a 只新增最薄的一层 bridge 与包装。

- Go bridge（桌面 app 内）：
  - LocalAPI 连接：复用 `internal/localapi.Client`（默认 system→user 探测；支持显式指定 addr）。
  - API 映射：把 `status/peers/tasks/create_task/get_task/get_report` 映射成前端可调用的方法（JS bindings）。
  - 事件流：打开 `LocalAPI /api/v0/events`（SSE body）并解析事件，转发为 Wails runtime events（`EventsEmit`）。
  - 终端桥接：提供 loopback-only WebSocket server（仅 `127.0.0.1`）：
    - 上行：xterm → WS → bridge → LocalAPI WS（二进制帧）
    - 下行：LocalAPI WS（二进制帧）→ bridge → WS → xterm
    - 控制帧：支持 `winsize`（复用 `shellproto` 语义）
  - report 保存：前端选路径 → bridge 拉取 report 文本并写入文件。
- Web UI（复用 `internal/http_panel/assets/`）：
  - 保留现有 `MD3 + xterm.js` 的布局与渲染；仅替换 transport：
    - 从 `fetch/EventSource` 切换到 “JS bindings + runtime events”
    - `sh_attach` 仍使用 WebSocket（指向 loopback-only WS server）

### `D1a` 交付分阶段（建议）

- `D1a(v0)`：开发/自测可用（最小闭环 + 内嵌终端 + report 保存）。
- `D1a(v1)`：发包可交付：
  - Windows：NSIS 安装器 + WebView2 runtime 策略（`embed`）+ 安装时注册/启动 daemon 服务（复用 `miopunch install-system-daemon`）。
  - Linux：`.deb` 打包（WebKitGTK 4.0/4.1 双变体）+ postinst best-effort 加组并注册/启动 daemon 服务（实现口径见本文“决策与问题清单”）。

### `D1b`（Android：控制端）

- 最小目标：
  - 作为 operator 的“随身控制端”，完成 join/ping/sh 等核心动作。
  - UI 采用 Material 3，并支持 dynamic color（若所选框架可行）。
  - App 内必须可用地承载交互式终端（`sh_attach`）。
- 说明：
  - Android 端不以“一次性复用桌面端壳”作为硬约束；可允许技术分叉，但应尽量复用同一套 LocalAPI 语义与 Go 侧逻辑（减少行为漂移）。

## 非目标（明确推后）

- 不把 Door 1 当作“继续扩展打洞/协议语义”的入口；相关能力进入第二方向（TCP punching）或主线（证据驱动）。
- 不在 `D1a(v0)` 追求自动更新与复杂系统集成；安装器/发行包只要求满足“把 daemon/服务装到正确位置并可启动”的最小基线。
- 不在 `D1a` 一次性解决所有 Linux 发行版的打包分发策略（先确保开发/自测可跑，再逐步收敛发行形态）。
- 不在 `D1b` 立即承诺 iOS、完整移动端多机型适配与完美原生交互（先把控制端闭环跑通）。

## 安全与权限边界

- 客户端不得新增“对外网络监听”作为默认行为；对 daemon 只通过本机 IPC（LocalAPI）连接。
- 允许为桌面端 `xterm.js sh_attach` 启动 loopback-only 的本机 WebSocket（仅 `127.0.0.1`，随机端口）作为 UI↔bridge 的内部通道；该通道应具备最小防护（例如随机 token / origin check），避免被本机其它进程或网页轻易复用。
- 依赖现有 `LocalAPI` 的安全边界：
  - OS 权限（socket mode / named pipe DACL）
  - 固定 `Host` 约束（`local-miopunch.localapi`）
- 任何涉及“daemon 启停/托管”的行为必须显式提示权限边界与失败原因，避免默默失败或 silent fallback。

## UX 口径（D1a v0：连接与错误）

- 连接探测顺序：system → user →（可选）用户显式指定（等价于 CLI `--localapi`），并在 UI 明确显示当前连接的是哪一个端点。
- 失败分类（对齐 `cmd/miopunch/localapi_client.go` 的 reason_code 语义）：
  - `bad_request`：LocalAPI 地址格式错误（用户配置/高级设置）。
  - `forbidden`：权限不足（Linux 未加入 `miopunch-operators`；Windows 非安装 system service 的 operator 用户）。
- `daemon_not_running`：socket/pipe 不存在或不可达（daemon 未运行/未安装）。
- 其它/未知：端点可连接但协议不匹配、返回非预期状态码等（疑似版本不匹配、端点损坏或环境异常）。
- `forbidden` 的建议动作（桌面 v0 基线）：
  - Linux(system daemon)：提示加入 `miopunch-operators` 并重新登录（例如：`sudo usermod -aG miopunch-operators $USER`；然后注销/重新登录）。
  - Windows(system daemon)：提示“用安装 system service 的同一 Windows 用户运行/打开 GUI”，或“卸载后用目标用户重新安装服务”。
- `daemon_not_running` 的建议动作（桌面 v0 基线）：
  - 前台模式：`miopunch up`
  - system service 模式：`miopunch install-system-daemon`（Windows 需要管理员权限；Linux 需要 root）
- 展示层级（默认 vs 详情）：
  - 默认：一句话结论 + 2–3 条“下一步动作”（suggestions）。
  - 详情：展开显示 `stage + reason_code + facts + addr`（不默认铺开底层原始错误栈）。
- “其它/未知”错误的建议动作（桌面 v0 基线）：
  - 先提示“可能是版本不匹配 / 端点不对 / 环境异常”，并建议：
    - 通过安装器 Repair/重装（确保 GUI 与 daemon 版本一致）
    - 导出 installer log + runtime log 以便排障
    - （高级用户）用 CLI 显式指定 `--localapi ...` 直连并查看 `reason_code/facts`
- UI 不做自动提权/自动安装：
  - 当 daemon 未运行/未安装时，UI 只负责给出明确的“下一步命令/动作指引”，由用户或安装器/包管理器执行。
- 互操作（避免口径漂移）：
  - UI 必须提供“一键复制等价 CLI 命令”（至少包含：启动前台 daemon、安装/Repair system service、查看日志）。
- 高级设置（桌面 v0 基线）：
  - 提供“LocalAPI 地址 override”（默认空；仅用于排障/高级用户），语义等价于 CLI `--localapi`。
  - UI 必须明确提示“override 会跳过默认探测顺序”，并提供“一键清除 override”。

## 日志与诊断（D1a v0：安装 vs 运行）

- 安装/卸载/Repair 日志（installer log）：
  - Linux(.deb)：`/var/log/miopunch/install.log`（由 maintainer scripts 追加写入；失败时提示该路径）。
  - Windows(NSIS)：`%ProgramData%\\miopunch\\install.log`（追加写入），并提供“导出到用户指定路径”能力。
- 日常运行日志（runtime log）：
  - daemon（system service）：
    - Linux：`/var/log/miopunch/miopunch.log`（约定；必要时可补充 `journalctl -u miopunch` 的查看指引）。
    - Windows：`%ProgramData%\\miopunch\\<operator_sid>\\logs\\miopunch.log`（约定）。
  - GUI（桌面 app）：
    - Linux：`os.UserConfigDir()/miopunch/logs/miopunch-desktop.log`（约定）。
    - Windows：`%LocalAppData%\\miopunch\\logs\\miopunch-desktop.log`（约定）。
- 轮转（v0 约定）：单文件 `10MB`，最多 `1` 个备份，覆盖最旧。
- UI 对日志路径的交互：
  - 只展示路径并提供复制（不强制自动打开目录/文件，避免打乱用户习惯与权限边界）。

## 可观测性与回归

- Door 1 的 GUI 不应成为“不可回归”的黑盒：
  - 核心行为必须能落到可复现的 task/report 产物（复用现有落盘与导出逻辑）。
  - GUI 仅做“展示/触发”，不定义独立的状态机语义；核心语义仍由 daemon/task 层给出并可测试。
- `sh_attach` 的终端桥接必须可诊断：
  - 断链/超时/权限/peer 拒绝等情况要有明确可见的错误表达（复用 reason_code 为主）。

## 测试与验收（最小要求）

- Go 侧（daemon/LocalAPI/task）仍以现有单测/集成测试为回归基线。
- Door 1 引入的新增逻辑（例如 bridge 层）应具备最小单元测试（尤其是协议桥接、参数校验、错误映射）。
- 至少固化一条“真实可演示”的桌面端 smoke：
  - 连接 daemon → join/ping → 打开内嵌终端完成一次交互式 shell。

## 建议的 change 切分（用于后续 OpenSpec）

- Change `D1a`：桌面端 Wails client（LocalAPI-only + 内嵌终端 + 基础任务）
- Change `D1b`：Android 控制端（框架选型确定后再开；优先解决 Material 3 + dynamic color + 终端承载）

## 调研笔记（Wails：官方约束与社区反馈摘录）

本节用于记录 Door 1 相关的“会影响架构选择”的事实；不在这里做实现细节设计。

- Wails `AssetServer` 能力矩阵显示：WebSockets 不支持；Windows 上 Response Body Streaming 不支持（影响 SSE）。因此需要避开“用 AssetServer Handler 直接挂 WS/SSE”的方案。
- Windows WebView2：
  - Wails 在 Windows 上依赖 Microsoft WebView2 Runtime，并提供 `-webview2` 构建策略（download/embed/browser/error）与 fixed version runtime 的指引。
  - 选定策略：`-webview2 embed`（内嵌官方 bootstrapper；不捆绑 fixed runtime）。
  - 维护者社区讨论中明确提到“并非所有机器都有 WebView2；runtime 体积较大”，并讨论了用 bootstrapper/installer 解决依赖的策略。
- Windows 安装器：
  - Wails 官方支持用 NSIS 生成 Windows installer（`wails build -nsis`），并生成默认配置骨架（`build/windows/installer`）。
- Linux 发包：
  - Wails v2 文档未提供“一键 Linux 打包发行”的内建工具链；建议采用独立 packaging（例如 `nfpm`）产出 `.deb/.rpm`，因此 Linux 的交付需要我们自建 packaging 流程。
- Linux 依赖：
  - Wails 需要 `libgtk3 + libwebkit`；并对较新发行版（例如 Ubuntu 24.04）给出 `libwebkit2gtk-4.1-dev` + `-tags webkit2_41` 的指引。
- Frontend build：
  - `D1a(v0)` 以“静态前端 + 最小改动复用 `http_panel/assets`”为第一目标，避免引入 Vite 动态资产等复杂度。
  - 官方文档明确：Wails v2 的 Dynamic Assets 与 Vite v5 不兼容；若需要该能力应停留在 Vite v4。

参考链接（官方/社区）：

- Wails v2 options / AssetServer matrix：https://wails.io/docs/v2.11.0/reference/options
- Wails v2 安装依赖（含 Ubuntu 24.04 `webkit2_41` 提示）：https://wails.io/docs/v2.11.0/gettingstarted/installation
- Wails v2 Windows（`-webview2` 四种策略 + fixed runtime）：https://wails.io/zh-Hans/docs/v2.11.0/guides/windows/
- Wails v2 NSIS installer（`wails build -nsis`）：https://wails.io/docs/guides/windows-installer
- Wails v2 Linux bundling（文档中提到需自行打包，可用 nfpm）：https://wails.io/docs/guides/file-association/
- Wails v2 runtime events（Go/JS 都可 emit/on）：https://wails.io/docs/reference/runtime/events/
- WebView2 runtime 依赖讨论（维护者/社区）：https://github.com/wailsapp/wails/discussions/736
- Wails v2 Dynamic Assets（Vite v5 不兼容）：https://wails.io/docs/v2.11.0/guides/dynamic-assets/
- nFPM（deb/rpm 打包工具）：https://nfpm.goreleaser.com/docs/

## 决策与问题清单（统一入口）

### 已选定（但仍需要落地实现）

- `D1a(v0)` 生命周期：GUI 仅连接已运行 daemon（不在 App 内做“安装/启停/托管”的交互）。
- UI 构建策略：静态复用 `internal/http_panel/assets/`；UI 设计风格以 `MD3` 为基线。
- 数据通道：Wails bindings + runtime events。
- 交互式终端：loopback-only WS + token（用户无感）以最大化复用 `xterm.js`。
- 连接失败口径（桌面 v0）：区分 `bad_request / forbidden / daemon_not_running` 等，并默认展示“下一步动作”；详情里显示 `stage + reason_code + facts + addr`；UI 提供“一键复制等价 CLI 命令”。
- 高级设置（桌面 v0）：提供 LocalAPI override（等价 `--localapi`，默认隐藏/可清除），用于排障与非标准环境。
- Installer/发行包职责：不重复实现 service/权限/stable-binary 逻辑；只调用 `miopunch install-system-daemon`/`uninstall-system-daemon` 并记录日志（单一事实源在 `miopunch` 二进制）。
- 日志约定（桌面 v0）：installer log + runtime log 路径固定（Windows GUI log 使用 `%LocalAppData%`；见上文“日志与诊断”），并采用最小轮转（`10MB × (1+1)`）。
- Windows：
  - `wails build -webview2 embed`（内嵌 bootstrapper；缺 runtime 时走安装引导策略）。
  - GUI 安装路径：`%ProgramFiles%\\miopunch\\miopunch-desktop.exe`（与 daemon stable binary path 同目录）。
  - WebView2 离线体验（v1）：不支持离线；若缺 WebView2 Runtime 且无法下载，则应提示原因与下载指引并退出。
  - NSIS installer 复制 `miopunch.exe + miopunch-desktop.exe`，并调用 `miopunch install-system-daemon` 完成服务注册/启动（复用既有 service 逻辑）。
  - 安装失败语义：若 `miopunch install-system-daemon` 失败，则安装器应 fail-fast（安装失败并提示原因与修复建议）。
  - 卸载语义：卸载器 best-effort 调用 `miopunch uninstall-system-daemon`（需要管理员权限）；失败则提示原因但仍继续卸载 GUI/二进制；并明确 state 保留（对齐命令输出口径）。
  - Repair flow：安装器提供 Repair/重新安装入口，本质为“重新复制二进制 + 重跑 `miopunch install-system-daemon`”（需要管理员权限，失败则提示原因与修复建议）。
  - 安装/Repair 诊断输出：安装器应落地 `install.log`（或等价日志），并提供“一键导出”到用户指定路径的能力。
  - 安装后启动：安装器提供 “Launch miopunch” 选项（默认勾选）。
- Linux：
  - 首发 `.deb` 提供 WebKitGTK 4.0/4.1 两个变体（4.1 变体使用 `libwebkit2gtk-4.1` 且构建需 `-tags webkit2_41`）。
  - stable binary path：`/usr/bin/miopunch`（Debian policy 合规；需在 `D1a` 中对齐实现）。
  - GUI 安装路径：`/usr/bin/miopunch-desktop`；并提供 `.desktop` entry + icon（展示名为 `miopunch`，Exec 指向 `miopunch-desktop`）。
  - `.desktop` / icon 落点：
    - `/usr/share/applications/miopunch.desktop`
    - `/usr/share/icons/hicolor/...`（按桌面环境惯例提供至少一套尺寸）
  - `.deb` service 落地方式：postinst 调用 `miopunch install-system-daemon`（复用既有 service/权限逻辑）。
  - `.deb` 权限：best-effort 加组（若能推断出 operator 用户则自动加入 `miopunch-operators`；否则不阻塞安装），并在安装结束时始终输出“如何手动加组并重新登录”的指引（对齐 Linux 常见软件如 docker 的用户行为）。
  - `.deb` 安装失败语义：`miopunch install-system-daemon` 失败则安装失败（fail-fast，安装器必须提示原因与修复建议）。
  - `.deb` 卸载语义：
    - `prerm` 调用 `miopunch uninstall-system-daemon`；若提示“service 未安装/不存在”则继续卸载，其它失败则卸载失败（fail-fast）。
    - `apt remove`：移除二进制与桌面入口，但保留 system state（例如 `/var/lib/miopunch`）。
    - `apt purge`：在 `remove` 基础上删除 system state（例如 `/var/lib/miopunch`）与日志目录（例如 `/var/log/miopunch`）。
  - `.deb` 文件归属：由 dpkg 直接安装/卸载 `/usr/bin/miopunch` 与 `/usr/bin/miopunch-desktop`（卸载即应删除 CLI/daemon 与 GUI）。
  - 安装/卸载诊断日志：落 `/var/log/miopunch/install.log`（安装/卸载失败时提示日志路径）。

### 待定（需要继续讨论/调研）

- Android（`D1b`）框架选型与交付形态（不阻塞 `D1a`；本轮讨论暂不展开；必要时引入/切换 `Flutter` 的具体边界与回滚策略后续收口）。
- Linux 发包策略的后续补齐：
  - `.rpm` 是否进入主线交付；以及不同发行版的依赖口径如何表达。
  - 是否需要把 `.deb` 的产物与依赖口径扩展成“多发行版矩阵”（例如 Debian/Ubuntu/其他）并固化 CI 验证。

### 已知折中 / 风险（先记录）

- Linux 安装场景的 operator 用户识别：
  - `miopunch install-system-daemon` 会尝试从 `SUDO_USER/DOAS_USER/PKEXEC_UID` 推断 operator 用户；在 `apt/dpkg` maintainer scripts 环境下这些信息可能不可用或不符合预期，进而导致安装失败或安装后当前桌面用户无权限访问 system LocalAPI。
  - 需要明确“推荐安装方式”（例如通过 sudo 执行 apt/dpkg），并在失败提示里给出手动修复步骤：`groupadd -f miopunch-operators`、`usermod -aG miopunch-operators <user>`、重新登录，然后重试安装/Repair。
- `.deb` 与 `install-system-daemon` 的“stable binary copy”重复：
  - `.deb` 直接安装 `/usr/bin/miopunch` 后再调用 `miopunch install-system-daemon`，会与“stable binary copy”逻辑存在重复；需要确保实现不会与 packaging 互相打架（例如：若当前已在 stable path，则跳过自拷贝/覆盖）。
- `-webview2 embed` 的网络依赖：
  - `embed` 仅内嵌 bootstrapper；首次运行缺 WebView2 Runtime 时仍可能需要联网安装 runtime（后续若要支持离线 fixed runtime 再单独收口）。
- `.deb` 安装到 `/usr/local/bin/miopunch`（不采用）：
  - Debian policy 明确要求包不写入 `/usr/local`；因此 `.deb` 以 `/usr/bin/miopunch` 为基线（与当前实现的 stable path 不一致处由 `D1a` 对齐）。
