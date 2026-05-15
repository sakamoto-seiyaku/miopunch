# WSL2 / Windows shell 真机测试计划

日期：2026-05-15

状态：计划文档，尚未执行。本文只定义后续应当补齐的真实运行测试矩阵、证据标准和脚本分组，不代表这些测试已经通过。

## 1. 背景

`docs/notes/2026-05-14-wsl2-windows-connectivity-debug-discussion.md` 证明了一个有效方法：不要只看 UI 或单次成功摘要，而是用 Windows + WSL2 mirrored 的真实环境，按 signaling、candidate、punching、dataplane、hello/governance、payload、session lifecycle 分层收集证据。

shell 功能比 `ping` 更容易被误判，因为它同时依赖：

- 真实 LocalAPI daemon。
- 真实 membership / governance 状态。
- 真实 UDP/TCP dataplane session。
- `sh_ls` / `sh_attach` task 协议。
- LocalAPI WebSocket。
- Linux PTY 或 Windows ConPTY。
- 远端 `tmux`、`wsl.exe` 或 `ssh.exe`。
- terminal 输入、输出、resize、关闭与重连。

因此后续 shell 回归测试必须是一组一组真实运行的测试，而不是只靠 Go unit test、浏览器 fake bridge 或一次手动 GUI smoke。

## 2. 目标

- 建立 Windows + WSL2 真实环境下 shell 的可重复测试矩阵。
- 覆盖 `sh ls`、`sh attach`、输入输出、resize、关闭、重连、失败口径和 transport 选择。
- 每组测试都产出可归档证据：CLI stdout/stderr、task report、daemon log、topology snapshot、进程状态和必要的远端 tmux 证据。
- 明确哪些测试要求 fresh daemon / fresh session，哪些测试专门验证 session reuse。
- 先形成文档和后续脚本边界；本文不执行测试，也不新增脚本。

## 3. 非目标

- 不把 MQTT、governance、LocalAPI、ConPTY 的实现问题混在一个结论里。
- 不用 `ping` 成功替代 shell 成功；`ping` 只作为 shell 前置链路 gate。
- 不用 desktop fake tests 替代真机 shell tests。
- 不在本文里新增产品代码、测试脚本或 OpenSpec 变更。

## 4. 当前代码事实

- `miopunch sh ls <peer_id> [target]` 是非交互 task，可以配合 `--format json --report <path>` 收集机器可读证据。
- `miopunch sh <peer_id> [target] [-s session] [-u|-t|--p2p-network ...]` 是交互式 shell，不支持 `--format json`，但支持 `--report <path>`。
- `sh` 支持 `-u` / `-t`，分别映射到 `udp_only` / `tcp_only`。
- Windows 被控端 target 来源：
  - `wsl:<distro>` 来自 `wsl.exe -l -q`。
  - `ssh:<host>` 来自 Windows 用户的 `~/.ssh/config`。
- Linux 被控端 target 当前为 `local`，依赖本机 `tmux`。
- attach 默认 session 为 `main`。
- `sh_attach` task 创建后，LocalAPI WebSocket 必须在约 30 秒内接入，否则 task 超时。
- shell 日志关键点包括：
  - `sh_attach ready`
  - `shell websocket attached`
  - `first local websocket data`
  - `first local websocket winsize`
  - `first remote stream data`
  - Windows `conpty first read returned`
  - Windows `conpty first write returned`

## 5. 测试环境约定

后续脚本应统一使用隔离根目录，避免污染用户日常 daemon：

```bash
export MIO_REALTEST_ROOT=/tmp/miopunch-wsl2-windows-shell
export MIO_LINUX_BIN="$MIO_REALTEST_ROOT/bin/miopunch-linux"
export MIO_WINDOWS_EXE='C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-shell\bin\miopunch.exe'
export MIO_LINUX_LOCALAPI="unix:$MIO_REALTEST_ROOT/run/linux/localapi.sock"
export MIO_LINUX_STATE="$MIO_REALTEST_ROOT/run/linux/state.json"
export MIO_REPORTS="$MIO_REALTEST_ROOT/reports"
export MIO_ARTIFACTS="$MIO_REALTEST_ROOT/artifacts"
```

Windows 路径必须用单引号保护，避免从 WSL2 调用 `.exe` 时把 `C:\...` 变成无效路径。

## 6. 证据标准

每个测试 case 至少记录：

- 测试 id、方向、发起端、被控端、target、session、transport 约束。
- 双方 peer id、net id、governance role 摘要。
- 发起端 CLI stdout/stderr。
- 发起端 task report。
- 双方 daemon log。
- 测试前后的 `topology --format json`。
- 测试前后的 `ls --format json`。
- 对 shell attach，还要记录输入 marker、输出 marker、resize marker、退出方式和 task final reason。

