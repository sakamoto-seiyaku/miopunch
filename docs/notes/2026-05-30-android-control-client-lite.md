# Android 控制端 D1b-lite 最短方案

日期：2026-05-30

状态：面试展示向方案记录。本文只记录最短可演示路径，不代表正式 Android 产品线已经启动。

## 目标

做一个极简 Android 控制端，让手机作为 operator 连接到其它机器的远端 shell。

这条路线只服务一个面试展示目标：

```text
Android 手机
-> join 现有 miopunch 网络
-> ping 远端 peer
-> 打开远端 Linux/Windows host 暴露的 shell target
```

Android 不作为被控端，不向别人暴露 Android 本机 shell。

## 当前事实

- `cmd/miopunch` 已能交叉编译为 Android arm64 executable：

```bash
export PATH=/usr/local/go/bin:$PATH
CGO_ENABLED=0 GOOS=android GOARCH=arm64 \
  go build -trimpath -ldflags="-s -w" \
  -o /tmp/miopunch-android-arm64 ./cmd/miopunch
```

- 本地检查产物是 `ARM aarch64` PIE executable，大小约 `12M`。
- 当前 `join` / `ping` / `sh` 命令通过本机 `LocalAPI` 连接 `miopunch up` runtime。
- 因此 Android App 即使“不常驻”，也仍需要在 App 生命周期内临时托管一个本机 `miopunch up` 进程。
- App 退出或用户点击 Stop 时，终止本地 runtime 与 shell 子进程即可。

## 2026-05-30 WSL + Android CLI 实测

本次不走 HTTP / APK 壳，直接用 WSL Linux binary 和 Android arm64 binary 做真实互联验证。

环境：

- Linux 端：WSL，`192.168.4.5/24`。
- Android 端：Pixel 6a，`arm64-v8a`，`wlan0=192.168.4.151/24`。
- ADB：Windows `adb.exe` 需要从 WSL 通过 `/init ... adb.exe -- <args>` 调用。
- Android 权限：普通 `adb shell` 可执行 `miopunch --help`；`miopunch up` 需要 `su -c`，否则 Unix socket bind 到 `/data/local/tmp/...` 会遇到 permission denied。
- Broker：本次用临时本地 MQTT broker，并用 `adb reverse tcp:18883 tcp:18883` 让 Android 访问 WSL broker。

已确认成立：

- Android 上直接运行 CLI 没问题，`cmd/miopunch` 的 Android arm64 产物可执行。
- Linux `init-network` 成功。
- Linux 生成 invite、Android `join` 成功。
- Linux `approve` 成功。
- 两端 `ls` 都能看到对端 `online`。
- Android 到 WSL ICMP 通：`ping 192.168.4.5` 0% packet loss。
- 裸 UDP 双向通：
  - Android -> WSL：WSL `nc -u -l -p 49001` 能收到 Android 发来的 payload。
  - WSL -> Android：Android `toybox nc -u -l -p 49002` 能收到 WSL 发来的 payload。
- 裸 TCP 双向通：
  - Android -> WSL：WSL `nc -l -p 49011` 能收到 Android 发来的 payload。
  - WSL -> Android：Android `toybox nc -l -p 49012` 能收到 WSL 发来的 payload。

当前失败点：

- 单向 `miopunch ping` 在 `Punch` 阶段超时。
- Linux -> Android 失败事实：
  - `local_candidates=host@192.168.4.5:48438`
  - `remote_candidates=host@192.168.4.151:43615`
  - `attempt_results=timeout=1`
  - `error=punch failed: make hole ... wait detect message error: context deadline exceeded`
- Android -> Linux 失败事实类似：
  - `local_candidates=host@192.168.4.151:43615`
  - `remote_candidates=host@192.168.4.5:48438`
  - `attempt_results=timeout=1`
- 早先一次双向同时 ping 的日志里，双方 punch 曾经进入 `punch attempt selected`，但后续卡在 `Shell` / `SecureSession`，Android LocalAPI 还出现过需要重启 `up` 才恢复的状态。

