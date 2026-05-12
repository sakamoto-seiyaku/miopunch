# 2026-05-12 Shell Related Issues

> 状态：问题记录。
>
> 当前结论：问题 1 已解决；问题 2 的早期 `idle_timeout` 已由
> `shell-session-activity-accounting` 修复；前端 terminal DOM 生命周期问题也已修复。
> 最后一段卡点已在 2026-05-13 闭环：Windows `startConPTY` 创建子进程时，
> pseudoconsole attribute、`CREATE_NO_WINDOW`、以及 std handle 继承语义组合错误，
> 导致 `ssh.exe` / `cmd.exe` 输出没有进入 Miopunch 的 ConPTY output pipe。
> 修复后 `debug-conpty-smoke` 的 `cmd` / `ssh-printf` / `ssh-tty` / `ssh-tmux`
> baseline 均能读到首帧输出，用户现场确认 GUI `ssh:ale/main` shell attach
> 已恢复正常。

## 问题 1：`ssh:<name>` shell target 可 discover，但实际 attach 因远端缺少 `tmux` 失败（已解决）

### 现象

- Linux 侧发起 `sh_attach`，任务 `FRB6PL2LHTK74W4TAH4W4UQJOM` 最终失败。
- Linux task report 明确记录：
  - `reason_code=SH_CONNECTOR_FAIL`
  - `shell_layer=ssh`
  - `shell_close=ssh process exited: process exited: 127`
- Windows acceptor 日志也记录到同一失败：
  - `pocacceptor sh_attach runtime failed ... shell_layer=ssh err=process exited: 127`
- 但 desktop shell 页面在 discover 阶段仍然能看到 `ssh:ale` 这样的 target，容易误判为“目标已经可用，只是 attach 后段还有别的问题”。

### 原因分析

- Windows 上的 `ssh:<name>` target 实现位于 [internal/shelltarget/tmux_windows.go](/home/js/Git/miopunch/internal/shelltarget/tmux_windows.go:1)。
- 对 `ssh:<name>`，当前实现是直接在远端执行：
  - session 枚举：`ssh <name> -- tmux list-sessions -F "#S"`
  - attach：`ssh -tt <name> tmux new -A -s <session>`
- 也就是说，这个 target 的真实要求不是“Windows 本机装 `tmux`”，而是：
  - `ssh:<name>` 指向的远端落点必须能直接执行 `tmux`
- desktop 里的空 target discover 只是先列出本机可见的 SSH alias，并不校验远端 `tmux` 是否存在，因此会出现：
  - discover 成功
  - 真实 attach 或列 session 时才以 `127` 失败
- 现场复现也验证了这一点：
  - `ssh ale tmux -V`
  - 返回：`zsh:1: command not found: tmux`

### 解决结果

- 在 `ale` 侧安装/修复 `tmux` 后，这一类 `process exited: 127` 不再出现。
- 因此问题 1 的根因已经确认并解决。

## 问题 2：shell attach 已进入 `ready`，但 UI 无输出且无法输入（前端 terminal DOM 被重建）

### 现象

- 历史任务 `H7ZG2DEIFVY5CP5ME5MCZ6P6WE` 曾经表现为 attach 已 ready 后最终
  `idle_timeout`。这属于 dataplane session activity accounting 问题。
- `shell-session-activity-accounting` 后，新的现场不再能用 `idle_timeout` 解释。
- 最新任务 `Y33AHL2BCLN5LSOTQJ62VUN63A` 显示 Linux 侧已走到：
  - `CapabilityHandshake: shell attach request`
  - `SessionAttach: waiting for local websocket`
  - `SessionAttach: shell websocket attached`
- Windows acceptor 同时显示：
  - `pocacceptor sh_attach ready: task_id=Y33AHL2BCLN5LSOTQJ62VUN63A ...`
- 此后没有对应的 `idle_timeout` / `task done` 作为当前直接根因。
- 用户在 Windows 手工验证：
  - `ssh -vvv -tt ale tmux new -A -s main` 完全正常。
  - `tmux list-clients` 能看到 `/dev/pts/0 session=main term=xterm-256color size=120x30`。
  - `tmux list-panes` 能看到 `%0 tty=/dev/pts/1 cmd=zsh dead=0 size=120x29`。
  - `tmux send-keys ... echo __MIO_REMOTE_TO_UI__` 后，`capture-pane` 能看到输出。