fresh connectivity / shell 测试必须额外确认：

- 没有 `session_reused=true`，除非该 case 专门验证 reuse。
- report 中有符合预期的 `attempt_path`、`path_family`、`data_proto`。
- `hello=ok` 或 shell attach 等价握手成功。
- 如果失败，必须归类到明确阶段，而不是笼统写“shell 不通”。

## 7. 测试分组

### G0. Preflight 与环境清理

目的：先确认测试不是被旧进程、旧 state、broker、tmux 或 target 配置污染。

应覆盖：

- Windows 侧无旧 `miopunch.exe up` / desktop 残留。
- WSL2 侧无旧 Linux daemon 残留。
- Windows daemon 用非 TTY 前台方式启动，启动后必须用 Windows CLI `ls` 验证 LocalAPI 可用。
- Linux daemon 用前台长进程方式启动，启动后必须用 Linux CLI `ls` 验证 LocalAPI 可用。
- 双方 `local.mqtt_broker` 一致，优先使用当前已验证可达的 broker。
- Linux 有 `tmux`。
- Windows 有 `wsl.exe`。
- 如果要测 `ssh:<host>`，Windows `ssh <host> -- tmux -V` 必须成功。

失败归类：

- `DAEMON_NOT_RUNNING` 属于 LocalAPI / daemon 启动问题。
- `tmux: command not found` 属于 shell target 前置依赖问题。
- MQTT timeout 属于 signaling 问题，不进入 shell attach 结论。

### G1. Fresh network / membership bootstrap

目的：为后续 shell 测试建立干净网络，避免 `approve_decl issuer is not an admin` 这类治理状态污染。

应覆盖：

- Windows 或 Linux 任选一侧 fresh `init-network`。
- admin 侧 `invite --mode approve`。
- member 侧 `join`。
- admin 侧 `approve`。
- 双方 `ls --format json` 都能看到对方 peer。
- 双方 topology 中 governance role 与成员关系符合预期。

通过标准：

- 后续任意方向 `ping` 不再出现 `HELLO_ISSUER_NOT_ADMIN` / `FORBIDDEN`。
- 如果治理失败，停止进入 shell 矩阵，先归类为 membership setup failure。

### G2. Shell 前置 connectivity gate

目的：在 shell 之前先证明 UDP/TCP 链路和 hello/ping 分层正常。

矩阵：

| id | 方向 | 命令类型 | transport | fresh 要求 |
| --- | --- | --- | --- | --- |
| G2-01 | Linux -> Windows | `ping` | `-u` | 是 |
| G2-02 | Linux -> Windows | `ping` | `-t` | 是 |
| G2-03 | Linux -> Windows | `ping` | auto | 是 |
| G2-04 | Windows -> Linux | `ping` | `-u` | 是 |
| G2-05 | Windows -> Linux | `ping` | `-t` | 是 |
| G2-06 | Windows -> Linux | `ping` | auto | 是 |

通过标准：

- `reason_code=OK`。
- `hello=ok`、`ping=ok`。
- `-u` 的 `path_family=udp4` 且 `data_proto=quic` 或当前配置的 UDP dataplane。
- `-t` 的 `path_family=tcp4` 且 `data_proto=tls`。
- fresh case 不允许 `session_reused=true`。

### G3. Shell discovery

目的：覆盖 `sh_ls` 的真实远端 target / session 枚举。

矩阵：

| id | 方向 | target | transport | 期望 |
| --- | --- | --- | --- | --- |
| G3-01 | Linux -> Windows | 空 | auto | 返回至少一个 `wsl:<distro>` 或配置好的 `ssh:<host>` |
| G3-02 | Linux -> Windows | 空 | `-u` | 与 auto 等价的 target 发现 |
| G3-03 | Linux -> Windows | 空 | `-t` | 与 auto 等价的 target 发现 |
| G3-04 | Windows -> Linux | 空 | auto | 返回 `target=local` |
| G3-05 | Windows -> Linux | 空 | `-u` | 返回 `target=local` |
| G3-06 | Windows -> Linux | 空 | `-t` | 返回 `target=local` |
| G3-07 | Linux -> Windows | `wsl:<distro>` | auto | 返回 tmux session 列表或空列表 |
| G3-08 | Linux -> Windows | `ssh:<host>` | auto | 返回 tmux session 列表或空列表 |
| G3-09 | Windows -> Linux | `local` | auto | 返回 tmux session 列表或空列表 |

通过标准：

- CLI JSON/report 中 task 结束为 OK。
- 目标为空时输出 `target=...` facts。
- 目标不为空时输出 `session=...` facts 或明确空 session 列表，不把空列表当失败。
- 失败必须暴露 `shell_layer` 或 remote error suggestion。