初步判断：

- 不是“Android 不能直接调用 CLI”。
- 不是基础 IP 网络不通。
- 不是 WSL 和 Android 之间裸 UDP/TCP 被阻断。
- 当前更像 miopunch v1 punch/session 的运行态问题：单向 dial/accept 的 detect 握手不稳定，且并发双向 ping 后可能污染 Android LocalAPI / session 状态。
- `miopunch ping -t` 当前会把 CLI 参数解析为 `p2p_network=tcp_only`，但 `Runtime.doPing -> ensurePeerSession -> punch.Dial` 这条路径没有实际使用 `PingArgs.P2PNetwork`；所以本次 `-t` 结果不能视为真正 TCP punch 验证。

## 2026-05-30 恢复策略：UDP direct-first

本轮恢复 Android/WSL 可演示路径时，不先做 HTTP 壳、APK 壳或 TCP Door-2。

最短修复路径：

```text
Android/WSL host candidate
-> 先用同一个 UDP socket + traversal demux 做 direct_ipv4 握手
-> direct_ipv4 成功则直接进入 KCP/TLS/yamux session
-> direct_ipv4 超时或失败再回落到现有 punching_ipv4
```

演示验收口径：

- `miopunch ping <peer>` 成功时，CLI JSON/report facts 需要出现 `selected_path=direct_ipv4` 或 `selected_path=punching_ipv4`。
- Android/WSL 同 LAN 演示优先期待 `selected_path=direct_ipv4`。
- `miopunch sh ls <peer>` 成功时同样保留 `selected_path` 证据，但原有目标/会话 line output 不变。
- Punch 阶段失败时，facts 需要能区分 direct timeout 与 punching timeout，方便面试现场解释问题卡在哪个阶段。

明确非目标：

- 本轮不恢复 `ping -t` / `p2p_network=tcp_only` 的真实 TCP 路径。
- 本轮不恢复 Door-2 TCP direct/punching，也不把 TCP 作为 Android/WSL 演示验收条件。
- 本轮不做 APK、HTTP 控制面或 Android 被控端。

对面试展示的影响：

- 可以说 Android CLI 路线已经被实机证明可执行、可入网、可发现对端，并能通过 `direct_ipv4` 完成 Android/WSL `ping` 与 `sh ls`。
- 交互式 shell attach 后续已由 Android control-lite APK 实机确认，手机端可以打开 Linux/WSL 远端 shell 并执行 `whoami`。
- 下一步优先把演示流程和证据固定下来，而不是继续做 HTTP 或完整 Android 产品壳。
- 演示优先级应调整为：先把 CLI 端到端 shell 跑稳，再包一层极薄 Android 控制端。

## 2026-05-30 direct-first 实现后实测

本轮已按 `restore-pocv1-udp-direct-android-wsl-demo` change 恢复 POC v1 UDP direct-first：

- Linux/WSL 与 Android arm64 binary 均从当前 tree 构建。
- controlled broker 使用本地 Docker mosquitto，Android 通过 `adb reverse tcp:18883 tcp:18883` 访问。
- 两端 daemon 均使用 `--log-level trace`。
- `init-network -> invite -> join -> approve -> ls -> ping` 实跑成功。
- Linux -> Android `ping` JSON facts 包含 `selected_path=direct_ipv4`。
- Linux -> Android `sh ls` 实跑成功，JSON facts 包含 `selected_path=direct_ipv4`、`target=local`。
- 双端 daemon trace log 都能看到 `attempt.direct_ipv4.ok`、`punch run selected ... selected_path=direct_ipv4` 和 UDP traversal demux routing 记录。

本次 evidence 保存在：

```text
/tmp/miopunch-restore-pocv1-demo/evidence/
```

关键文件：

- `ping-android.redacted.json`
- `sh-ls-android.redacted.json`
- `linux-miopunch.log`
- `android-miopunch.log`

## 2026-05-31 APK lite 实机验收