- 但 miopunch UI 仍然没有显示 remote 输出，也无法输入。

### 已确认边界

- 这已经不是问题 1 的 `tmux missing` / `process exited: 127`。
- 这也不是 discover 失败，或者 attach 之前的早期握手失败。
- `ssh`、远端 `tmux`、远端 pane、winsize 都能单独工作。
- `sh_attach` 任务与 LocalAPI shell websocket 已经挂上。
- 旧的 `localapi_ws 1006 unexpected EOF` 只能说明后续本地 WS 被关闭，不足以解释最初
  “UI 空白且不能输入”的第一故障点。

### 原因分析

- 前端 shell 页当前由 [cmd/miopunch-desktop/frontend/dist/assets/app.js](/home/js/Git/miopunch/cmd/miopunch-desktop/frontend/dist/assets/app.js:1022)
  的 `setPage()` 整页渲染：
  - `host.innerHTML = html`
- shell 页模板每次都会创建新的：
  - `<div class="terminal mt" id="terminal"></div>`
- runtime `desktop:state` / `desktop:runtime` 事件会触发 `scheduleRender()`，从而重建当前 shell 页 DOM。
- `openTerminal()` 已经把 xterm 挂到旧的 `#terminal`，并保存到 `shellState.term`。
- 一旦 shell 页被 runtime 更新重渲染：
  - 旧的 xterm DOM 被 `innerHTML` 销毁。
  - 新页面出现一个空的 `#terminal`。
  - `shellState.term` 和 WebSocket 仍然存在，但它们写入的是旧 terminal 对象。
  - 键盘焦点也不再落在可见 xterm textarea 上。
- 因此现场会表现为：
  - remote/tmux 侧确实有输出。
  - UI 看不到输出。
  - UI 也无法输入。
  - 后续 `1006 unexpected EOF` 更像是结果，不是第一原因。

### 后续

- 修复方向是在 shell 页重渲染时保留并 reparent 已打开的 xterm DOM，而不是重建一个空
  terminal 容器。
- 同时保留现有行为：离开 shell 页或切换 peer 时仍主动断开 shell transport。

## 备注

- 当前日志里还存在另一类独立问题：`CandidateExchange` / MQTT barrier timeout。
- 这一类问题不属于本文的 shell attach 主线问题，后续应单独记录和分析，避免与上面两个问题混在一起。

## 问题 3：`sh_attach ready` 后 UI 等待远端输出，Windows ConPTY 无首帧 read（继续排查）

### 现象

- 最新现场里 GUI 已显示 xterm，状态停在：
  - `Terminal bridge connected. Waiting for shell output from ssh:ale/main...`
- terminal 内只显示前端本地写入的 `Connecting...`，没有远端 shell 首屏输出。
- Linux 侧日志到达：
  - `kind=sh_attach stage=SessionAttach message=shell websocket attached`
- Windows 侧日志到达：
  - `pocacceptor sh_attach ready ... target=ssh:ale session=main`
- Windows 进程现场显示：
  - `miopunch.exe` 子进程已启动 `ssh.exe ssh -tt ale tmux new -A -s main`
- 远端 `ale` 现场显示：
  - 存在对应 `sshd: js@pts/0`
  - 存在对应 `tmux new -A -s main`
- Linux/UI -> Windows 输入方向已有日志证明可达；Windows -> Linux heartbeat
  JSON 也可达。
- 但 Windows 侧始终没有：
  - `pocacceptor sh_attach first pty read`
  - `pocacceptor sh_attach first stream data write`

### 当前原因边界

- `pocacceptor sh_attach ready` 只证明 Windows acceptor 已启动 shell target，并把 OK control 发回 Linux task bridge。
- 它不证明：
  - Windows `ptySess.Read` 已经从 ConPTY/ssh/tmux 读到任何输出。
  - Windows acceptor 已把 PTY bytes 写成 `shellproto.KindData`。
  - Linux `bridgeShell` 已从 remote stream 读到 `KindData`。
  - LocalAPI WS 或 desktop terminal bridge 已把数据写到浏览器。
- 因此当前不能再把“UI 仍是 `Connecting...` / 等待 shell output”归因到前端
  DOM 生命周期、desktop bridge、LocalAPI WS 或 dataplane 主链路。
- 当前已确认卡点在 Windows 被控端 `shelltarget` / ConPTY output read 边界：
  `ssh.exe` 和远端 `tmux` 进程存在，但 Windows `ptySess.Read` 没有读到任何
  terminal bytes。