### G4. Interactive shell 基础闭环

目的：证明真实 shell 可以输入、输出、退出，而不是只完成 attach 握手。

后续脚本应使用真实 PTY 驱动交互式 CLI，例如 `expect`、Python `pty` 或同等方式。不能用 `--format json`，因为交互式 `sh` 明确不支持 JSON 输出。

矩阵：

| id | 方向 | target | session | transport | 输入 |
| --- | --- | --- | --- | --- | --- |
| G4-01 | Windows -> Linux | `local` | `main` | auto | `printf MIO_SHELL_OK...` 后 exit |
| G4-02 | Windows -> Linux | `local` | `main` | `-u` | 同上 |
| G4-03 | Windows -> Linux | `local` | `main` | `-t` | 同上 |
| G4-04 | Linux -> Windows | `wsl:<distro>` | `main` | auto | `printf MIO_SHELL_OK...` 后 exit |
| G4-05 | Linux -> Windows | `wsl:<distro>` | `main` | `-u` | 同上 |
| G4-06 | Linux -> Windows | `wsl:<distro>` | `main` | `-t` | 同上 |
| G4-07 | Linux -> Windows | `ssh:<host>` | `main` | auto | `printf MIO_SHELL_OK...` 后 exit |

通过标准：

- 本地 CLI stdout 捕获到唯一 marker。
- report 最终 exit code 为 OK 或明确的用户主动关闭语义。
- daemon log 出现 `shell websocket attached`。
- daemon log 出现远端数据首帧证据。
- Windows 被控端 case 出现 `conpty first read returned` 和 `conpty first write returned`，或有明确解释为什么该 target 不经过 ConPTY。

### G5. Terminal 行为细节

目的：覆盖 shell 不是“只会 echo 一行”，而是基本终端行为可用。

case：

- 连续输入多行命令，输出顺序不乱。
- 大输出，例如 1 MiB 文本或 5000 行，不能截断、死锁或提前关闭。
- 二进制安全边界：至少覆盖 tab、退格、方向键或可观测控制序列。
- `Ctrl-C` 中断长命令后 shell 仍可继续输入。
- 发送 EOF / `exit` 后 task 能收尾，CLI 不挂死。
- resize 到 `100x30`、`120x40`、`80x24`，远端 `stty size` 或 `tmux display -p` 能观测到变化。
- 断开本地 CLI 后，远端 tmux session 可重连。

通过标准：

- 每个行为都有 marker。
- 失败要区分为本地 CLI PTY、LocalAPI WS、dataplane stream、远端 PTY/ConPTY、tmux 本身。

### G6. Session reuse 与 fresh 语义

目的：把产品行为和调试行为分开。普通产品可以复用 session，但协议专项测试不能被 reuse 污染。

case：

- 先 `ping -u` 成功，再 `sh -t`，观察是否复用 UDP session。
- 先 `ping -t` 成功，再 `sh -u`，观察是否复用 TCP session。
- 先 `sh ls -u`，再 `sh -t`。
- 同一 target/session 关闭后立即重连。
- daemon 重启后重跑同一 case，确认 fresh 行为。

通过标准：

- reuse 专项 case 必须明确记录 `session_reused=true/false`。
- 如果用户请求 `-u` / `-t` 被已有 session 覆盖，报告必须记录实际 `path_family` 和 `data_proto`。
- fresh 专项 case 必须通过 daemon 重启或显式清 session 保证没有 reuse。

### G7. 生命周期与保活

目的：覆盖“刚连上”和“真实使用一段时间”是两件事。

case：

- shell attach 后空闲 30 秒，再输入 marker。
- shell attach 后空闲超过 2 分钟，再输入 marker。
- shell attach 期间后台 `maintain-neighbors -u` / `-t` 不应破坏 shell。
- 同一 peer 上同时执行 `ping`、`sh ls` 和一个已连接 shell。
- daemon 正常退出时，shell CLI 能退出并有可解释原因。
- 被控端 tmux pane 退出时，发起端 task 能收尾。

通过标准：

- 活跃 shell 不应被 session idle closer 意外关闭。
- 关闭原因必须可解释，不能只留下本地 `unexpected EOF`。

### G8. Negative / failure cases

目的：失败也要可回归，避免以后又只能靠手动猜。

case：

- Linux 被控端缺少 `tmux`，期望 `SH_TMUX_MISSING` 或等价建议。
- Windows `ssh:<host>` 指向的远端缺少 `tmux`。
- Windows `wsl:<missing>` target 不存在。
- `sh ls <peer> missing-target` 返回 target not found。
- LocalAPI 未运行时，CLI 返回 `DAEMON_NOT_RUNNING`。
- `sh` 带 `--format json` 时，CLI 返回明确 bad request。
- WebSocket 30 秒内不接入 `sh_attach` task，返回 timeout。
- Windows daemon 用 TTY 方式启动导致 LocalAPI 不可用时，归类为 daemon 启动方式问题。