本轮已按 `android-control-lite-apk-demo` change 做出极简 Android 控制端 APK，并在 Pixel 6a + Linux/WSL peer 上完成实机演示验证。

已确认成立：

- APK 内打包 Android arm64 `miopunch` payload，App 启动时 `--help` 检查通过。
- App 可以启动和停止 app-lifetime `miopunch up`，LocalAPI 使用 `<cacheDir>/miopunch-localapi.sock`，state 使用 `<filesDir>/state/state.json`。
- App 日志区可以显示 runtime stderr、CLI action stdout/stderr、`reason_code`、`facts`、`suggestions`，调试时不必只看 ADB logcat。
- App 可以通过 invite 加入既有测试网络，`LS` 能看到 Linux/WSL peer。
- Android -> Linux/WSL `Ping` 成功，输出包含 `reason_code=OK` 与 `selected_path=direct_ipv4`。
- Android -> Linux/WSL `Open Shell` 成功，App 内 xterm WebView 可以正确渲染 tmux/zsh ANSI 输出。
- 手机端发送 `whoami`，远端 shell 返回 `js`。

本次关键证据：

```text
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-ping-ok.png
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-shell-whoami-js.png
```

现场演示口径可以调整为：

- “手机是真实控制端，不是截图，也不是 ADB 代打。”
- “Android App 内部仍复用同一个 miopunch CLI/runtime，没有新开 HTTP bridge。”
- “先 `Ping` 证明 identity-bound session/path 可用，再 `Open Shell` 进入远端 Linux shell。”
- “当前实机证据是同 LAN/WSL 演示，`selected_path=direct_ipv4`；跨 NAT/移动网络展示另行准备稳定 broker/host。”

## 非目标

- 不做 Android 被控端。
- 不做后台常驻 daemon、自启动、系统服务或通知栏长期驻留。
- 不做完整入网管理、成员治理、revoke、复杂 peer 管理。
- 不做扫码、联系人、配置中心或 Material 3 完整体验。
- 不重写 `LocalAPI`、shell stream、runtime action 或打洞协议。
- 不把这条路线做成正式 D1b 产品线；正式 Android 控制端后续另开 OpenSpec change。

## 两阶段路线

### 1. Day 0：Termux/ADB 真实验证

先不做 APK，先证明手机能作为真实控制端打开远端 shell。

推荐用 ADB 或 Termux 把 Android binary 放到手机可执行目录，使用移动网络跑通：

```bash
adb push /tmp/miopunch-android-arm64 /data/local/tmp/miopunch
adb shell chmod +x /data/local/tmp/miopunch
adb shell /data/local/tmp/miopunch --help
```

建议固定一组路径，避免默认 home / XDG 解析在 Android 上不稳定：

```bash
LOCALAPI=unix:/data/local/tmp/miopunch-localapi.sock
STATE=/data/local/tmp/miopunch-state/state.json

/data/local/tmp/miopunch up \
  --localapi "$LOCALAPI" \
  --state_path "$STATE"
```

另开一个 shell 跑控制命令：

```bash
/data/local/tmp/miopunch --localapi "$LOCALAPI" join "<invite_code>"
/data/local/tmp/miopunch --localapi "$LOCALAPI" ls
/data/local/tmp/miopunch --localapi "$LOCALAPI" ping "<peer_id>"
/data/local/tmp/miopunch --localapi "$LOCALAPI" sh ls "<peer_id>" local
/data/local/tmp/miopunch --localapi "$LOCALAPI" sh "<peer_id>" local -s main
```

Day 0 成功标准是手机移动网络下进入远端 shell，并能运行 `date`、`whoami`、`uptime`、`ls` 这类无风险命令。

### 2. APK lite：极薄 Android 壳

APK 只做一个控制台壳，底层仍复用同一个 `miopunch` binary。

最小 UI：