### 当前定位程度

- 已确认的直接原因：
  - Windows ConPTY output pipe 没有产出可由 `ptySess.Read` 读出的首帧数据。
- 尚未 100% 确认的具体代码根因：
  - `PSEUDOCONSOLE_INHERIT_CURSOR` 已经被 A/B 排除为单点根因：最新日志显示
    `conpty_flags=0`，但问题仍复现。
  - `CREATE_NO_WINDOW` 也已被 A/B 排除为“可直接删除”的修复点：删除后
    `ssh.exe` 在 Windows 侧立即以 `0xC0000142` / `STATUS_DLL_INIT_FAILED`
    退出，`sh_attach` 直接变成 `SH_CONNECTOR_FAIL`。
  - 当前剩余嫌疑：
    - `CreatePseudoConsole` 收到的 `inRead` / `outWrite` 在 `CreateProcess`
      之前就被本进程关闭。
    - Windows GUI/session 场景下 `ssh.exe` 需要 `CREATE_NO_WINDOW` 才能稳定初始化，
      但仍可能需要更接近官方流程的 pipe handle 生命周期。
    - `ssh.exe` 在 ConPTY host 模式下存在额外初始化依赖，需要用 Windows-only
      ConPTY smoke 与 Miopunch sh_attach 桥接拆开验证。
- 官方参考：
  - Microsoft 的 pseudoconsole walkthrough 要求先建立同步通信 channels，再调用
    `CreatePseudoConsole`，然后用 `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` 和
    `EXTENDED_STARTUPINFO_PRESENT` 创建子进程；并明确说明传给 pseudoconsole
    创建的 pipe handle 应在 child process `CreateProcess` 完成后再释放。
    参考：
    <https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session>
  - Microsoft `CreatePseudoConsole` 文档说明 `dwFlags=0` 是标准 pseudoconsole
    创建；`PSEUDOCONSOLE_INHERIT_CURSOR` 需要额外异步 cursor query 处理。
    参考：
    <https://learn.microsoft.com/en-us/windows/console/createpseudoconsole>
  - Microsoft process creation flags 文档说明 `CREATE_NO_WINDOW` 会让 console
    application 在没有 console window 的情况下运行，且 console handle 不会被设置。
    这与 pseudoconsole 要把 console app 关联到指定 ConPTY 的目标存在明显冲突风险。
    参考：
    <https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags>

### 本轮补充

- 在 Windows acceptor、Linux task bridge、desktop terminal bridge 增加 bounded 首帧诊断日志。
- 日志只记录 `task_id`、peer/target/session、方向、frame kind、bytes，不记录 terminal payload。
- 前端状态也同步修正：
  - WebSocket open 后显示等待 shell output。
  - 收到第一帧远端数据后才显示 `Connected to <target>/<session>`。
- 在 Windows ConPTY backend 增加 bounded 诊断日志：
  - `conpty create start`
  - `conpty process started`
  - `conpty resize start/done`
  - `conpty read wait start`
  - `conpty first read returned`
  - `conpty first write returned`
- 将 `CreatePseudoConsole` flags 从 `PSEUDOCONSOLE_INHERIT_CURSOR` 改为 `0`
  做 A/B 验证。
- A/B 结果：`flags=0` 后仍无 `first pty read`，所以 cursor inherit 不是当前
  唯一根因。

### 2026-05-13 最新证据链

- Linux desktop bridge：
  - `terminal bridge backend websocket attached: task_id=MEUQBYWG2BDALT44WTB52MBWXA`
  - `terminal bridge frontend websocket attached`
  - `terminal bridge first frontend to backend frame ... bytes=49`
- Linux task bridge：
  - `SessionAttach: shell websocket attached`
  - `sh_attach bridge first local websocket winsize ... size=134x24`
  - `sh_attach bridge first remote stream json write ... bytes=49`
  - `sh_attach bridge first remote stream json ... op=heartbeat`
  - 没有 `sh_attach bridge first remote stream data`
- Windows acceptor / shelltarget：
  - `conpty create start: application=ssh size=80x24 flags=0`
  - `conpty process started: pid=45544 ... command_line="ssh -tt ale tmux new -A -s main"`
  - `conpty read wait start: pid=45544`
  - `pocacceptor sh_attach ready: task_id=MEUQBYWG2BDALT44WTB52MBWXA`
  - `pocacceptor sh_attach first visitor json ... op=winsize`
  - `conpty resize done: pid=45544 size=134x24 err=<nil>`
  - 没有 `conpty first read returned`，也没有 `pocacceptor sh_attach first pty read`