通过标准：

- 每个失败都有稳定 reason、stage、facts、suggestions。
- 不允许把依赖缺失误报成 punching/dataplane 失败。

### G9. Desktop GUI 真机 smoke

目的：CLI shell 通过后，再确认桌面真实 terminal bridge 没有回归。

case：

- Windows GUI 控制 Linux `local/main`。
- Linux GUI 控制 Windows `wsl:<distro>/main`。
- Linux GUI 控制 Windows `ssh:<host>/main`。
- GUI disconnect 后 reconnect。
- GUI 运行中 daemon 重启，UI 能进入可恢复状态。

通过标准：

- UI 看到远端首帧输出。
- UI 输入能到达远端。
- resize 能到达远端。
- 关闭/重连不需要重启整个 app。
- task report 和 daemon log 与 CLI case 一致。

### G10. Stress / soak

目的：捕捉只在重复运行或长时间使用后出现的问题。

case：

- 连续 20 次 `sh ls`。
- 连续 20 次短 shell attach / echo / exit。
- 一个 shell 保持 30 分钟，每 60 秒输入 marker。
- 两个发起端同时 attach 同一 peer 的不同 session。
- 两个发起端同时 attach 同一 peer 的同一 session，期望锁或 tmux 语义稳定。
- shell attach 期间网络短暂抖动或 broker 不可达，已经建立的 shell 不应依赖 MQTT payload relay。

通过标准：

- 无 goroutine/process 泄漏迹象。
- 无持续增长的 stale session。
- Windows 无残留 `ssh.exe` / `wsl.exe` / `conhost.exe` 子进程。
- Linux 无残留 tmux client 或 orphaned CLI。

## 8. 建议脚本边界

后续如果落脚本，建议按真实测试分层，而不是写一个超大脚本：

- `scripts/realtest/wsl2_windows_shell/00-preflight.sh`
- `scripts/realtest/wsl2_windows_shell/01-bootstrap-network.sh`
- `scripts/realtest/wsl2_windows_shell/02-connectivity-gate.sh`
- `scripts/realtest/wsl2_windows_shell/03-shell-discovery.sh`
- `scripts/realtest/wsl2_windows_shell/04-shell-attach-basic.sh`
- `scripts/realtest/wsl2_windows_shell/05-terminal-behavior.sh`
- `scripts/realtest/wsl2_windows_shell/06-session-lifecycle.sh`
- `scripts/realtest/wsl2_windows_shell/07-negative.sh`
- `scripts/realtest/wsl2_windows_shell/08-gui-smoke.md`
- `scripts/realtest/wsl2_windows_shell/lib/common.sh`
- `scripts/realtest/wsl2_windows_shell/lib/windows.ps1`

脚本原则：

- 每个脚本都能单独运行。
- 每个脚本都写入独立 artifact 子目录。
- 每个 case 都有 timeout。
- 所有 Windows 命令从 WSL2 发起时都必须保留原始 stdout/stderr。
- 失败时停止当前批次，但保留 daemon log 和 state。
- 默认不杀用户日常 daemon，除非显式传入 `MIO_REALTEST_KILL_EXISTING=1`。

## 9. 推荐执行顺序

第一轮只跑最小闭环：

1. G0 preflight。
2. G1 fresh network / membership。
3. G2-01、G2-02、G2-04、G2-05。
4. G3-01、G3-04、G3-07、G3-09。
5. G4-01、G4-04。

第二轮补 transport 与 target：

1. G4 全矩阵。
2. G5 resize / 大输出 / Ctrl-C。
3. G6 reuse 专项。

第三轮补产品体验：

1. G7 生命周期。
2. G8 negative。
3. G9 GUI smoke。
4. G10 soak。

## 10. 开放问题

- 是否需要给 `sh` 增加非交互测试模式，例如 `miopunch sh exec <peer> <target> -- <cmd>`，以降低 PTY 驱动脚本复杂度。
- 是否需要给 `sh` 增加 `--fresh` / `--no-reuse`，避免协议专项测试被 session reuse 污染。
- 是否需要把 shell 首帧、winsize、PTY/ConPTY 证据结构化进 task report，而不是只存在 daemon log。
- 是否需要为 Windows 自定义 LocalAPI npipe 修复后，再把 Windows daemon 完全隔离纳入默认脚本。
- GUI smoke 是否应进入常规 gate，还是仅作为 release 前真机 gate。