- Invite code 输入框。
- Peer ID 输入框。
- Target 输入框，默认 `local`。
- Session 输入框，默认 `main`。
- `Start Runtime` 按钮。
- `Join` 按钮。
- `Ping` 按钮。
- `List Sessions` 按钮。
- `Open Shell` 按钮。
- `Stop` 按钮。
- 日志输出区域。
- 简易终端区域。

App 内部路径：

```text
localapi = unix:<cacheDir>/miopunch-localapi.sock
state    = <filesDir>/state/state.json
logs     = <filesDir>/logs/
```

App 启动 runtime：

```bash
miopunch up \
  --localapi unix:<cacheDir>/miopunch-localapi.sock \
  --state_path <filesDir>/state/state.json
```

App 执行动作：

```bash
miopunch --localapi unix:<cacheDir>/miopunch-localapi.sock join <invite_code>
miopunch --localapi unix:<cacheDir>/miopunch-localapi.sock ls
miopunch --localapi unix:<cacheDir>/miopunch-localapi.sock ping <peer_id>
miopunch --localapi unix:<cacheDir>/miopunch-localapi.sock sh ls <peer_id> <target>
miopunch --localapi unix:<cacheDir>/miopunch-localapi.sock sh <peer_id> <target> -s <session>
```

第一版 shell 不直接实现 LocalAPI client。直接启动 `miopunch sh` 子进程：

- Android 终端输入写入子进程 stdin。
- 子进程 stdout 写入终端区域。
- 子进程 stderr 写入日志区域。
- 子进程退出码非 0 时，原样展示 CLI 输出中的 `stage`、`reason_code`、`facts`、`suggestions`。

## Android 打包注意点

- 优先只支持 `arm64-v8a`，避免一开始做多 ABI。
- Android 官方 ABI 文档说明 `arm64-v8a` 是标准 64-bit ARM ABI，native code 通常按 `lib/<abi>/lib<name>.so` 放入 APK，并由系统提取到 `nativeLibraryDir`。
- 第一版可以把 `miopunch` 产物作为 native payload 打包，并通过 App 可执行路径启动；如果直接从 App 私有文件目录执行在目标 Android 版本上受限，优先改为 native library extraction 路线。
- Android 官方 manifest 文档中的 `extractNativeLibs` 会影响 native library 是否在安装时解包；APK lite 需要实机验证该设置与执行路径。
- 不通过网络动态下载 binary，避免引入额外安全和演示不确定性。

参考：

- <https://developer.android.com/ndk/guides/abis.html>
- <https://developer.android.com/guide/topics/manifest/application-element#extractNativeLibs>

## 面试演示建议

演示前准备：

- 家里或云上保留一个远端 host，提前运行 `miopunch up`。
- 远端 host 已加入同一个测试网络。
- 远端 target 固定为 `local` 或已验证的 Windows `wsl:<distro>` / `ssh:<name>`。
- 远端 session 固定为 `main`。
- 手机提前装好 APK lite，或保留 ADB/Termux 路线作为兜底。

现场顺序：

```text
Start Runtime
-> 粘贴 invite code 并 Join
-> 输入 peer_id
-> Ping
-> List Sessions
-> Open Shell
-> 在远端 shell 中运行 date / whoami / uptime / ls
```

对外说明口径：

- “手机不是被控端，只是控制端。”
- “被控端在家里/云上网络后面，没有开放公网 SSH 端口。”
- “先过一次 identity-bound ping / SecureSession gate，再打开 shell。”
- “如果失败，CLI 原样输出 stage、reason_code、facts、suggestions，我可以按阶段拆解是 signaling、打洞、session 还是 shell target 的问题。”

## 验收清单

- Android arm64 binary 能在目标手机执行 `--help`。
- Android 移动网络下 `join` 成功。
- Android 移动网络下 `ping <peer_id>` 成功。
- Android 移动网络下 `sh <peer_id> local -s main` 能进入远端 shell。
- APK lite 能启动和停止本地 runtime。
- APK lite 能显示 CLI stdout/stderr。
- APK lite 能把终端输入透传给 `miopunch sh` 子进程。
- App 退出或 Stop 后，本地 runtime 与 shell 子进程被清理。

