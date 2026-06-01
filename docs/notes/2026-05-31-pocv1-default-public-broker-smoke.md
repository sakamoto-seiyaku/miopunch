# POC v1 默认公网 Broker Smoke 记录

日期：2026-05-31

## 范围

验证 POC v1 在未显式配置 broker 时，是否默认使用
`tcp://broker.emqx.io:1883`。本次验证只使用公网 EMQX broker，没有启动本地
MQTT broker。

## 构建和运行环境

- 从当前 worktree 重新构建 session bundles：
  `scripts/release/build_bundles.sh`
- Linux bundle：
  `/tmp/miopunch-wsl2-windows-cli-smoke/current/wsl/miopunch_0.0.0-git55bd5c4_linux_amd64_session`
- Windows bundle：
  `C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-cli-smoke\current\windows\miopunch_0.0.0-git55bd5c4_windows_amd64_session`
- WSL daemon 启动时未传 `--broker`。
- Windows daemon 启动时未传 `--broker`。

## 通过路径

默认 broker 已出现在 bootstrap 和 invite/join/approve 输出中：

- `init-network`：`broker_endpoint=tcp://broker.emqx.io:1883`
- `invite`：`broker_endpoint=tcp://broker.emqx.io:1883`
- `join`：`broker_endpoint=tcp://broker.emqx.io:1883`
- `approve`：`broker_endpoint=tcp://broker.emqx.io:1883`

干净 `sh ls` 验证中的网络和 peer：

- `network_id=G5EZTOGPD4GE5MUX6YRSAKAT2U`
- WSL/admin peer：`XKZBB3WL4V4QM4AUK33Z2GAUSQ`
- Windows/member peer：`RF4QX4K6OODUDBFZA63KXCJEJY`

WSL 到 Windows 的 shell target discovery 通过：

- `sh ls RF4QX4K6OODUDBFZA63KXCJEJY`：`status=done`
- `selected_path=direct_ipv4`
- `target_count=29`
- 目标列表包含 `wsl:Debian` 和 `wsl:Ubuntu`。

Ready probe 也通过：

- `sh ls RF4QX4K6OODUDBFZA63KXCJEJY --ready`：`status=done`
- `selected_path=direct_ipv4`
- `ready_target_count=3`
- ready targets：`ssh:ale`、`wsl:Debian`、`wsl:Ubuntu`
- `unsupported_target_count=0`
- `unknown_target_count=26`

`unknown` 的 SSH 条目来自 Windows 用户 SSH 配置里的主机可达性、密钥权限、
host key 等状态，不是 miopunch broker 或 secure session 的失败。

## 观察到的问题

另一次默认 broker 验证中，WSL 到 Windows 的 ping 先成功；随后执行 Windows
到 WSL ping，并发/反向 shell probe 后，后续操作复用了一个 UDP socket 已经
关闭的 direct path session。

失败事实：

- `stage=SecureSession`
- `reason_code=UNAVAILABLE`
- `selected_path=direct_ipv4`
- `error=tls handshake: write udp 0.0.0.0:48245: raw-write udp4 0.0.0.0:48245: use of closed network connection`
- Windows 反向重试也使用了对应端口并失败：
  `write udp4 0.0.0.0:56673->192.168.4.5:48245: use of closed network connection`

Artifacts：

- 通过的默认 broker shell run：
  `/tmp/miopunch-wsl2-windows-cli-smoke/current/artifacts-default-sh/`
- 较早失败的默认 broker run：
  `/tmp/miopunch-wsl2-windows-cli-smoke/current/artifacts-default/`

## 后续

需要继续查 direct path session 生命周期：当 secure-session handshake 失败或
底层 UDP socket 已关闭后，后续 `ping` / `sh ls` 是否仍然选择了 stale peer
session，而不是强制重新 punching 并建立新的 session。

## 根因更新

这里不是“UDP direct 设计本身错误”，而是设计约束没有写严，实现时把资源所有权
放松了。

原始意图是每个 runtime 绑定一个长期 UDP 端口，用这个端口做候选地址、打洞和
secure session。这个端口应该由 runtime 统一关闭。但实现里 `PathResult.Conn`
直接携带了 runtime 的 `*net.UDPConn`，`PathResult.Close()` 又会关闭它；
session transport 的 cleanup 也把同一个 `result.Conn` 放进 closers。这样一次
TLS/KCP handshake 失败、peer session 关闭、或反向/并发操作触发的 fatal cleanup，
都会把 runtime 还打算继续复用的 UDP fd 关掉。

后续 Runtime 只检查 `r.udpConn != nil`，看不到 fd 已经关闭，于是继续广告同一个
端口并把同一个 Go 指针交给下一次 punch/session。写包时就会出现
`use of closed network connection`。

修复方向：

- Runtime 是 daemon UDP socket 的唯一 owner。
- `PathResult` 只是 selected path 描述，不拥有 UDP socket；`Close()` 保留但不
  关闭 socket。
- secure session 可以关闭 TLS/KCP/yamux 自己创建的资源，但不能关闭 runtime
  借给它的 UDP socket。
- 对已确认 transport fatal 的 peer session，Runtime 从 manager 中移除；下一次
  action 可以重新 punch/upgrade，而不是复用坏 session。

## 修复后双向 ping 复测

日期：2026-05-31 16:03-16:13 Asia/Taipei。

重新构建并解压：

- Linux bundle：
  `/tmp/miopunch-wsl2-windows-cli-smoke/current/wsl/miopunch_0.0.0-git55bd5c4_linux_amd64_session`
- Windows bundle：
  `C:\Users\stati\AppData\Local\Temp\miopunch-wsl2-windows-cli-smoke\current\windows\miopunch_0.0.0-git55bd5c4_windows_amd64_session\miopunch_0.0.0-git55bd5c4_windows_amd64_session`

注意：WSL session bundle 的默认 `data/localapi.sock` 路径超过 Unix socket
路径长度限制，本轮 WSL daemon 使用短 LocalAPI：
`unix:/tmp/mp-poc1-wsl/localapi.sock`；state path 使用
`/tmp/miopunch-wsl2-windows-cli-smoke/current/run/wsl/state.json`。

本轮仍未启动本地 MQTT broker。`init-network`、`join`、`approve` 输出均确认：

- `broker_endpoint=tcp://broker.emqx.io:1883`

本轮网络：

- `network_id=YJY2TM552ZMKQGDX2EI34IRENU`
- WSL/admin peer：`YM57RYXIDOCVH3CTDPEYIDFJWA`
- Windows/member peer：`ZBCXWADWV3CPNQUN65AUHTJUSA`

顺序双向 ping 结果：

- WSL -> Windows：
  `status=done stage=SecureSession reason_code=OK ping=ok selected_path=direct_ipv4`
- Windows -> WSL：
  `status=done stage=Shell reason_code=OK ping=ok selected_path=direct_ipv4`

反向后重复 ping 也通过，未再看到
`use of closed network connection`。

额外并发双向 ping 观察：

- Windows -> WSL 通过：
  `status=done stage=Punch reason_code=OK ping=ok selected_path=direct_ipv4`
- WSL -> Windows 出现一次 TLS handshake read timeout：
  `stage=SecureSession reason_code=UNAVAILABLE selected_path=direct_ipv4`

这个并发现象不是旧的已关闭 UDP fd 复用问题；本轮日志/grep 没有发现
`use of closed network connection`。它更像双端同时发起 secure-session upgrade
时的会话建立竞争，后续顺序重试立即恢复。