- Windows 现场进程：
  - `miopunch.exe` 子进程存在 `ssh.exe pid=45544`
  - `ssh.exe pid=45544` 子进程存在 `conhost.exe`
- 远端 `ale` 现场：
  - 存在 `sshd: js@pts/0`
  - 存在 `tmux new -A -s main`
  - `tmux list-clients` 只看到 `/dev/pts/0 term=xterm-256color size=120x30`
- 推论：
  - Linux/UI -> Windows 的 winsize JSON 已经到达 Windows acceptor，并触发了
    `ResizePseudoConsole(134x24)`。
  - Windows -> Linux 的 heartbeat JSON 已经到达 Linux task bridge。
  - 但 terminal data 方向仍未出现首包。
  - 远端 tmux client size 仍是 `120x30`，没有变成 Miopunch UI 发送的
    `134x24`，这支持“`ssh.exe` 没有正确绑定到 Miopunch 创建的 ConPTY console
    链路”这一假设。

### 如果当前 ConPTY 创建根因成立，修复思路

- 调整 Windows `startConPTY` 创建顺序，使其贴近 Microsoft pseudoconsole
  walkthrough：
  - `CreatePipe`
  - `CreatePseudoConsole(..., flags=0, &pcon)`
  - 准备 `STARTUPINFOEX` / `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  - `CreateProcess` 子进程，creation flags 暂时保留
    `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW`
  - `CreateProcess` 成功后再关闭本进程持有的 `inRead` / `outWrite`
  - 保留 `inWrite` / `outRead` 作为 Miopunch 与 ConPTY 通信的两端
- 保留现有 bounded 诊断日志，用下一轮现场验证：
  - 是否出现 `conpty first read returned bytes>0`
  - 是否出现 `pocacceptor sh_attach first pty read`
  - 是否出现 `sh_attach bridge first remote stream data`
  - 远端 `tmux list-clients` size 是否变为 UI winsize，例如 `134x24`
- 如果上述改动后仍不出首包，再增加一个 Windows-only ConPTY smoke：
  - 本地 `cmd.exe /c echo __MIO_CONPTY_SMOKE__`
  - Windows `ssh -tt ale "printf ..."`
  - 用它把“Miopunch sh_attach 桥接问题”和“Windows ConPTY + ssh.exe
    单机创建问题”彻底拆开。

### 2026-05-13 修复尝试

- 已在 Windows `startConPTY` 路径调整创建流程：
  - `CreatePseudoConsole(..., flags=0, &pcon)` 保持不变。
  - `CreateProcess` flags 从
    `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW`
    改为 `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT`。
  - `inRead` / `outWrite` 不再在 `CreatePseudoConsole` 后立即关闭，而是延后到
    `CreateProcess` 成功后关闭。
  - `CreateProcess` 前任一失败路径会同时关闭 `inRead`、`inWrite`、`outRead`、
    `outWrite` 并关闭 pseudoconsole。
- 本地验证已通过：
  - `GOOS=windows GOARCH=amd64 go test -c -o /tmp/miopunch-shelltarget.test.exe ./internal/shelltarget`
  - `go test ./internal/shelltarget ./internal/pocacceptor ./internal/task`
  - `go test ./...`
  - `go vet ./...`
  - `bash scripts/check_no_xtcp_imports.sh`
- 仍需现场验证：
  - 重新构建并在 Windows 侧运行。
  - 如果出现 `conpty first read returned bytes>0`，则此轮根因基本闭环。
  - 如果仍无 `first pty read`，下一步应加 Windows-only ConPTY smoke，把
    `cmd.exe` / `ssh.exe` 单机 ConPTY 行为与 Miopunch sh_attach 桥接链路拆开。

### 2026-05-13 现场结果：移除 `CREATE_NO_WINDOW` 后的新失败

- 用户重新构建并保留 `data/` 后复测，出现两次 `sh_attach`：
  - `BUV5COH4WC6X5OND3Q4V4UOLZU`
  - `X74O6WB7G6ZM7CKDVVEWNV454Y`
- Linux task report：
  - 两次都停在 `CapabilityHandshake: shell attach request`
  - 两次都是 `reason_code=SH_CONNECTOR_FAIL`
  - facts 都是 `ssh process exited: process exited: 3221225794`
- Windows acceptor / shelltarget：
  - `conpty create start: application=ssh size=80x24 flags=0`
  - `conpty process started ... create_flags=525312 ... command_line="ssh -tt ale tmux new -A -s main"`
  - `conpty read wait start`
  - 约 3ms 后 `pocacceptor sh_attach setup runtime failed ... err=process exited: 3221225794`
  - 随后 `conpty first read returned ... bytes=0 err=read conpty-out: file already closed`
- `3221225794 = 0xC0000142 = STATUS_DLL_INIT_FAILED`。
- 结论：
  - 这不是旧的 “ssh/tmux 存活但 ConPTY 无输出” 卡点。
  - 这是新补丁引入的更早失败：`ssh.exe` 进程启动后立即初始化失败。
  - `create_flags=525312` 说明此轮运行已经移除了 `CREATE_NO_WINDOW`。
  - 因此下一步不应继续沿着“删除 `CREATE_NO_WINDOW`”推进，而应恢复
    `CREATE_NO_WINDOW`，只保留 “`inRead` / `outWrite` 延后到 `CreateProcess`
    成功后关闭” 做下一轮 A/B。

### 2026-05-13 下一轮 A/B 代码状态

- 已恢复 `CREATE_NO_WINDOW`，避免 `ssh.exe` 立即 `STATUS_DLL_INIT_FAILED`。
- 保留上一轮的 handle 生命周期修正：
  - `inRead` / `outWrite` 仍延后到 `CreateProcess` 成功后关闭。
  - 失败路径仍补齐 `inRead`、`inWrite`、`outRead`、`outWrite` 和
    pseudoconsole 清理。
- 这一轮要验证的单点变成：
  - 在保留 `CREATE_NO_WINDOW` 的前提下，仅修正 pipe handle 生命周期，是否能让
    ConPTY output pipe 产出首帧。

### 2026-05-13 现场结果：保留 `CREATE_NO_WINDOW` + 延后关闭 pipe handle

- 最新任务：`3UBXYK5WJWSPDYREB3WQMBRKHY`。
- Linux desktop bridge：
  - `terminal bridge backend websocket attached`
  - `terminal bridge frontend websocket attached`
  - `terminal bridge first frontend to backend frame ... bytes=49`
- Linux task bridge：
  - `SessionAttach: shell websocket attached`
  - `sh_attach bridge first local websocket winsize ... size=134x24`
  - `sh_attach bridge first remote stream json write ... bytes=49`
  - `sh_attach bridge first remote stream json ... op=heartbeat`
  - 用户输入后出现 `sh_attach bridge first local websocket data ... bytes=1`
  - 并出现 `sh_attach bridge first remote stream data write ... bytes=1`
  - 仍没有 `sh_attach bridge first remote stream data`
- Windows acceptor / shelltarget：
  - `conpty create start: application=ssh size=80x24 flags=0`
  - `conpty process started: pid=26228 ... create_flags=134743040 ... command_line="ssh -tt ale tmux new -A -s main"`
  - `conpty read wait start: pid=26228`
  - `pocacceptor sh_attach ready`
  - `pocacceptor sh_attach first visitor json ... op=winsize`
  - `conpty resize done: pid=26228 size=134x24 err=<nil>`
  - 用户输入后出现 `pocacceptor sh_attach first visitor data ... bytes=1`
  - `conpty first write returned: pid=26228 bytes=1 requested=1 err=<nil>`
  - `pocacceptor sh_attach first pty write ... bytes=1`
  - 仍没有 `conpty first read returned`，也没有 `pocacceptor sh_attach first pty read`
- 结论：
  - 恢复 `CREATE_NO_WINDOW` 后，`ssh.exe` 不再 `STATUS_DLL_INIT_FAILED`，回到旧卡点。
  - 延后关闭 `inRead` / `outWrite` 没有让 ConPTY output pipe 产出首帧。
  - 输入方向已经完整闭环：UI -> Linux task bridge -> Windows acceptor ->
    ConPTY input pipe，且 `Write` 返回成功。
  - 当前唯一未闭环方向是：`ssh.exe` / ConPTY output pipe -> Windows `Read`。
  - 下一步不应继续猜 bridge/dataplane/frontend，应增加 Windows-only ConPTY smoke，
    直接验证同一 `startConPTY` 能否从本地 `cmd.exe` 或 `ssh.exe` 读出 output。

### 下一次现场判定

- 有 `pocacceptor sh_attach first pty read`：Windows ConPTY 已读到远端输出。
- 有 `pocacceptor sh_attach first stream data write`：Windows 已把 PTY 输出写入 shell stream。
- 有 `sh_attach bridge first remote stream data`：Linux 已收到 remote data frame。
- 有 `sh_attach bridge first local websocket data write`：Linux 已写给 LocalAPI WS。
- 有 `terminal bridge first backend to frontend frame`：desktop bridge 已把数据转发给浏览器。
- 如果停在某一条之前，卡点就在上一段到这一段之间。

### 2026-05-13 新增诊断入口：Windows-only ConPTY smoke

- 新增隐藏命令：
  - `miopunch debug-conpty-smoke cmd`
  - `miopunch debug-conpty-smoke ssh-printf ale`
  - `miopunch debug-conpty-smoke ssh-tty ale`
  - `miopunch debug-conpty-smoke ssh-tmux ale main`
  - JSON 格式示例：
    `miopunch --format json debug-conpty-smoke --timeout 10s ssh-tmux ale main`
- 这个入口直接调用生产路径里的 Windows `startConPTY`，但不经过 desktop bridge、
  LocalAPI websocket、dataplane、Linux task bridge、Windows acceptor 的 shell
  stream 转发逻辑。
- 输出只记录诊断证据，不记录完整 terminal payload：
  - 实际 `application` / `args` / `command_line`
  - Windows child `pid`
  - `read_returned` / `read_timed_out` / `read_n` / `read_err`
  - `write_attempted` / `write_returned` / `write_n` / `write_err`
  - `wait_returned` / `wait_err`
  - 首帧 preview 的 bounded text/hex
- 实现上特意处理了短命令竞争：
  - `cmd /c echo` 或 `ssh ... printf` 可能先触发 `Wait()`，再由 pipe reader
    返回 output。
  - 因此 smoke 不会在 `Wait()` 先返回时立刻关闭 ConPTY，而是继续等首帧
    `Read()`，避免把健康的短命令误判成无输出。

### ConPTY smoke 判定逻辑

- `cmd` 都没有 `read_n>0`：
  - 卡点在本机 ConPTY baseline，优先查 `CreatePseudoConsole` / pipe handle /
    `CreateProcess` flags / output pipe read。
- `cmd` 有输出，但 `ssh-printf` 没有 `read_n>0`：
  - Miopunch bridge/dataplane/frontend 基本排除；卡点收敛到 Windows OpenSSH
    在该 ConPTY host 模式下的启动或输出行为。
- `ssh-printf` 有输出，但 `ssh-tty` 输出异常：
  - 重点看远端是否真的获得 PTY，以及 `stty size` / `tty` 是否符合预期。
- `ssh-tty` 有输出，但 `ssh-tmux` 没有 `read_n>0`：
  - 卡点收敛到 `tmux new -A -s main` 在该 ConPTY + OpenSSH attach 场景下
    是否产生首屏输出、是否等待输入、是否被已有 tmux client 状态影响。
- `ssh-tmux` 在 smoke 有输出，但 GUI attach 仍无 `pocacceptor sh_attach first pty read`：
  - 说明同一 ConPTY 生产 backend 在单机 smoke 能工作，下一步应比较 GUI
    acceptor 启动环境、cwd/env/session mode、以及 acceptor 的 attach 生命周期差异。

### 2026-05-13 WSL 直接运行 Windows exe 的阶段结论

- WSL 可以直接执行 extracted 里的 Windows `miopunch.exe`，因此后续可以用
  `debug-conpty-smoke` 在本机形成快速闭环。
- 原始代码状态（`CREATE_NO_WINDOW` + `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`
  传 `&pcon`）下，`cmd.exe /c echo` baseline 也失败：
  - child `cmd.exe` 约 150ms 正常退出。
  - `read_timed_out=true`
  - `read_n=0`
  - `read_err=read conpty-out: file already closed`
- 单独修正 pseudoconsole attribute 传值后，如果仍保留 `CREATE_NO_WINDOW`，
  `cmd` baseline 仍然 `read_n=0`。因此 attribute 传值不是唯一问题。
- 修正 pseudoconsole attribute 传值，并移除 `CREATE_NO_WINDOW` 后：
  - `cmd` / `ssh-printf` / `ssh-tty` / `ssh-tmux` 都能很快读到 16 bytes：
    `ESC[?9001h ESC[?1004h`。
  - 之前移除 `CREATE_NO_WINDOW` 时看到的 `ssh.exe 0xC0000142` 不再复现，
    说明它更像是“错误 pseudoconsole attribute + 无 `CREATE_NO_WINDOW`”的组合故障，
    不是 `ssh.exe` 必须保留 `CREATE_NO_WINDOW`。
- 但这还不是完整成功：
  - `__MIO_CONPTY_CMD__` / `__MIO_CONPTY_SSH__` marker 出现在父 stdout，
    没有进入 smoke 统计的 ConPTY `Read()` 聚合结果。
  - 这说明当前 WSL 直接运行 smoke 时，child 仍可能继承/连接到了父 console/stdout，
    而不是完全由 Miopunch 创建的 pseudoconsole output pipe 接管。
- 已排除的变量：
  - `inheritHandles=true` 没有改善，marker 仍写到父 stdout。
  - 将 child `currentDir` 固定到 `C:\Windows\System32` 只消除了 UNC cwd warning，
    没有让 marker 进入 ConPTY output pipe。
  - `DETACHED_PROCESS` 不成立：`cmd` 直接 `exit 1`，ConPTY output 仍为 0。

### 2026-05-13 根因闭环

- 关键修正组合：
  - `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` 必须传 pseudoconsole handle 值本身，
    不能传 Go 局部变量 `&pcon` 的地址。
  - `CreateProcess` 不能带 `CREATE_NO_WINDOW`，否则 `cmd.exe` baseline 都不会进入
    ConPTY output pipe。
  - `STARTUPINFOEX.StartupInfo.Flags` 必须设置 `STARTF_USESTDHANDLES`，且 std
    handles 保持空值，避免 WSL/console 父进程的 stdout/stderr 被 child 继续使用。
- 最小 baseline 成功证据：
  - `cmd` smoke：`read_n=90`，`preview_text` 包含 `__MIO_CONPTY_CMD__`。
  - `ssh-printf` smoke：`read_n=137`，`preview_text` 包含
    `__MIO_CONPTY_SSH__` 和 `Connection to ... closed`。
  - `ssh-tty` smoke：`read_n=153`，`preview_text` 包含
    `__MIO_CONPTY_TTY__`、`24 80`、`/dev/pts/3`。
  - `ssh-tmux` smoke：`read_n>0`（多次观测到 1322 / 2126），`preview_text`
    包含 tmux alternate-screen control sequence 和远端 zsh prompt / pane 内容。
- 因此当前问题的根因不在：
  - GUI terminal DOM
  - LocalAPI websocket
  - Linux task bridge
  - dataplane stream
  - Windows acceptor shell stream
  - Windows OpenSSH / remote tty / tmux 本身
- 精确根因：
  - Windows `startConPTY` 创建子进程时，process creation flags、pseudoconsole
    attribute 传值、以及 std handle 继承语义组合错误，导致子进程没有把 stdout/stderr
    绑定到 Miopunch 创建的 pseudoconsole output pipe。
  - 旧代码中 `CREATE_NO_WINDOW` 会让 child 不向 ConPTY output pipe 产出 terminal
    bytes；去掉它后，如果不设置 `STARTF_USESTDHANDLES`，child 又会把 output 写到父
    console/stdout，造成 smoke 中 marker “看得见但 `Read()` 读不到”的假阳性。

### 2026-05-13 现场确认：GUI shell attach 恢复正常

- 用户用最新构建重新运行 desktop GUI 后确认 `ssh:ale/main` 已经正常。
- 因此问题 3 已闭环：
  - Windows ConPTY baseline 已恢复。
  - Windows `ssh -tt ... tmux new -A -s main` 输出能进入 Miopunch 的 ConPTY
    output pipe。
  - 后续 Windows acceptor / Linux task bridge / LocalAPI websocket / desktop
    terminal bridge 能把远端 tmux 屏幕输出显示到 GUI。
- 若后续再出现类似现象，第一判定点应先跑：
  - `miopunch debug-conpty-smoke --timeout 5s cmd`
  - `miopunch debug-conpty-smoke --timeout 5s ssh-tmux <host> <session>`
- 只有 smoke 成功而 GUI 失败时，才继续看 `pocacceptor sh_attach first pty read`
  之后的 shell stream / task bridge / desktop bridge 链路。