## 主要风险

- Android 某些版本限制从 App 私有目录执行普通文件，需要改走 native library extraction。
- `miopunch sh` 当前以传统终端 stdin/stdout 为核心；APK 内终端需要验证 raw mode、控制字符和输入法行为。
- 手机网络下 STUN / broker DNS 与超时预算仍可能影响稳定性；必要时固定 broker 或使用已验证的网络。
- 现场演示不要临时生成全新复杂网络。优先使用预先验证过的 home peer、target、session。

## 2026-05-31 APK lite 实机操作复盘

本轮目标是把 Android 手机作为真实控制端，连接 Linux/WSL peer 并在 App 内看到远端 shell 输出。最终路径已经跑通：

```text
Docker broker
-> Linux/WSL miopunch up
-> Android control-lite Start Runtime
-> Ping Linux peer
-> Open Shell
-> whoami 返回 js
```

实测环境：

- Android：Pixel 6a，package `com.miopunch.controlite`。
- Linux/WSL peer binary：`/tmp/miopunch-control-lite-demo-final/miopunch`。
- Broker：Docker `eclipse-mosquitto:2`，host 侧监听 `127.0.0.1:18883`。
- Android 访问 broker：`adb reverse tcp:18883 tcp:18883`。
- 证据目录：`/tmp/miopunch-control-lite-demo-final/evidence/`。

最终证据：

```text
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-ping-ok.png
/tmp/miopunch-control-lite-demo-final/evidence/2026-05-31-android-wsl-shell-whoami-js.png
```

### 实机操作顺序

1. 启动本地 broker，并确认 Android 有 adb reverse：

```bash
docker ps --filter name=miopunch-control-lite-broker
adb reverse tcp:18883 tcp:18883
adb reverse --list
```

2. Linux/WSL 端启动本地 peer：

```bash
DEMO=/tmp/miopunch-control-lite-demo-final
M=$DEMO/miopunch

$M up \
  --localapi unix:$DEMO/linux/localapi.sock \
  --broker tcp://127.0.0.1:18883 \
  --state_path $DEMO/linux/state.json \
  --log-level trace
```

3. Android 端启动 App，并预填已知 peer id：

```bash
adb shell am start -W -S \
  -n com.miopunch.controlite/.MainActivity \
  --es peer DXG2AQAP4Z7EDO3ZEV3AOXAXNI
```

4. App 内按顺序操作：

```text
Start Runtime
-> Ping
-> Open Shell
-> 输入或注入 whoami
```

为了稳定复现 shell 输入，可以用 intent 走同一条 App shell-send 路径：

```bash
adb shell am start -W \
  -n com.miopunch.controlite/.MainActivity \
  --es line whoami
```

### 问题和解决

1. ADB 输入长 invite code 不稳定。

现象：通过 `adb shell input text` 输入长 invite 时，字符串会被输入法或 shell 转义破坏，导致 Join 失败。

解决：debug APK 支持 intent extra 预填字段：

```bash
adb shell am start -W \
  -n com.miopunch.controlite/.MainActivity \
  --es invite "$INVITE_CODE" \
  --es peer "$PEER_ID"
```

这只解决调试输入问题，不绕过 App 的 Join/Ping/Open Shell 行为。

2. 操作输出只在 ADB/logcat，App 界面没有反馈。

现象：点击 `Start Runtime`、`LS`、`Ping`、`Open Shell` 后，ADB 能看到进程输出，但 App 日志区不刷新，现场演示不可解释。

解决：把 runtime stderr、CLI stdout/stderr、exit code、`reason_code`、`facts`、`suggestions` 都写入 App 内 `Logs` 区。这样现场不用切到 logcat，也能看到失败阶段。

3. 顶部状态块误显示成杂乱日志。

现象：界面顶部出现类似 `runtime=running shell=stopped network=not joined` 的块，和已有 Logs 区重复，且容易误导。

解决：移除顶部状态摘要，只保留按钮状态和 Logs 区。必要状态直接从操作输出里看。

4. TextView 无法正确显示远端 shell。

现象：远端 shell 是 tmux/zsh/PTY 输出，包含 ANSI escape、光标移动和颜色控制。TextView 会显示一堆乱码，不能作为交互式 shell 展示。

解决：APK 内置 xterm.js 和 CSS，用 WebView 渲染 shell：

```text
assets/terminal/index.html
assets/terminal/vendor/xterm/xterm.js
assets/terminal/vendor/xterm/xterm.css
```

App 把 `miopunch sh` 子进程 stdout 写入 xterm，把 xterm 输入回写到子进程 stdin。

5. xterm 输出底部被裁掉。

现象：xterm 默认行数大于 WebView 可见区域，新的 `whoami` 输出可能落到不可见底部。

解决：`terminal/index.html` 按 WebView 实际高度动态调整 xterm rows，写入后 `scrollToBottom()`。

6. shell 输入换行不生效。

现象：发送命令后远端 tmux/PTY 没有按 Enter 执行。

解决：交互式 shell 输入必须发送 carriage return：

```text
\r
```

不能只发送 line feed：

```text
\n
```

7. Join/approve 有时错过。

现象：Android 端 Join 发出后，Linux 端后启动 approve 可能错过非 retained join request。

解决：Linux 端先启动 `approve <invite_code>` 等待，再点 Android `Join`。

8. Android 接口枚举有权限限制。

现象：Android App 沙箱内 runtime trace 出现：

```text
netlinkrib: permission denied
candidate=127.0.0.1 reason=no_usable_ipv4
```

影响：日志里会看到 loopback fallback，不应只凭这条判断演示失败。

处理：现场以最终 `Ping` facts 为准。成功时会显示：

```text
reason_code=OK
selected_path=direct_ipv4
```

9. APK 重装后首次 punch 可能 timeout。

现象：APK reinstall 后 Android peer online，但连续 Ping 超时，facts 显示 direct/punching deadline exceeded。

本次处理：重启 Linux/WSL peer 后恢复，随后 Ping 和 Open Shell 成功。

现场排障顺序：

```text
确认 broker running
-> 确认 adb reverse
-> 确认 Linux peer 进程和 localapi.sock
-> Linux ls 看 Android peer online
-> Android retry Ping
-> 如果仍 timeout，重启 Linux peer 后重试
```

### 下次演示前检查

1. 检查 broker：

```bash
docker ps --filter name=miopunch-control-lite-broker
```

2. 检查 adb reverse：

```bash
adb reverse --list
```

必须看到：

```text
tcp:18883 tcp:18883
```

3. 检查 Linux peer：

```bash
ps -p "$(cat /tmp/miopunch-control-lite-demo-final/linux/miopunch.live.pid)" -o pid,stat,cmd
/tmp/miopunch-control-lite-demo-final/miopunch \
  --localapi unix:/tmp/miopunch-control-lite-demo-final/linux/localapi.sock \
  --format json --redact ls
```

4. 检查 APK：

```bash
android/control-lite/scripts/build-debug-apk.sh
android/control-lite/scripts/install-debug-apk.sh
```

5. 检查最终 App 内证据：

```text
Ping: reason_code=OK, selected_path=direct_ipv4
Shell: whoami -> js
```

### 复盘结论

- Android 不是只能通过 CLI/ADB 展示；极薄 APK 壳已经能支撑手机端现场演示。
- 当前 APK 仍复用同一个 `miopunch` binary/runtime，没有引入 HTTP bridge。
- 演示时应先用 `Ping` 证明路径与 session 可用，再 `Open Shell` 展示远端 shell。
- 当前证据是同 LAN/WSL 场景；跨 NAT 或移动网络展示需要另行固定 broker、host 和超时参数。
- 这条路线的优先级应继续保持“可运行、可解释、可复现”，不要在演示前扩成完整 Android 产品。
