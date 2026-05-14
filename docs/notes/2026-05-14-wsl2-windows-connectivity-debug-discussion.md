# WSL2 / Windows 连接问题排查记录

日期：2026-05-14

状态：临时讨论与排查文档。本文记录现象、实验事实、日志证据、代码阅读结论和当前推理；除非明确标注为“已确认”，否则不要当成最终根因。

## 1. 背景与目标

- 环境：Windows 主机 + WSL2 Linux。
- WSL2 网络模式：mirrored mode。
- 预期：Windows 与 WSL2 共享较接近的网络栈，本机跨系统互通应比普通 NAT 穿透更容易。
- 当前现状：miopunch 能建立部分 UDP/TCP 路径，但整体表现不稳定，仍接近 POC 状态。
- 当前排查目标：
  - 把 TCP / UDP / keepalive / topology / governance 这些表面混在一起的问题拆开。
  - 每个问题都先记录“证据链”，再判断是否需要改实现。
  - 优先建立可重复、闭环、CLI-first 的调试方式，避免桌面 UI 掩盖真实状态。

## 2. 已使用的实验环境与产物

- WSL2 侧控制实验流程。
- Windows 侧使用已有桌面/daemon 进程，并通过 Windows CLI 驱动。
- 临时 Linux daemon 使用隔离状态目录：
  - `/tmp/miopunch-debug`
- 主要日志：
  - `/tmp/miopunch-debug/artifacts/linux-daemons-miopunch.log`
  - `/tmp/miopunch-debug/artifacts/windows-miopunch.log`
- Windows 报告目录：
  - `C:\Users\stati\AppData\Local\Temp\miopunch-debug\windows\reports`
- Windows peer：
  - `BD5Q6OKGEXFMCVLWEF4HK2LVXA`
- 实验中出现过的 Linux peers：
  - `MPL3T6I7WRD65U644T5UHQS3II`
  - `RAGUV3DWDRIY4V4BXZXCKAF4N4`
  - `IWVMRM5K55H5BQ4QESXYXYJLXI`

## 2.1 现场实验技巧与易踩坑

这些不是根因结论，而是为了避免后续实验反复卡在同一批外围问题。

### MQTT broker / invite code

已观察到的事实：

- Windows 侧 `invite` 报告里出现过多个 broker timeout：
  - `mqtt.eclipseprojects.io:1883`
  - `broker.hivemq.com:1883`
  - 若干直接 IP broker
- 最终有效事实通常是：
  - `brokers_effective=broker.emqx.io:1883`
  - `invite_brokers=44.232.241.40:1883`
- Linux `join` 早期失败过：
  - `mqtt connect failed: 44.232.241.40:1883: future canceled`
  - suggestion: `set local.mqtt_broker to a reachable broker shared by both machines`
- 成功 join 的报告仍会看到 `invite_brokers=44.232.241.40:1883`，但实际状态里的 `local.mqtt_broker` 是 `broker.emqx.io:1883`。

实验规则：

- 新建隔离 state 时，先确认 Windows 与 Linux 双方 `local.mqtt_broker` 一致，并且优先使用当前可用的 `broker.emqx.io:1883`。
- 如果 `join` 卡在 `connect invite brokers` 或 `wait membership bundle`，先检查 broker reachability 和双方 broker 配置，不要马上归因到 UDP/TCP 打洞。
- `invite_brokers=44.232.241.40:1883` 可以理解为 invite code 中记录的 broker 地址；实验记录里需要同时写下 `brokers_effective`，否则后面会误判。
- 如果报告里有 `mqtt hello barrier timeout (10s)`，这是 signaling 阶段失败，不是 NAT punching 或 dataplane 失败。

### Windows 旧进程残留

已观察到的事实：

- Windows 侧可能残留旧的 `miopunch.exe up` / 桌面进程。
- 旧 daemon 可能持有默认 LocalAPI、旧 state、旧 peer 身份或旧 session，导致后续 CLI 实验连到错误进程。
- 用户已通过 Windows 任务管理器清理过一次旧进程。
- 2026-05-14 22:13 从 WSL2 用 TTY 方式启动 Windows `miopunch.exe up` 时，进程存在，但 CLI 连接默认 npipe 报 `DAEMON_NOT_RUNNING`；该启动方式输出了终端控制序列，疑似进入了 Windows 控制台/TTY 特殊路径，不能作为可靠 daemon 启动方式。
- 同一命令改成非 TTY 前台方式后，立即输出：
  - `miopunch up: serving LocalAPI (user) at npipe:\\.\pipe\miopunch\localapi-...`
  - 随后 Windows CLI `ls` 成功。

实验规则：

- 每轮 fresh 实验前先确认 Windows 侧是否还有旧进程；不确定时，直接请用户在 Windows 任务管理器里清理。
- 如果需要从 WSL2 停 Windows 进程，优先使用明确 PID：
  - `powershell.exe -NoProfile -Command 'Stop-Process -Id <PID> -Force'`
- 不要在“可能有旧进程”的状态下解释 TCP/UDP 结果；先把进程归属确认干净。
- 从 WSL2 拉起 Windows daemon 时，优先用非 TTY 前台进程；不要把“Windows 进程存在”直接等同于 “LocalAPI 已经可用”，必须再跑一次 Windows CLI `ls` 验证。

### fresh 实验与 session reuse

已观察到的事实：

- `ping -t` / `ping -u` 当前可能复用已有 session。
- session reuse key 没有按请求的 `P2PNetwork` / path family 过滤。
- 因此 `ping -t` 可能复用先前的 `udp4` session，并显示 `session_reused=true`。

实验规则：

- 需要验证真实 TCP/UDP punching 时，必须重启双方 daemon 或显式清空 session，避免 reuse。
- 实验报告中只要出现 `session_reused=true`，该次结果不能用来证明当前 TCP/UDP 打洞能力。
- clean protocol 实验必须分开跑：
  - TCP：`ping -t`
  - UDP：`ping -u`
  - auto 只用于产品默认行为观察，不用于证明某个协议是否可打通。

### Windows LocalAPI override

已观察到的事实：

- Windows `--localapi npipe:...` 启隔离 daemon 目前会受限。
- 服务端 listen 侧需要 `OperatorSID`，但 `ParseAddr("npipe:...")` 不会补上。
- 客户端 dial 自定义 npipe 不需要 `OperatorSID`，所以失败点主要在服务端 listen override。

实验规则：

- 短期不要把 Windows 自定义 npipe 当作可靠隔离手段。
- 若要完整隔离 Windows 侧，优先使用单独 state，但 LocalAPI 仍可能只能走默认 pipe；因此必须先确认实际连接到哪个 daemon。
- 中期需要修复 Windows `--localapi npipe:...` 服务端地址初始化，否则闭环实验会持续受限。

### Linux daemon 存活方式

已观察到的事实：

- 在 WSL2 里用 `nohup` 启 Linux daemon 曾出现进程很快退出或被 shell/session 清理的情况。
- 长时间实验如果 daemon 中途退出，会伪装成 punching、keepalive 或 topology 问题。

实验规则：

- 需要观察实时日志时，优先用前台长进程 session 启 Linux daemon。
- 每次实验前用 `ls` / `ps` / LocalAPI socket 检查 daemon 仍然活着。
- 实验结束时再主动停止前台 daemon，避免新旧进程叠加。

### 报告与日志采集

实验规则：

- 每个 CLI 命令都尽量加 `--format json --report <path>`，不要只看终端摘要。
- 从 WSL2 调 Windows `.exe` 时，Windows 路径里的反斜杠必须用单引号保护，例如：
  - `--report 'C:\Users\stati\AppData\Local\Temp\...\report.md'`
- 如果不保护，路径可能变成 `C:.Users...`，甚至出现“report 导出失败但 ping 已经执行”的污染样本。
- 每次实验至少记录：
  - task id
  - peer id / net id
  - `attempt_path`
  - `path_family`
  - `data_proto`
  - `hello`
  - `ping`
  - 是否出现 `session_reused=true`
- 如果失败，优先按阶段分类：
  - MQTT signaling
  - candidate gather
  - punching attempt
  - dataplane handshake
  - hello/governance
  - logical ping payload
  - idle/session lifecycle

## 3. 当前主要结论摘要

- TCP 不是单纯“打不通”。至少 Windows -> Linux fresh TCP 曾成功到达 `punching_tcp4` / `tcp4` / `tls`，且 `ping=ok`。
- `ping -t` 不一定真的测试 TCP。当前实现会先复用已有 session，且复用逻辑不按用户指定的 UDP/TCP 网络策略过滤，所以 `ping -t` 可能直接复用 `udp4`。
- UDP 的“非对称”更像观测/状态记录问题，而不是底层路径完全没通。发起侧记录 task/topology 成功，被动 acceptor 侧没有把同等 runtime evidence 写回 task manager topology。
- 2026-05-14 20:13 fresh Linux -> Windows UDP 已确认成功：`punching_ipv4` / `udp4` / `quic` / `hello=ok` / `ping=ok`，且没有 `session_reused=true`。
- 2026-05-14 20:22 fresh Windows -> Linux TCP 失败已稳定复现两次：TCP punching 有 `tcp_conns=1`，但后续 TLS winner election 失败：`follower did not receive winner signal`。
- 2026-05-14 21:07 已复现 `ping -t` 被 UDP session reuse 污染：先 fresh `ping -u` 成功，再 `ping -t`，第二次报告为 `data_proto=quic`、`session_reused=true`、`path_family=udp4`。
- 2026-05-14 21:11 已复现 `ping -u` 被 TCP session reuse 污染：先 fresh `ping -t` 成功，再 `ping -u`，第二次报告为 `data_proto=tls`、`session_reused=true`、`path_family=tcp4`。
- 2026-05-14 21:07 已复现被动侧 topology evidence 缺失：Linux 主动 UDP 成功后，Windows acceptor 日志有 `incoming attempt` / `connectivity attempt delegated`，但 Windows topology 仍为 `active=[]`、`attempts=[]`、`payloads=[]`。
- 2026-05-14 21:09 fresh Windows -> Linux TCP 进一步确认：TCP punching 层很快返回 `tcp_conns=1`，但 TLS converge 仍等待慢失败 candidate，直到慢 candidate 失败后才进入 election。
- Linux -> Windows 的 UDP/TCP 失败，实验里都已经到达对应路径，之后失败在 hello/governance：`approve_decl issuer is not an admin`。
- keepalive 需要拆成两层看：
  - QUIC 底层配置有 `KeepAlivePeriod=10s`、`MaxIdleTimeout=30s`。
  - miopunch 自己还有 `DefaultSessionIdleTimeout=2m`，它只看逻辑 stream activity，不看 QUIC 底层 keepalive。因此底层 keepalive 不能阻止 miopunch session idle close。
- `invite --mode auto` 当前看起来只被编码进 invite code，没有真正完成 auto-approve 流程。
- Windows 自定义 `--localapi npipe:...` 启第二个隔离 daemon 目前看起来有实现障碍：Windows pipe listener 要求 `OperatorSID`，但 `ParseAddr("npipe:...")` 没有带上。
- 当前排查优先级调整：先把 UDP/TCP punching 与连接保持彻底拆清楚，治理问题先作为干扰项记录，不作为主线。

## 3.1 打洞链路专项排查框架

这一轮只把链路分层，不先讨论 UI 或治理：

```text
state/config
  -> gather 本地 UDP/TCP socket 与候选地址
  -> MQTT signaling 交换双方候选
  -> punchdecision 决定 effective p2p_network 与 UDP/TCP 行为
  -> connectivity.Attempt 按策略尝试 direct / punching
  -> dataplane session 建立：UDP=QUIC/KCP, TCP=TLS+yamux
  -> logical stream：hello / ping / shell
  -> session lifecycle：reuse / activity / idle close / keepalive
```

当前代码事实：

- `connectivity.Gather` 根据 `P2PNetwork` 决定是否同时开 UDP socket 和 TCP listener。
- TCP listen port 不是独立随机语义：优先取 `base + 100`。如果 UDP/base port 是 `P`，TCP listener 通常是 `P+100`。
- `punchdecision.Analyze...` 会合并双方 `P2PNetwork`：
  - `udp_only` 禁用 TCP punching。
  - `tcp_only` 要求双方有 TCP capability。
  - `auto` 会保留 TCP 与 UDP 两类候选。
- `connectivity.Attempt` 的 auto 尝试顺序是：
  - `direct_tcp6`
  - `direct_tcp4`
  - `punching_tcp4`
  - `direct_ipv6`
  - `direct_ipv4`
  - `punching_ipv4`
- 因此 auto 模式下，只要 TCP 条件满足，代码会先试 TCP，而不是先试 UDP。
- 干净实验必须用 `-u` / `udp_only` 与 `-t` / `tcp_only` 分开跑，并避免 session reuse。

待验证的关键假设：

- Windows/WSL2 mirrored 下，TCP `base+100`、STUN mapped TCP ports、以及 candidate `+100 offset` 是否符合真实网络行为。
- UDP punching 成功后，底层 QUIC session 是否双方都存活，只是被动侧没有进入桌面 topology。
- 应用层 idle close 是否是“等一会儿断开”的主因，而不是 NAT 映射失效。
- `ping -t/-u` 的复用是否会污染所有后续实验结果。

## 4. 现象与证据

### 4.1 TCP 状态：不是纯网络层失败

用户最初观察：TCP 一直连不上。

实验事实：

- Windows -> Linux fresh TCP 成功：
  - `attempt_path=punching_tcp4`
  - `path_family=tcp4`
  - `data_proto=tls`
  - `hello=ok`
  - `ping=ok`
- Linux -> Windows fresh TCP 到达 TCP punching path：
  - `attempt_path=punching_tcp4`
  - `path_family=tcp4`
  - `data_proto=tls`
  - 后续失败：`approve_decl issuer is not an admin`

日志证据：

```text
windows-miopunch.log:
task fact ... message=attempt_path=punching_tcp4
task fact ... message=path_family=tcp4

linux-daemons-miopunch.log:
task fact ... message=attempt_path=punching_tcp4
task fact ... message=path_family=tcp4
task fact ... message=approve_decl issuer is not an admin
```

当前判断：

- TCP punching 至少在 Windows/WSL2 mirrored 环境里有成功样本。
- Linux -> Windows 失败不能直接归因于 TCP punching；它更像“路径建立之后的授权/成员治理失败”。
- 后续如果要定位 TCP 真实失败，应强制 fresh TCP attempt，避免被 session reuse 污染。

### 4.2 `ping -t` 会复用 UDP session，导致测试语义失真

实验事实：

- 在已有 UDP session 后执行 `ping -t`，报告出现：
  - `session_reused=true`
  - `path_family=udp4`

日志证据：

```text
windows-miopunch.log:
task fact ... term_id=session_reused message=session_reused=true
task fact ... term_id=path_family message=path_family=udp4

linux-daemons-miopunch.log:
task fact ... term_id=session_reused message=session_reused=true
task fact ... term_id=path_family message=path_family=udp4
```

代码事实：

- 入口：`internal/task/poc_dial.go`
- `dialPeerStream` 在 gather / punching 前先查 `m.sessions.Find(reuseKey)`。
- reuse key 包含：
  - `RemotePeerID`
  - `Protocol`
  - `SecurityID`
  - 不包含指定的 `P2PNetwork` 约束。
- 找不到 `cfg.DataProto` 后还会 fallback 到 `ProtocolTLS`。
- `SessionManager.Find` 只有在 `PathFamily` 非空且非 `unknown` 时才按 path family 过滤；当前 reuse key 没填 path family。

当前判断：

- `ping -t` 当前语义更接近“优先复用任何可用 session，否则按 tcp_only 发起新连接”。
- 这对普通用户可能合理，但对排查 TCP 是严重干扰。
- 后续需要考虑：
  - `ping -t` / `ping -u` 的 session reuse 尊重 P2PNetwork。
  - 或增加调试参数，例如 `--fresh` / `--no-reuse`。

### 4.3 UDP 非对称：被动侧缺少 topology/runtime evidence

用户现象：

- Windows ping Linux，Windows 显示 OK。
- Linux 侧没有明显状态变化。
- Linux 再 ping Windows 后，Linux 才像是“重新打洞成功”。

实验事实：

- Windows -> Linux `ping -u` 成功：
  - `attempt_path=punching_ipv4`
  - `path_family=udp4`
  - `data_proto=quic`
  - `hello=ok`
  - `ping=ok`
- 之后 Linux topology 曾显示：
  - `active=[]`
  - `attempts=[]`
  - `payloads=[]`

被动侧日志证据：

```text
linux-daemons-miopunch.log:
pocacceptor connectivity attempt ready ... path=punching_ipv4 tcp_conns=0 protocol=quic
pocacceptor connectivity attempt delegated ... path=punching_ipv4
```

代码事实：

- topology runtime 写入点：
  - `internal/task/topology_runtime.go`
  - `recordTopologyAttempt`
  - `recordTopologyPayload`
- 调用点主要在 task 发起侧：
  - `internal/task/poc_dial.go`
  - `internal/task/ping.go`
  - 以及其他主动 task。
- 被动 acceptor 路径：
  - `internal/pocacceptor/acceptor.go`
  - 能 log incoming attempt / accepted stream。
  - 但不接入 `task.Manager` 的 topology runtime，因此不会记录同等 attempts/payloads。

当前判断：

- “UDP 已打通但 Linux 无变化”目前更像 runtime/topology 观测缺口。
- 被动侧确实建立或接收了 session/stream，但它的证据留在 acceptor log，不进入桌面 topology 所看的 task runtime。
- 这解释了为什么双方看起来需要各自发起一次：不是网络一定要打两次，而是每边只有主动发起时才写自己的 task/topology 证据。

### 4.4 `approve_decl issuer is not an admin` 是独立主线

实验事实：

- Linux -> Windows UDP 到达 `punching_ipv4` / `udp4` 后失败。
- Linux -> Windows TCP 到达 `punching_tcp4` / `tcp4` 后失败。
- Windows acceptor 日志明确显示收到 stream 后，在 hello 阶段拒绝：

```text
windows-miopunch.log:
pocacceptor accepted stream failed: proto=quic ... path_family=udp4 ... err=approve_decl issuer not admin
pocacceptor accepted stream failed: proto=tls ... path_family=tcp4 ... err=approve_decl issuer not admin
```

代码事实：

- 接收端校验入口：`internal/pocacceptor/acceptor.go` 的 `handleHello`。
- 如果 hello metadata 带了 `approve_decl`：
  - 解析 decl。
  - 要求 `decl.Kind == approve_member`。
  - 调用 `head.AdminEd25519Pub(decl.IssuerPeerID)`。
  - 如果 issuer 不是 admin，返回 `HELLO_ISSUER_NOT_ADMIN`。
- 发起端 hello 组装：
  - `internal/task/hello.go`
  - `findSelfApproveDeclJSON` 会找本地“批准自己”的 approve decl，并放入 hello metadata。
- approve/invite 生成路径：
  - `internal/task/invite.go` 会用当前 self identity 生成 invite code，并确保 governance head 存在。
  - `internal/task/approve.go` 会检查“当前 self 是否等于 invite issuer”，但没有看到它检查“当前 self 是否是 head 里的 admin/owner”。
  - `buildMembershipBundleCiphertext` 直接调用 `pocstate.NewApproveMemberDeclV0(..., selfID, ...)` 生成 approve decl。
  - `NewApproveMemberDeclV0` 只负责 canonicalize 和签名，不知道治理 head，因此也不会拒绝非 admin issuer。
- 已检查状态文件：
  - Windows/Linux 同一个 `net_id` 和 head hash。
  - 但 governance head 的 owner/admin 是 `J2AJQ7QS73FQOF5AS5OYBV35LQ`。
  - 当前 Windows peer 是 `BD5Q6OKGEXFMCVLWEF4HK2LVXA`，不是该网络的 owner/admin。

当前判断：

- 这不是 UDP/TCP 打洞问题，而是成员治理/状态污染问题。
- 当前 Windows peer 似乎能执行 `invite` / `approve`，并产生由非 admin 签发的 approve decl。
- 接收端按治理规则拒绝这个 decl 是合理的。
- 已确认的实现缺口：
  - `approve` 在发布 membership bundle 前缺少 admin/owner 前置校验。
  - `invite` 也缺少“当前 peer 是否有资格签发后续 approval”的显式校验。
  - CLI/桌面没有提前暴露“你当前身份不是 admin，生成的 invite/approval 不能被对端接受”。
- 也可能混有历史状态污染：旧 admin `J2AJ...` 与当前 Windows peer `BD5Q...` 不一致。

### 4.5 keepalive / idle timeout：当前不是长期保活

用户现象：

- 桌面端打洞成功后，等一段时间就断。
- 没看到长期 keepalive 包启动或维持状态。

代码事实：

- `dataplane/session.go`
  - `DefaultSessionIdleTimeout = 2 * time.Minute`
  - `lastActivity` 初始为 session 创建时间。
  - 只有 `logicalStream.Read/Write/Close` 和 `OpenStream/AcceptStream` 会 `markActivity()`。
- `dataplane/session_transport.go`
  - yamux / QUIC session 都启动 `startIdleCloser`。
  - 如果 `idleFor() >= timeout`，关闭 session，reason 是 `idle_timeout`。
- QUIC 底层配置：
  - `MaxIdleTimeout: 30 * time.Second`
  - `KeepAlivePeriod: 10 * time.Second`
- dataplane 测试也明确表达了语义：
  - 逻辑 stream activity 能保持 session healthy。
  - 真正 inactive 的 session 会被 idle timeout 关闭。

日志证据：

```text
linux-daemons-miopunch.log:
pocacceptor accept stream failed: proto=quic ... path_family=udp4 err=Application error 0x0 (local): idle_timeout

windows-miopunch.log:
pocacceptor accept stream failed: proto=quic ... path_family=udp4 err=Application error 0x0 (remote): idle_timeout
```

当前判断：

- QUIC 底层 keepalive 只能帮助 QUIC 连接/NAT 映射，不会更新 miopunch `lastActivity`。
- miopunch 应用层 session 当前会在无逻辑 stream 读写约 2 分钟后主动关闭。
- 因此“打洞后保持长期连接”目前不是已实现的行为。
- `maintain-neighbors` 不是后台持续 keepalive loop；它只是一次 task，检查当前 selected neighbors 并尝试 ping。

### 4.6 `maintain-neighbors` 与 topology 可能出现状态分裂

实验事实：

- Windows `maintain-neighbors -u`：
  - `maintain_neighbors_selected=2`
  - `maintain_neighbors_succeeded=1`
  - `maintain_neighbors_failed=1`
  - `active_neighbors=2`
- Linux `maintain-neighbors -u`：
  - Windows peer 的 active ping 失败：`FORBIDDEN`
  - final facts 仍有 `active_neighbors=1`

日志证据：

```text
linux-daemons-miopunch.log:
neighbor_failed=BD5Q6OKGEXFMCVLWEF4HK2LVXA:FORBIDDEN
maintain_neighbors_succeeded=0
maintain_neighbors_failed=2
active_neighbors=1
```

代码事实：

- `internal/task/maintain_neighbors.go`
  - 先从 `TopologySnapshot()` 读取 active neighbors。
  - 对 selected neighbors 逐个启动 child `ping` task。
  - 最后再读取一次 `TopologySnapshot()`，把 active 数量写成 fact。
- 这意味着：
  - child ping 的成功/失败是一条证据。
  - session registry/topology snapshot 的 active 状态是另一条证据。
  - 两者可以短时间内不一致。
- `internal/task/poc_dial.go`
  - fresh dial 建立 dataplane session 后，先执行 `m.sessions.Put(sess)`。
  - 随后才 `sess.OpenStream(...)` 并进入 hello/ping 流程。
- `internal/task/ping.go`
  - 如果 hello 返回 `FORBIDDEN`，`runPingTask` 会结束 task。
  - 但 defer 只关闭 logical stream：`defer res.stream.Close()`。
  - 它不会关闭刚刚放进 `Manager.sessions` 的 peer session。
- `internal/task/topology.go` 和 `internal/task/desktop_state.go`
  - active peer sessions 都来自 `m.sessions.ListAllSummaries()`。
  - 因此“hello 已失败但 session 仍 healthy”会在 topology/desktop 里短时间显示 active。

当前判断：

- `maintain-neighbors` 不是严格的“健康状态裁判”。
- 它目前会把 task result 和 runtime session state 并列暴露，但没有强制一致。
- 这会加重桌面 UI 上“看起来连着/断着都可能”的困惑。
- Linux 侧出现 `neighbor_failed=...:FORBIDDEN` 但 `active_neighbors=1`，可以由上述顺序解释：transport session 已进入 manager，hello 失败只关闭 stream，session 等 idle closer 后才消失。

### 4.6.1 被动 acceptor 与桌面 session manager 不是同一套状态

代码事实：

- `internal/pocacceptor/acceptor.go` 自己维护 `peerSessionRegistry`。
- `servePeerSession` 对 accepted inbound session 调用 `reg.Replace(...)` / `reg.Remove(...)`。
- 这套 registry 用于 revoke、supersede 和 acceptor 生命周期管理。
- 但 desktop/topology 读取的是 `task.Manager.sessions`，不是 `pocacceptor.peerSessionRegistry`。

当前判断：

- 被动侧即使真实 accept 了对端 session，也不会自然出现在 desktop/topology 的 active peer sessions 中。
- 这进一步解释了 Windows 主动连 Linux 后，Linux UI/topology 没有同等 active evidence 的现象。
- 如果产品希望桌面显示“被动连入”，需要把 acceptor session evidence 显式接入 task/desktop topology，或设计统一 session registry。

### 4.7 `invite --mode auto` 尚未形成完整 auto approve

实验事实：

- `invite --mode auto` 后，Linux `join` 等待在 `wait membership bundle`。
- Windows 手动执行 `approve <invite_code>` 后，Linux join 才完成。

代码事实：

- `internal/controlplane/invite_code_v0.go` 定义了：
  - `InviteModeApprove`
  - `InviteModeAuto`
- `internal/task/invite.go` 会把 mode 写入 invite code。
- `internal/task/join.go` 没有使用 mode 自动拿到 membership bundle。
- `internal/task/approve.go` 仍是等待 join request 并发布 bundle 的任务。

当前判断：

- auto mode 当前更像 wire/config 字段，而不是完整产品能力。
- 后续需要二选一：
  - 完成真正 auto-approve。
  - 或先隐藏/禁用/改名，避免误导调试。

### 4.8 Windows 侧隔离 daemon 调试受限

目标：

- 希望从 WSL2 同时控制一个 Windows CLI/daemon 和一个 Linux daemon，形成完整闭环。
- 理想情况：启动独立 Windows daemon，使用单独 state/localapi，避免污染当前桌面进程。

代码事实：

- `cmd/miopunch/up_windows.go` 支持 `--localapi`。
- `serveUpWindows` 对 override 的处理是：
  - `overrideAddr, err = localapi.ParseAddr(override)`
  - 如果有 override，则最终 `addr = overrideAddr`
  - 随后调用 `localapi.Listen(addr, mode)`
- `internal/localapi/addr.go` 的 `ParseAddr("npipe:...")` 返回的 addr 不带 `OperatorSID`。
- `internal/localapi/addr_windows.go` 的默认地址 `DefaultSystemAddr` / `DefaultUserAddr` 会带 `OperatorSID`。
- `internal/localapi/listen_windows.go` 的 Windows pipe listener 要求 non-empty `OperatorSID`，否则返回 `empty operator_sid`。
- `internal/localapi/client_dial_windows.go` 只需要 pipe path，因此客户端连接 override 不依赖 `OperatorSID`。

当前判断：

- 自定义 named pipe 启第二个 Windows daemon 目前大概率不可用；阻塞点在服务端 listen override，而不是客户端 dial。
- 这会限制自动化实验，只能先驱动已有 Windows daemon 或改 localapi 初始化逻辑。

### 4.9 fresh Linux -> Windows UDP 成功，但被动侧 topology 没有记录 active

实验时间：2026-05-14 20:12-20:14。

前置状态：

- 用户已通过 Windows 任务管理器干掉原来的 `miopunch.exe` 进程。
- WSL2 检查 Windows 侧无残留 `miopunch.exe`。
- 重新启动 Windows daemon：
  - PID：`70292`
  - command line：`miopunch.exe up --state_path C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\windows\state.json`
- 重新启动 Linux daemon：
  - LocalAPI：`unix:/tmp/miopunch-link-lab/run/linux/localapi.sock`
  - state：`/tmp/miopunch-link-lab/run/linux/state.json`
- 双方 `ls` 都返回 `peer_count=1`。

实验命令：

```bash
/tmp/miopunch-link-lab/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-link-lab/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-link-lab/run/reports/linux-to-windows-udp-fresh-20260514-2012.md \
  ping -u FI5NWWE3G67A3MZ4FH5NTDWYWI
```

结果：

```text
task_id=RNYTE3YHSJUEZ3FCR62ENH22QA
reason_code=OK
sid=2cf829bf2fd85bc2163d2e036162cfc3
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

关键约束：

- 本次报告没有 `session_reused=true`。
- 因此这是一次可用于判断 UDP punching 的 fresh Linux -> Windows UDP 样本。

Linux 发起侧 topology：

```text
neighbors.active:
  peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
  data_proto=quic
  path_family=udp4
  healthy=true

attempts:
  attempt_path=punching_ipv4
  data_proto=quic
  path_family=udp4
  outcome=ok

payloads:
  evidence=ping=ok
```

Windows 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

Windows acceptor 日志：

```text
pocacceptor incoming attempt:
  sid=2cf829bf2fd85bc2163d2e036162cfc3
  dial_id=17787607832f72f1ff0e1ea58d

pocacceptor connectivity attempt ready:
  path=punching_ipv4
  tcp_conns=0
  protocol=quic
  selected_view=cn
  selected_reason=stun_rtt

pocacceptor connectivity attempt delegated:
  path=punching_ipv4
```

当前判断：

- UDP punching 本身在 fresh Windows/WSL2 mirrored 环境里可以成功。
- “Windows 发起 OK，但 Linux 侧 UI/topology 没变化”这类现象，现在有对称解释：被动 acceptor 侧真实收到了连接，但没有把 attempt/payload/active evidence 写入 task manager topology。
- 被动侧 topology 为空不能再被当作“链路没通”的证据；必须同时看 acceptor 日志。
- 下一步需要继续验证 idle 后两侧状态如何变化，确认是否由应用层 `DefaultSessionIdleTimeout=2m` 主动关闭。

### 4.10 UDP idle 后由应用层 session idle close 收掉

实验对象：

- 沿用 `4.9` 的 fresh Linux -> Windows UDP session。
- `sid=2cf829bf2fd85bc2163d2e036162cfc3`
- 最后一次业务 activity：约 2026-05-14 20:13:09。

实验方法：

- 成功 ping 后不再主动发 ping。
- 等待约 135 秒后查询双方 topology 和日志。

Linux 发起侧 topology：

```text
neighbors.active=[]

neighbors.unhealthy:
  peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
  data_proto=quic
  path_family=udp4
  last_activity_unix_ms=1778760789396
  close_reason=idle_timeout

reconnect_attempts:
  reason_code=idle_timeout
  retry_budget=1
  stop_condition=session_closed
```

Windows 被动侧日志：

```text
2026-05-14 20:15:58
pocacceptor accept stream failed:
  proto=quic
  sid=2cf829bf2fd85bc2163d2e036162cfc3
  path_family=udp4
  err=Application error 0x0 (remote): idle_timeout
```

当前判断：

- “等一会儿就断”这次有明确证据指向 miopunch 应用层 idle close，而不是 UDP NAT 映射自然失效。
- QUIC transport keepalive 没有阻止 miopunch session idle close；这和代码事实一致：`lastActivity` 只按逻辑 stream activity 更新。
- 如果产品目标是长期在线，需要应用层保活或 session lifecycle 语义重做；只依赖 QUIC keepalive 不够。

### 4.11 fresh Windows -> Linux UDP 成功，但 Linux 被动侧 topology 没有记录 active

实验时间：2026-05-14 20:19-20:21。

实验前污染样本：

- 第一次 Windows -> Linux UDP 命令的 `--report` 路径没有正确保护反斜杠。
- CLI 输出最后是 `export report` 失败：
  - `open C:.UsersstatiAppDataLocalTemp...: Access is denied`
- 但日志显示该命令实际已经完成了 UDP punching：
  - `task_id=MUCL7P3Y2LARR7TD36URYOFYTE`
  - `attempt_path=punching_ipv4`
  - `path_family=udp4`
- 随后第二次命令 `GY5RKATX2NMNLWQIDZ7PZUEBD4` 出现：
  - `session_reused=true`
- 因此这两次不能作为 fresh Windows -> Linux 证据。

清理动作：

- 停止 Windows PID `70292`。
- 确认无 Windows `miopunch.exe` 残留。
- 重新启动 Windows daemon：
  - PID：`66128`
  - state：`C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\windows\state.json`
- 重新启动 Linux daemon：
  - LocalAPI：`unix:/tmp/miopunch-link-lab/run/linux/localapi.sock`
- 双方 `ls` 都返回 `peer_count=1`。

有效实验命令：

```bash
/mnt/c/Users/stati/AppData/Local/Temp/miopunch-link-lab/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\windows\reports\windows-to-linux-udp-fresh-20260514-2019.md' \
  ping -u DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=DMRMOPA773OEKYI6SYBZERLGFU
reason_code=OK
sid=fac303d5226654b46c92a8920d42bf75
data_proto=quic
quic_cc=bbr
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

关键约束：

- 本次报告没有 `session_reused=true`。
- 因此这是一次可用于判断 UDP punching 的 fresh Windows -> Linux UDP 样本。

Windows 发起侧 topology：

```text
neighbors.active:
  peer_id=DHKCRZ62B7FH73QWFHHM524NFQ
  data_proto=quic
  path_family=udp4
  healthy=true

attempts:
  attempt_path=punching_ipv4
  data_proto=quic
  path_family=udp4
  outcome=ok

payloads:
  evidence=ping=ok
```

Linux 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

Linux acceptor 日志：

```text
pocacceptor incoming attempt:
  sid=fac303d5226654b46c92a8920d42bf75
  dial_id=1778761214b7ffb1c8e2413e36

pocacceptor connectivity attempt ready:
  path=punching_ipv4
  tcp_conns=0
  protocol=quic
  selected_view=cn
  selected_reason=stun_rtt

pocacceptor connectivity attempt delegated:
  path=punching_ipv4
```

当前判断：

- 用户最初描述的“Windows ping Linux 显示 OK，但 Linux 侧没有变化”已经被 fresh 实验复现。
- 这不是 UDP 没打通；UDP path、QUIC dataplane、hello、ping 都已成功。
- 问题在观测面/状态面：被动 acceptor session 没有进入 Linux task manager topology 的 active/attempt/payload evidence。
- 因此后续修复方向应优先考虑统一 active dial session 与 passive accepted session 的 runtime evidence，而不是先改 UDP punching 算法。

### 4.12 fresh Windows -> Linux TCP 失败在 TLS winner election

实验时间：2026-05-14 20:21-20:24。

清理动作：

- 为避免 `ping -t` 复用刚才 UDP session，重启双方 daemon。
- Windows daemon：
  - PID：`75448`
  - state：`C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\windows\state.json`
- Linux daemon：
  - LocalAPI：`unix:/tmp/miopunch-link-lab/run/linux/localapi.sock`
- 双方 `ls` 返回 `peer_count=1`。

第一次 TCP 命令：

```bash
/mnt/c/Users/stati/AppData/Local/Temp/miopunch-link-lab/bin/miopunch.exe \
  --format json \
  --report 'C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\windows\reports\windows-to-linux-tcp-fresh-20260514-2022.md' \
  ping -t DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=JVN3HJDZQF7BBHFUB3AY44L3RM
stage=DataplaneHandshake
reason_code=UNAVAILABLE
exit_code=3
dial peer: session shutdown
```

发起侧 Windows topology：

```text
neighbors.active=[]
neighbors.unhealthy:
  peer_id=DHKCRZ62B7FH73QWFHHM524NFQ
  data_proto=tls
  path_family=tcp4
  close_reason=transport_fatal

attempts:
  outcome=fail
  stage=PeerContact
  reason_code=UNAVAILABLE
```

被动侧 Linux acceptor 日志：

```text
pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=1
  protocol=quic

pocacceptor tls session failed:
  path=punching_tcp4
  err=tls election failed: follower did not receive winner signal
```

第二次 TCP retry：

```text
task_id=HGQWSYQI62NRN6QZRPAXHOYBJY
stage=DataplaneHandshake
reason_code=UNAVAILABLE
dial peer: session shutdown
```

Linux acceptor 再次出现同类日志：

```text
pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=1

pocacceptor tls session failed:
  err=tls election failed: follower did not receive winner signal
```

代码事实：

- `connectivity/attempt_tcp.go` 的 TCP punching 流程：
  - 收到第一条 TCP connection 后记录 winner。
  - `settleWindow=200ms` 后取消 punching loop。
  - 返回 `AttemptResult{Path: "punching_tcp4", TCPConns: tcpConns}`。
- `internal/task/poc_dial.go` 在 `punching_tcp4` 后调用 `dataplane.DialTLSSession`。
- `internal/pocacceptor/acceptor.go` 在 `punching_tcp4` 后调用 `dataplane.ServeTLSSession`。
- `dataplane/session_transport.go` 的 `newTLSSession`：
  - active dial 侧 `asClient=true`，角色为 `visitor`。
  - passive serve 侧 `asClient=false`，角色为 `client`。
  - 双方先 `convergePinnedTLS`，再创建 yamux session。
- `dataplane/tls_stream.go` 的 winner election：
  - `tlsElectionTimeout=2s`
  - `visitor` 是 leader，负责写 `miopunch/tls-election/v0:keep:<token>`。
  - `client` 是 follower，负责读这个 winner signal。
  - 当前被动侧报错正是 follower 未读到 leader 的 keep frame。

当前判断：

- 这不是“TCP 完全打不通”：两次 fresh TCP 都到达 `punching_tcp4`，且被动侧拿到了 `tcp_conns=1`。
- 失败点在 TCP dataplane 层：pinned TLS / winner election / yamux 前置阶段。
- 当前最可疑的不是 MQTT、governance 或 topology，而是 TCP punching 产出的单条连接在 TLS election 阶段没有完成双方收敛。
- 需要进一步增强 TCP dataplane 可观测性：active leader 侧应记录 TLS handshake 成功数、election writeFrame 成败、winner origin；passive follower 侧应记录 readFrame 的具体错误，而不是只保留 `follower did not receive winner signal`。
- 可能的修复方向之一：调整 TCP election 在单连接场景下的策略或扩大/重构 election 诊断；但现在先不直接改实现，先补足证据。

### 4.13 fresh Linux -> Windows TCP 成功，方向性差异明显

实验时间：2026-05-14 20:24-20:26。

实验目的：

- 对比 `4.12` 中 Windows -> Linux TCP 稳定失败。
- 验证当前环境是否是“所有 TCP 都坏”，还是方向/角色/连接数量组合导致失败。

实验命令：

```bash
/tmp/miopunch-link-lab/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-link-lab/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-link-lab/run/reports/linux-to-windows-tcp-fresh-20260514-2024.md \
  ping -t FI5NWWE3G67A3MZ4FH5NTDWYWI
```

结果：

```text
task_id=M5B3TRMH5PYWCMEAPV74QVCQ3Y
reason_code=OK
sid=2cf829bf2fd85bc2163d2e036162cfc3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

Linux 发起侧 topology：

```text
neighbors.active:
  peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
  data_proto=tls
  path_family=tcp4
  healthy=true

attempts:
  attempt_path=punching_tcp4
  data_proto=tls
  path_family=tcp4
  outcome=ok

payloads:
  evidence=ping=ok
```

Windows 被动侧日志：

```text
pocacceptor incoming attempt:
  sid=2cf829bf2fd85bc2163d2e036162cfc3

pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=4
  protocol=quic
```

Windows 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

对比结论：

- Linux -> Windows TCP 成功，Windows -> Linux TCP 失败；因此“TCP 不能打洞”这个结论不成立。
- 当前最明显差异：
  - Windows -> Linux 失败样本：Linux 被动侧 `tcp_conns=1`，随后 TLS election follower 读不到 winner signal。
  - Linux -> Windows 成功样本：Windows 被动侧 `tcp_conns=4`，最终 hello/ping 成功。
- 这提示 TCP dataplane election 对“只有一条 TCP connection 成功”的情况可能更脆弱，或者某个方向下 leader/follower 时序存在竞态。
- TCP 成功样本也再次证明被动侧 topology gap 不只影响 UDP：Windows 被动侧日志确认接受了 `punching_tcp4`，但 topology 仍没有 active/attempt/payload evidence。

### 4.14 TCP TLS election 根因定位：leader 等慢候选，follower election 先超时

本轮在临时分支 `debug/tcp-election-diagnostics-20260514` 上做诊断。

临时二进制路径：

```text
Linux:   /tmp/miopunch-link-lab/bin/miopunch-linux
Windows: C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\bin\miopunch.exe
```

新增诊断日志点：

- `dataplane/tls_stream.go`
  - `tcp tls converge start`
  - `tcp tls converge deadlines`
  - `tcp tls handshake start/ok/failed`
  - `tcp tls converge handshakes ready`
  - `tcp tls election deadlines`
  - `tcp tls election leader signal start/ok/failed`
  - `tcp tls election follower read start/ok/failed`
- 这些日志只用于临时定位，包含 `sid`、角色、candidate 数、origin、local/remote address、deadline、remaining_ms、handshake elapsed、read/write 错误。

#### 4.14.1 复现失败：Windows -> Linux TCP，默认 2s election timeout

命令：

```text
Windows 发起:
miopunch.exe --format json --report ...windows-to-linux-tcp-diag-20260514-02.md ping -t DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=VHQN7KXKF6XUDNUVKGK6XJGYLU
stage=DataplaneHandshake
reason_code=UNAVAILABLE
dial peer: session shutdown
```

Linux 被动侧日志：

```text
pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=1

tcp tls converge start:
  role=client
  as_client=false
  candidates=1

tcp tls handshake ok:
  origin=accept
  local=192.168.4.5:48717
  remote=192.168.4.5:52294
  elapsed_ms=1

tcp tls election deadlines:
  election_deadline=2026-05-14T20:43:22.584381739+08:00
  election_remaining_ms=1999

tcp tls election follower read failed:
  err=read tcp4 192.168.4.5:48717->192.168.4.5:52294: i/o timeout
```

Windows 主动侧日志：

```text
tcp tls converge start:
  role=visitor
  as_client=true
  candidates=3

tcp tls handshake ok:
  origin=dial
  local=192.168.4.5:52294
  remote=192.168.4.5:48717
  elapsed_ms=1133

tcp tls handshake failed:
  remote=104.28.193.48:61357
  elapsed_ms=6000
  err=context deadline exceeded

tcp tls converge handshakes ready:
  successes=1

tcp tls election leader signal ok:
  origin=dial
  local=192.168.4.5:52294
  remote=192.168.4.5:48717
```

关键事实：

- Linux follower 只有 1 条 TCP candidate，TLS handshake 很快成功，然后进入 2 秒 election read。
- Windows leader 有 3 条 TCP candidate，其中真实可用的 `192.168.4.5 -> 192.168.4.5` handshake 成功，但另一个公网/辅助 candidate 等到 6 秒 handshake deadline 后才失败。
- 当前 `convergePinnedTLS` 是“等待所有 TLS handshake goroutine 结束，才进入 election”。
- 因此 leader 发 winner signal 的时间晚于 follower 的 2 秒 read window。
- leader 后续 `writeFrame` 显示 `signal ok`，但 follower 已经超时并关闭 session，所以主动侧最终看到 `session shutdown`。

这轮已经能排除：

- 不是 TCP punching 没有建立：被动侧已经有 `tcp_conns=1`，两端 TLS handshake 都成功过。
- 不是 TLS pinning 身份校验失败：双方都有 `tcp tls handshake ok`。
- 不是 leader 写 signal 失败：leader 日志有 `tcp tls election leader signal ok`。
- 不是治理/成员授权问题：失败发生在 dataplane TLS/yamux 之前。

#### 4.14.2 对照验证：临时把 election timeout 从 2s 改成 8s

临时改动：

```go
tlsElectionTimeout = 8 * time.Second
```

同一方向同一命令：

```text
Windows 发起:
miopunch.exe --format json --report ...windows-to-linux-tcp-diag-20260514-03-election8s.md ping -t DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=NMJZD2T2VNCGCUMIF4CDJ5NS2E
stage=CapabilityHandshake
reason_code=OK
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

日志事实：

```text
Linux follower:
  election_remaining_ms=7999
  tcp tls election follower read ok
  frame_len=62
  keep_prefix=true

Windows leader:
  tcp tls election leader signal ok
```

结论：

- 只扩大 election read window，原来稳定失败的 Windows -> Linux TCP 就成功了。
- 因此根因已经高度收敛到 TCP TLS election 的时序设计：
  - follower 进入 election 太早且窗口太短；
  - leader 在进入 election 前等待所有候选 TLS handshake 完成；
  - 当两端候选数量不对称时，candidate 少的一侧会先进入 follower read 并提前超时。
- 这仍然不是最终修复方案，只是一个验证假设的临时实验。

#### 4.14.3 候选修复方向

更合理的修复方向不应只是把 2s 改大：

- 方向 1：leader 不要等所有 candidate handshake 完成才发 winner signal；一旦有可用 TLS connection，就可以进入 election 或快速 signal。
- 方向 2：follower 的 election read window 要覆盖 leader 侧最大 handshake convergence delay，或者双方需要共享同一个阶段预算。
- 方向 3：为 TCP candidate 收敛引入“首个成功后短暂 settle window”，而不是固定等待所有失败候选跑完。
- 方向 4：对明显不可达/低优先级 assisted/public candidate 做优先级排序或更短 handshake deadline，避免拖住已成功的 direct/mirrored path。
- 方向 5：保留当前诊断日志中的关键字段，后续改成 debug level 或事件证据，避免再次只能看到 `follower did not receive winner signal` 这种过粗错误。

当前最推荐先实现方向：

```text
first-success fast path + bounded settle window
```

原因：

- 它解决的是根因：leader 不应被慢失败 candidate 阻塞。
- 它不会单纯放大失败等待时间。
- 它仍允许短时间收集多条成功 TLS candidate，让 election 有选择余地。

更具体地说，当前代码在 `dataplane/tls_stream.go` 的行为类似：

```text
启动所有 TCP candidate 的 TLS handshake
等待所有 handshake goroutine 结束
收集 successes
再进入 leader/follower election
```

这个设计的问题是：一个好连接已经完成 TLS handshake 后，仍然要等其它慢失败连接结束。只要一侧 candidate 多、另一侧 candidate 少，少的一侧就可能先进入 follower read，并在 leader 还没开始 signal 前超时。

推荐修复后的行为应类似：

```text
启动所有 TCP candidate 的 TLS handshake
第一个 TLS handshake 成功后，立即启动一个很短的 settle window
settle window 内继续收集其它快速成功的 TLS conn
settle window 到期后，取消仍未完成的慢 handshake
用已经成功的 TLS conn 立即进入 election
关闭未选中的 TLS conn 和原始 TCP conn
```

这里的关键不是把 `tlsElectionTimeout` 从 2 秒改成 8 秒。8 秒实验只是证明“leader 太晚 signal，follower 太早 timeout”这个假设。真正修复应该减少 leader 进入 election 前的等待时间，而不是让 follower 永远等更久。

建议的初始参数：

- TLS handshake 总预算可以保留当前 6 秒，作为“完全没有成功连接”时的最大失败等待。
- 第一个 TLS handshake 成功后，开启 `tlsHandshakeSettleWindow`，初始可取 200ms，和 TCP punching 层 `settleWindow=200ms` 保持一致。
- election timeout 可以先恢复或保留较小值，重点验证 leader 是否能在 follower read window 内稳定 signal。
- 若 200ms settle window 在复杂 NAT 场景下太短，再通过实验调整；不要先用大 timeout 掩盖调度问题。

实现时要注意的细节：

- `resultCh` 不应该只在 `wg.Wait()` 后关闭才驱动主流程；主流程要能在第一个成功结果到达时开始 settle。
- settle 到期后应 cancel handshake context，让慢 handshake 尽快退出。
- 已成功但未被 election 选中的 TLS conn 必须关闭。
- handshake goroutine 可能在主流程已经选出 winner 后才返回，必须避免结果发送阻塞或泄漏。
- event/log 里应记录 `candidates`、`successes`、`first_success_elapsed_ms`、`settle_window_ms`、`winner_origin`，否则以后会再次只能看到过粗的 `follower did not receive winner signal`。

验收标准：

- Windows -> Linux fresh TCP 在 WSL2 mirrored 环境稳定得到 `attempt_path=punching_tcp4`、`path_family=tcp4`、`hello=ok`、`ping=ok`。
- Linux -> Windows fresh TCP 不能回归。
- 单 candidate、多个 candidate、慢失败 candidate 混入时都不能被慢失败 candidate 拖过 follower election window。
- 失败日志要能区分：
  - 没有 TCP conn；
  - TCP conn 有，但 TLS handshake 全失败；
  - TLS handshake 有成功，但 election signal/read 失败；
  - yamux/session 后续失败。

### 4.15 类似风险：后续需要验证的地方

这几个点和 4.14 属于同类“阶段预算、候选筛选、复用策略不一致”风险。当前还没有把它们全部证明为 bug，先作为后续验证清单记录。

#### 4.15.1 direct TCP dial 也有 first-success settle，但需要验证慢候选是否仍会污染耗时

`connectivity/attempt_tcp.go` 的 direct TCP 分支已经有类似逻辑：

```text
第一个 TCP dial 成功后，200ms 后 cancel
返回收集到的 TCP conns
```

风险点：

- 它解决的是 TCP connect 阶段，不解决后续 TLS handshake 阶段。
- 返回的 `TCPConns` 顺序主要由到达顺序决定，不代表路径质量最优。
- 需要验证 direct TCP + 多候选情况下，后续 TLS converge 是否仍会被慢 handshake 拖住。

2026-05-14 21:09 状态：

- 现场 WSL2/Windows fresh TCP 仍然走 `punching_tcp4`，没有命中 `direct_tcp4` 成功样本。
- 因此 direct TCP 分支暂时还是代码层风险，尚未在当前现场环境独立复现。
- 但风险逻辑和 punching TCP 一致：direct TCP 分支返回的是 `TCPConns`，后续仍进入同一个 `dataplane/tls_stream.go` 的 TLS converge；只要 direct TCP 交出多条 TCP conn，其中一条 TLS handshake 慢失败，就会触发同类等待问题。
- 后续要完整验证 direct TCP，需要构造能稳定命中 `direct_tcp4` 且混入慢 TLS candidate 的场景，或者补一个 targeted dataplane/connectivity 单测。

#### 4.15.2 TCP punching 层和 TCP dataplane 层有两套 settle/timeout 语义

TCP punching 层：

```text
first TCP conn -> settleWindow=200ms -> return TCPConns
```

TCP dataplane 层：

```text
all TLS handshakes complete -> election
```

风险点：

- 两层语义不一致：punching 层已经选择“不要等慢候选”，dataplane 层又重新等待慢候选。
- 后续修复应让 TLS converge 使用同样的 first-success/bounded-settle 思路。
- 需要测试“TCP punching 快速返回 1 条好 conn + 1 条慢 conn”时，dataplane 不再被慢 conn 拖住。

2026-05-14 21:09 fresh Windows -> Linux TCP 已验证这个风险是现场真实行为。

报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-tcp-budget-20260514-2110.md

task_id=7NIDUXUOULL3WURXI55BHHRR5E
reason_code=OK
attempt_path=punching_tcp4
path_family=tcp4
data_proto=tls
hello=ok
ping=ok
```

时间线：

```text
21:09:23.528 Windows task 进入 PunchAttempt
21:09:24.526 Linux 被动侧 connectivity attempt ready: path=punching_tcp4 tcp_conns=1
21:09:24.527 Linux follower TLS handshake start
21:09:24.529 Linux follower TLS handshake ok, candidates=1 successes=1
21:09:24.734 Windows leader TLS handshake start, candidates=3
21:09:26.556 Windows leader 好 candidate TLS handshake ok
21:09:29.335 Windows leader 慢 candidate TLS handshake failed
21:09:29.335 Windows leader 才进入 election 并 signal ok
```

结论：

- punching 层已经在约 1 秒内交出可用 TCP conn。
- follower 因为只有 1 个 candidate，几乎立刻进入 election read。
- leader 的好 TLS conn 在 21:09:26 已经可用，但当前 converge 仍等到 21:09:29 慢 candidate 失败后才 election。
- 这就是 4.14 根因在现场的另一组证据：punching 层 first-success/bounded-settle 和 dataplane 层 wait-all TLS converge 语义冲突。
- 当前这次因为临时 `tlsElectionTimeout=8s` 所以最终成功；若恢复 2s，follower 很容易在 leader signal 前超时。

#### 4.15.3 `ping -t` / `ping -u` 的 session reuse 会污染实验结论

`internal/task/poc_dial.go` 里 session reuse 先按配置协议找 session，找不到又 fallback 到 TLS session，但没有把用户这次命令指定的 path family 作为强约束。

风险点：

- `ping -t` 可能复用既有 UDP session，导致用户以为测了 TCP，实际没有 fresh TCP attempt。
- `ping -u` 也可能被已有 session 干扰。
- 后续需要补一个明确语义：
  - 调试命令支持强制 fresh attempt；或
  - session reuse 必须尊重用户指定的 protocol/path family；或
  - 报告里强制突出 `session_reused=true`，并标记该次不能证明当前打洞能力。

2026-05-14 21:07 与 21:11 已现场复现双向污染。

步骤：

```text
1. 重启双方 daemon，清空 runtime session。
2. Linux -> Windows 执行 fresh UDP:
   report=/tmp/miopunch-link-lab/run/reports/linux-to-windows-udp-reuse-seed-20260514-2107.md
3. 同一 Linux daemon 立即执行 ping -t:
   report=/tmp/miopunch-link-lab/run/reports/linux-to-windows-tcp-after-udp-reuse-20260514-2108.md
```

第一次 UDP 结果：

```text
task_id=W6IGFQYBS7BEQQB6NMWXIVKM2A
attempt_path=punching_ipv4
path_family=udp4
data_proto=quic
hello=ok
ping=ok
```

第二次 `ping -t` 结果：

```text
task_id=E7N4QNZHP7S33FM6MUWRF4W6UU
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
ping=ok
```

反向步骤：

```text
1. 重启双方 daemon，清空 runtime session。
2. Windows -> Linux 执行 fresh TCP:
   report=C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-tcp-budget-20260514-2110.md
3. 同一 Windows daemon 立即执行 ping -u:
   report=C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-udp-after-tcp-reuse-20260514-2111.md
```

fresh TCP 结果：

```text
task_id=7NIDUXUOULL3WURXI55BHHRR5E
attempt_path=punching_tcp4
path_family=tcp4
data_proto=tls
hello=ok
ping=ok
```

后续 `ping -u` 结果：

```text
task_id=KTJNO5H2BRM6OGBB2XJJWA73RI
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
ping=ok
```

结论：

- 这次 `ping -t` 完全没有证明 TCP punching；它复用了上一条 UDP/QUIC session。
- 反向也成立：`ping -u` 可以复用上一条 TCP/TLS session，因此也不能证明 fresh UDP punching。
- 代码层原因和现场事实一致：`dialPeerStream` 的 reuse key 没有把 `tcp_only` 转成 `PathFamilyTCP4` 约束。
- 后续所有 TCP/UDP 能力实验，只要报告中出现 `session_reused=true`，就必须判定为“session 可复用性样本”，不能判定为 fresh punching 样本。

#### 4.15.4 decision 层的 TCP `ReadTimeoutMs` 与 attempt 层固定预算不完全一致

`internal/punchdecision/decision.go` 会生成 TCP `ReadTimeoutMs`，但 `connectivity/attempt_tcp.go` 目前主要使用固定预算：

```text
auto:     totalBudget=5s, dialTimeout=1.5s
tcp_only: totalBudget=10s, dialTimeout=2.5s
```

风险点：

- 日志和报告里展示的 `read_timeout_ms` 可能让人以为 attempt 完全按该预算执行。
- 后续如果 NAT analyzer 输出更复杂的 TCP 行为，attempt 层可能没有完整尊重。
- 当前不是 4.14 的直接根因，但容易造成排查误判，需要单独验证。

2026-05-14 21:09 已确认这个风险至少是“观测语义不一致”。

现场报告中同一轮 Windows -> Linux TCP 显示：

```text
tcp_punching_plan: role=receiver send_delay_ms=0 read_timeout_ms=10000
peer_tcp_punching_plan: role=sender send_delay_ms=200 read_timeout_ms=9800
```

但 `connectivity/attempt_tcp.go` 的实际 attempt 参数是：

```text
tcp_only:
  totalBudget=10s
  dialTimeout=2.5s
  dialRoundInterval=200ms
  settleWindow=200ms
auto:
  totalBudget=5s
  dialTimeout=1.5s
```

本轮时间线也能看出实际行为不是“完整等待 read_timeout_ms”：

```text
21:09:23.528 Windows 进入 PunchAttempt
21:09:24.526 Linux 已 ready: tcp_conns=1
21:09:24.732 Windows 进入 DataplaneHandshake
```

结论：

- `read_timeout_ms` 目前更像 decision 层给出的行为描述/计划字段，不等同于 attempt 层每个 read/dial 操作的真实超时。
- attempt 层的 `totalBudget/dialTimeout/settleWindow` 才决定实际 TCP punching 调度。
- 这不是 4.14 的直接根因，但会污染排查语言：看到 `read_timeout_ms=10000` 不能推导出 TCP punching 或 TLS election 一定等满 10 秒。
- 后续建议把报告字段拆清楚：decision planned read timeout、attempt total budget、dial timeout、settle window、实际 elapsed。

#### 4.15.5 被动侧 topology evidence 缺失会掩盖真实成功路径

多次实验已经看到：被动侧日志确认 accept 了 UDP/TCP attempt，但 topology 里没有 active/attempt/payload evidence。

风险点：

- UI 可能显示“没有变化”，但底层其实已经 accept 过连接。
- 用户会把 topology 缺口误判成 UDP/TCP 打洞失败。
- 后续需要定义 passive accept 的 topology 表达，例如 `attempt_path=passive_accept_udp4/tcp4`，并记录 payload/stream evidence。

2026-05-14 21:07 已再次现场复现。

Linux 主动 UDP 成功后，Linux topology：

```text
neighbors.active:
  peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
  data_proto=quic
  path_family=udp4
  healthy=true

attempts:
  attempt_path=punching_ipv4
  data_proto=quic
  path_family=udp4
  outcome=ok

payloads:
  evidence=ping=ok
```

Windows 被动侧日志：

```text
pocacceptor incoming attempt:
  sid=2cf829bf2fd85bc2163d2e036162cfc3
  dial_id=177876404137dca5def922e7d2

pocacceptor connectivity attempt ready:
  path=punching_ipv4
  tcp_conns=0
  protocol=quic
  selected_view=cn
  selected_reason=stun_rtt

pocacceptor connectivity attempt delegated:
  path=punching_ipv4
```

Windows topology 同时仍为：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

结论：

- 被动侧确实参与了 UDP punching，并进入 acceptor delegated path。
- 桌面/topology 仍完全没有 passive evidence。
- 该问题已经不是推测，而是稳定复现的 observability gap。

### 4.16 链路层 Bug 定位行动纲领

这一节是后续继续找 bug / 定位根因的行动纲领，不是修复方案。当前 Windows + WSL2 mirrored 网络环境里，两端网络本身相对可控，并且可以由 WSL2 驱动 Windows/Linux 两个 daemon，因此适合把链路层高风险流程尽量扫完。

目标：

- 先找问题、定位原因，不急着进入 production fix。
- 每个疑点都尽量形成“代码事实 + 现场日志 + 实验样本 + 当前判断”。
- 问题之间可能互相关联，先把根因图谱铺开，再统一讨论修复顺序。
- 范围限定在链路层主线：候选收集、signaling/decision、TCP/UDP attempt、dataplane、hello 前后 session lifecycle、keepalive、topology evidence。

#### 4.16.1 实验纪律

每轮实验前置要求：

- 清理 Windows/Linux 旧 daemon，确认 LocalAPI 指向当前实验进程。
- Windows/Linux 使用同一 commit 编译出的临时二进制。
- 使用隔离 state，避免桌面旧 state、旧 admin、旧 session 污染实验。
- 双方 broker 保持一致，当前优先 `broker.emqx.io:1883`。
- fresh TCP/UDP 能力实验必须重启 daemon 或显式清空 session；只要报告出现 `session_reused=true`，该样本不能证明 fresh punching 能力。

每个实验至少记录：

- hypothesis：本轮要验证什么。
- command：Windows/Linux 实际命令。
- task id、sid、peer id、report path。
- 两端关键日志片段。
- `attempt_path`、`path_family`、`data_proto`、`hello`、`ping`、`session_reused`。
- 当前结论状态：`confirmed`、`rejected`、`inconclusive`、`deferred`。

#### 4.16.2 风险扫描框架

后续按阶段扫描，而不是按“连不上”这种表面现象扫描：

```text
state/config
  -> gather candidates
  -> MQTT signaling / decision
  -> connectivity attempt
  -> TCP/UDP dataplane
  -> hello / governance boundary
  -> session registry / reuse
  -> idle / keepalive
  -> passive acceptor / topology evidence
```

每个阶段都重点找这几类风险：

- 阶段预算不一致：例如 TCP punching 已经 bounded settle，但 TLS converge 仍 wait-all。
- 候选筛选不一致：例如 direct、assisted、mapped、candidate offset 的语义不同。
- 状态提前写入：例如 session 在 hello 成功前进入 manager。
- 状态未写回：例如 passive acceptor 成功但 topology 没有 evidence。
- 错误被折叠：例如最终只看到 `session shutdown`，看不到 TLS election 或 hello 的具体失败点。

#### 4.16.3 重点实验矩阵

候选与 decision：

- 验证 TCP `base+100`：比较双方 `tcp_p`、实际 TCP listener、TCP STUN mapped ports、`TCPCandidateAddrs +100` 是否符合现场行为。
- 验证 direct TCP 为什么当前现场没有命中：重点看 private TCP direct 是否被 decision 层过滤为 non-direct，以及是否只剩 assisted/punching。
- 验证 auto 模式顺序：确认 TCP-first 会不会在 TCP 慢失败时拖慢 UDP fallback 或污染用户判断。
- 验证 CN/global STUN arbitration：确认 selected view 是否让 TCP/UDP candidate 集合不一致。

TCP attempt / dataplane：

- 继续复现 `punching_tcp4 tcp_conns=1` 与 leader 多 candidate 的非对称场景。
- 补齐每条 TCP conn 的 origin、local/remote、candidate source、connect elapsed、TLS elapsed、失败原因。
- 单独构造 direct TCP 多 candidate 样本，确认 direct TCP 是否也会被 TLS wait-all 拖住。
- 区分四类失败：没有 TCP conn、TLS 全失败、TLS 成功但 election 失败、yamux/session 后失败。

UDP / QUIC：

- 双向 fresh UDP：确认 active 侧 payload ok、passive acceptor 日志 ok、passive topology empty 的稳定性。
- 等待 idle timeout：记录 QUIC close reason 与 miopunch `lastActivity`，验证底层 QUIC keepalive 不会更新应用层 session activity。
- 验证 UDP session reuse 与 fresh UDP punching 的边界，避免后续实验误判。

Session lifecycle / reuse：

- 完整复测四格矩阵：
  - UDP 后 `ping -t`
  - TCP 后 `ping -u`
  - Windows 发起后 Linux 发起
  - Linux 发起后 Windows 发起
- 验证 `SessionManager.Put(sess)` 在 hello 前发生的影响：hello/governance 失败后 session 是否仍短时出现在 active topology。
- 验证 `maintain-neighbors` 的 task result 与 active session snapshot 为什么会分裂。

Passive acceptor / topology：

- 对 UDP/TCP 被动 accept 分别采集：
  - `incoming attempt`
  - `connectivity attempt ready`
  - `connectivity attempt delegated`
  - `AcceptStream`
  - hello/ping/sh payload
  - close reason
- 明确哪些 evidence 只存在于 `pocacceptor.peerSessionRegistry`，哪些进入 `task.Manager.sessions/topology`。
- 当前目标是定位 observability gap，不先设计统一 session registry。

#### 4.16.4 临时诊断改动原则

允许在临时 debug 分支做诊断改动：

- 在 gather / decision / attempt / dataplane / session / acceptor 增加结构化日志字段。
- 在 report facts 中增加临时诊断字段，例如 candidate source、attempt elapsed、TLS first-success elapsed、settle/wait elapsed、session put/open/hello timing。
- 增加 debug-only 开关来强制 fresh/no-reuse、强制协议、缩短 idle timeout、或注入慢 candidate。

约束：

- 所有临时改动必须标注为 diagnostic。
- 不把临时 timeout 放大当成 production fix。
- 不在本阶段引入正式 public API 或产品语义。
- 每次改动都要记录目的、使用方式、实验结论和是否需要回滚。

#### 4.16.5 最低完成集

本轮链路层定位至少要回答：

- TCP `base+100` / candidate offset 在 WSL2 mirrored 下是否可靠。
- direct TCP 为什么当前没有命中，以及是否存在同类 TLS wait-all 风险。
- auto TCP-first 是否造成 UDP fallback 延迟或用户误判。
- UDP passive topology gap 的完整证据链。
- session reuse 污染的完整矩阵。
- hello 前 session 入库是否导致 active/topology 分裂。
- idle / keepalive 的精确行为和 close reason。
- TCP TLS election 根因是否仍然是最强证据链，且没有被其他问题混淆。

#### 4.16.6 实验记录：TCP direct / base+100 / assisted 路径

本节记录从 4.16 行动纲领开始后的第一组 TCP 风险扫描。目标不是修复，而是把 direct TCP 为什么没有命中、TCP `base+100` candidate 是否可靠、attempt 层 winner 与 dataplane 最终 winner 是否一致这几个问题固定证据。

实验前置 trick：

- Windows daemon 不使用自定义 `--localapi npipe:...`。自定义 npipe 在当前 Windows 代码路径下会因为 `operator_sid` 为空失败：
  - `failed to listen: empty operator_sid`
  - 后续 Windows CLI 也不传 `--localapi`，使用默认 `npipe:\\.\pipe\miopunch\localapi-<sid>`。
- Windows exe 如果仍有旧 daemon 进程占用，`go build -o .../miopunch.exe` 会失败：
  - `open .../miopunch.exe: permission denied`
  - 需要先清理 Windows `miopunch.exe` 进程。本轮用 `Stop-Process -Force` / `taskkill` 清理。
- broker 保持 `broker.emqx.io:1883`，避免把 MQTT broker 问题误判成 punching 问题。
- 本组样本都在重启双方 daemon 后单向执行，避免 `session_reused=true` 和双向并发日志交叉。

临时诊断改动：

- `dataplane/tls_stream.go` 保留之前 TCP TLS election 诊断日志，并临时把 `tlsElectionTimeout` 放大到 `8s`。这是为了绕过已知 2s election 前置问题，让后续链路证据能跑完；不是 production fix。
- `internal/punchdecision/decision.go` 增加 diagnostic 日志：
  - `visitor_peer_tcp_direct`
  - `visitor_tcp_candidate_addrs`
  - `visitor_tcp_assisted_addrs`
  - `client_peer_tcp_direct`
  - `client_tcp_candidate_addrs`
  - `client_tcp_assisted_addrs`
  - `tcp_selected_view/tcp_selected_reason`
- `connectivity/attempt.go` 增加 diagnostic 日志：
  - attempt 实际看到的 `peer_tcp_v4/peer_tcp_v6`
  - direct_tcp4 是否被跳过
- `connectivity/attempt_tcp.go` 增加 diagnostic 日志：
  - TCP punching target 列表和 source count
  - TCP punching first observed winner 的 source

focused validation：

```text
go test ./connectivity ./internal/punchdecision
```

结果：通过。

样本 1：Windows -> Linux，TCP-only。

命令：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\bin\miopunch.exe --format json --report C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-tcp-riskscan-20260514-02-diag.md ping -t DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=4YDJ3A2UQ2IEC65MH5FU6E6LHU
sid=fac303d5226654b46c92a8920d42bf75
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

关键证据：

```text
diagnostic tcp decision material:
  visitor_peer_tcp_direct=[
    [fd76:d462:c4e3:0:1842:a13c:234d:1bee]:45296
    [fd76:d462:c4e3:0:49f3:929a:a311:d6e7]:45296
  ]
  visitor_tcp_candidate_addrs=[
    104.28.225.47:46730
    104.28.225.47:46828
  ]
  visitor_tcp_assisted_addrs=[
    10.255.255.254:45296
    192.168.4.5:45296
    172.17.0.1:45296
  ]
```

```text
diagnostic attempt candidate summary:
  peer_tcp_v6=2
  peer_tcp_v4=0
  peer_tcp_v4_addrs=[]

diagnostic direct tcp4 skipped:
  reason=no_peer_tcp_v4_candidates
```

```text
diagnostic tcp punching targets:
  assisted_exact=3
  candidate_exact=2
  targets=[
    10.255.255.254:45296
    192.168.4.5:45296
    172.17.0.1:45296
    104.28.225.47:46730
    104.28.225.47:46828
  ]

diagnostic tcp punching winner:
  origin=dial
  winner_target_source=assisted_exact
  laddr=192.168.4.5:51414
  raddr=192.168.4.5:45296
```

TLS 证据：

```text
tcp tls handshake start:
  remote=104.28.225.47:46828
  remote=104.28.225.47:46730
  remote=192.168.4.5:45296

tcp tls handshake failed:
  remote=104.28.225.47:46828
  err=forcibly closed

tcp tls handshake failed:
  remote=104.28.225.47:46730
  err=forcibly closed

tcp tls handshake ok:
  remote=192.168.4.5:45296
```

样本 2：Linux -> Windows，TCP-only。

命令：

```text
/tmp/miopunch-link-lab/bin/miopunch-linux --localapi unix:/tmp/miopunch-link-lab/run/linux/localapi.sock --format json --report /tmp/miopunch-link-lab/run/reports/linux-to-windows-tcp-riskscan-20260514-02-diag.md ping -t FI5NWWE3G67A3MZ4FH5NTDWYWI
```

结果：

```text
task_id=6VSHNCZYY5LZGOH22NIVHHR5LY
sid=2cf829bf2fd85bc2163d2e036162cfc3
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

关键证据：

```text
diagnostic tcp decision material:
  visitor_peer_tcp_direct=[
    [fd76:d462:c4e3:0:c65:feb3:c246:537b]:64244
    [fd76:d462:c4e3:0:1842:a13c:234d:1bee]:64244
    [fd76:d462:c4e3:0:213d:70f5:b881:4065]:64244
    [fd76:d462:c4e3:0:44ff:3234:3f07:1515]:64244
  ]
  visitor_tcp_candidate_addrs=[
    104.28.225.48:46229
    104.28.225.48:46597
  ]
  visitor_tcp_assisted_addrs=[
    172.17.128.1:64244
    192.168.4.5:64244
  ]
```

```text
diagnostic attempt candidate summary:
  peer_tcp_v6=4
  peer_tcp_v4=0
  peer_tcp_v4_addrs=[]

diagnostic direct tcp4 skipped:
  reason=no_peer_tcp_v4_candidates
```

```text
diagnostic tcp punching targets:
  assisted_exact=2
  candidate_exact=2
  targets=[
    172.17.128.1:64244
    192.168.4.5:64244
    104.28.225.48:46229
    104.28.225.48:46597
  ]

diagnostic tcp punching winner:
  origin=dial
  winner_target_source=candidate_exact
  laddr=192.168.4.5:46151
  raddr=104.28.225.48:46597
```

TLS 证据：

```text
tcp tls handshake start:
  remote=104.28.225.48:46597
  remote=192.168.4.5:64244

tcp tls handshake ok:
  remote=192.168.4.5:64244

tcp tls handshake failed:
  remote=104.28.225.48:46597
  err=context deadline exceeded
```

当前定位结论：

- `direct_tcp4` 没有命中不是因为 direct TCP 尝试失败，而是当前 decision/候选分类没有产出 IPv4 direct candidate。
- 代码事实：`connectivity/tcp_candidates.go` 里 `isTCPDirectIPv4ListenAddr` 要求 IPv4 不是 private / CGNAT；Windows + WSL2 mirrored 下双方可用的 `192.168.4.5` 被归入 `tcp_assisted_addrs`，不是 `peer_tcp_direct_addrs`。
- 当前 `peer_tcp_direct` 主要是 IPv6 ULA；attempt 会先跑 `direct_tcp6`，但 800ms 超时失败，然后因为 `peer_tcp_v4=0` 明确跳过 `direct_tcp4`。
- TCP `base+100` / public `candidate_exact` 在本环境下不可靠：
  - Windows -> Linux：两个 public `candidate_exact` 都进入 TLS，但立即被 RST/forcibly closed；真正成功的是 `192.168.4.5` assisted。
  - Linux -> Windows：attempt 层 first observed winner 是 public `candidate_exact`，但 TLS 最终成功的是 `192.168.4.5` assisted；public candidate 的 TLS 在 6s 后 deadline。
- 新识别风险：attempt 层 `tcp punching winner` 只表示“最早观察到 TCP conn”，不等于 dataplane TLS 最终选中的可用连接。报告/topology 如果只看 attempt winner，可能误判为 public candidate 成功。
- TCP TLS wait-all 风险仍成立，而且这组样本再次强化：
  - 即使 assisted/private 连接很快可用，public candidate 仍会进入 TLS converge。
  - 当前 dataplane 等所有 TLS handshakes 完成后才进入 election，因此慢失败 public candidate 会拖住成功路径。

状态：

- `direct_tcp4 未命中原因`：confirmed。
- `base+100/public candidate 在 WSL2 mirrored 下不可靠`：confirmed。
- `attempt winner 与 TLS winner 可能不一致`：confirmed。
- `direct TCP 是否也存在 TLS wait-all 同类风险`：deferred。当前 direct_tcp4 没有候选，direct_tcp6 直接超时，尚未构造出 direct TCP 多连接样本。

#### 4.16.7 实验记录：auto TCP-first 失败后是否 UDP fallback

目标：验证 `p2p_network=auto` 在 TCP-first 已经拿到 TCP conn，但后续 TLS/dataplane 失败时，是否会继续 UDP fallback。

临时诊断条件：

- 把 `dataplane/tls_stream.go` 的 `tlsElectionTimeout` 从 8s 临时切回 2s。
- 目的：复现已知 TCP TLS election 失败，让 auto 的后续行为暴露出来。
- 该改动只是制造前置失败，不是 production fix。

focused validation：

```text
go test ./dataplane
```

结果：通过。

样本：Windows -> Linux，auto。

命令：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\bin\miopunch.exe --format json --report C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-auto-after-tcpfail-20260514-01.md ping DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=TS4FN7IQSZ3VU4QYROVDFUPFZU
stage=DataplaneHandshake
reason_code=UNAVAILABLE
exit_code=3
fact=dial peer: session shutdown
```

报告事实里同时存在 UDP 和 TCP plan：

```text
punching_plan:
  enabled=true
  role=receiver
  ttl=7
  read_timeout_ms=5200
  candidate_addrs=1
  assisted_addrs=3

tcp_punching_plan:
  enabled=true
  role=receiver
  read_timeout_ms=5000
```

关键日志：

```text
diagnostic attempt candidate summary:
  p2p_network=auto
  peer_udp_v6=2
  peer_udp_v4=0
  peer_tcp_v6=2
  peer_tcp_v4=0

diagnostic direct tcp4 skipped:
  reason=no_peer_tcp_v4_candidates

diagnostic tcp punching winner:
  winner_target_source=assisted_exact
  laddr=192.168.4.5:52702
  raddr=192.168.4.5:48125
```

TLS 失败链路：

```text
tcp tls handshake ok:
  remote=192.168.4.5:48125
  elapsed_ms=857

tcp tls handshake failed:
  remote=104.28.225.47:27128
  elapsed_ms=6000
  err=context deadline exceeded

tcp tls election deadlines:
  election_remaining_ms=2000

task fact:
  dial peer: session shutdown

task done:
  reason_code=UNAVAILABLE
```

被动侧对应日志：

```text
pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=1

tcp tls election follower read failed:
  err=i/o timeout

pocacceptor tls session failed:
  err=tls election failed: follower did not receive winner signal
```

代码事实：

- `connectivity.Attempt` 的 order 是 `tcp6 -> tcp4 -> udp6 -> udp4`。
- auto 下如果 TCP punching 返回 `AttemptResult{Path: "punching_tcp4", TCPConns: ...}`，`Attempt` 直接返回该结果。
- `internal/task/poc_dial.go` 拿到 `attemptRes.TCPConns` 后进入：

```text
if len(attemptRes.TCPConns) > 0 {
  dpCfg.Proto = tls
  sess, err = dataplane.DialTLSSession(...)
}
if err != nil {
  return nil, err
}
```

- 这里没有在 `DialTLSSession` 失败后重新调用 UDP attempt，也没有保存/恢复前面 UDP punching 的 fallback 机会。

当前定位结论：

- `auto` 当前不是“TCP dataplane 失败后自动 UDP fallback”。
- 一旦 TCP attempt 层先返回 TCP conn，后续 TLS/session 失败会直接结束 task。
- 因此用户看到的 auto 行为可能是：TCP-first 已经打到某条 TCP conn，但 TLS election / session 阶段失败，最终命令失败；不会自动切到 UDP，即使 UDP plan 同时可用。
- 这和 `attempt_path` 的分层有关：fallback 只发生在 `connectivity.Attempt` 内部选 path 之前；一旦 path 返回，dataplane 层失败不再触发更低优先级 path。

状态：

- `auto TCP-first 是否造成 UDP fallback 延迟或用户误判`：confirmed。
- 更精确地说，不只是延迟；在 TCP path 已返回但 dataplane 失败时，当前没有 UDP fallback。

#### 4.16.8 实验记录：强制 private IPv4 进入 gather direct 后仍未命中 direct_tcp4

目标：验证如果把 WSL2 mirrored 下实际可达的 private IPv4 `192.168.4.5` 临时放进 TCP direct bucket，是否能命中 `direct_tcp4`，并进一步观察 direct TCP 是否也存在 TLS wait-all / winner 不一致风险。

临时诊断条件：

- 在 `connectivity/tcp_candidates.go` 临时修改 `isTCPDirectIPv4ListenAddr`。
- 原始语义：IPv4 direct listen 只允许非 private、非 CGNAT。
- 临时语义：允许 private IPv4 进入 direct bucket，但仍排除 loopback / unspecified / link-local / multicast / CGNAT。
- 该改动故意违反当前测试语义，仅用于绕过前置候选分类，不能作为修复提交。

focused validation：

```text
go test ./connectivity
```

结果：预期失败。

失败原因不是编译或运行错误，而是 `TestClassifyTCP4ListenCandidates` 仍按 production 语义期望 private IPv4 进入 assisted bucket；临时绕过后 `10.0.0.2` 也进入 direct bucket，因此测试失败。该失败反而确认临时代码已经改变 gather 分桶。

样本：Windows -> Linux，TCP-only。

命令：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\bin\miopunch.exe --format json --report C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-directtcp4-private-diag-20260514-01.md ping -t DHKCRZ62B7FH73QWFHHM524NFQ
```

结果：

```text
task_id=OQDCJZQSRQO2NVPET3KZUM434I
stage=DataplaneHandshake
reason_code=UNAVAILABLE
fact=dial peer: context deadline exceeded
```

关键日志事实：

```text
gather snapshot:
  tcp_direct=5
  tcp_assisted=0

diagnostic tcp decision material:
  visitor_peer_tcp_direct=[fd17:625c:f037:2:a00:27ff:fedd:c3e5:...]
  visitor_tcp_assisted_addrs=[]
  client_peer_tcp_direct=[fd17:625c:f037:2:a00:27ff:fedd:c3e5:...]
  client_tcp_assisted_addrs=[]

diagnostic attempt candidate summary:
  peer_tcp_v6=2
  peer_tcp_v4=0
  peer_tcp_v4_addrs=[]

diagnostic direct tcp4 skipped:
  reason=no_peer_tcp_v4_candidates

diagnostic tcp punching targets:
  assisted_addrs=[]
  candidate_exact=2
  targets=[
    104.28.225.47:28471
    104.28.225.47:28265
  ]

tcp tls converge failed:
  successes=0
```

代码证据：

```text
internal/punchdecision/decision.go

vResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(cm.TCPDirectAddrs)
vResp.PeerTCPDirectAddrs, dropped = filterTCPDirectIPv4Addrs(vResp.PeerTCPDirectAddrs)

cResp.PeerTCPDirectAddrs, invalid = filterValidHostPorts(vm.TCPDirectAddrs)
cResp.PeerTCPDirectAddrs, dropped = filterTCPDirectIPv4Addrs(cResp.PeerTCPDirectAddrs)

filterTCPDirectIPv4Addrs:
  if ip.IsLoopback() || ip.IsUnspecified() || ip.IsLinkLocalUnicast() ||
     ip.IsMulticast() || ip.IsPrivate() {
    dropped = append(dropped, addr)
    continue
  }
```

当前定位结论：

- 第一层 gather 分桶确实被临时绕过：private IPv4 已经从 `tcp_assisted` 移到 `tcp_direct`。
- 但第二层 decision 仍把 private IPv4 从 `PeerTCPDirectAddrs` 里过滤掉。
- 所以这次仍然没有真正验证 `direct_tcp4`；`peer_tcp_v4=0`，attempt 明确跳过 direct TCP4。
- 因为 private IPv4 被移出 assisted，又没进入 direct，TCP punching 只剩 public `candidate_exact`；在 WSL2 mirrored 环境下该路径继续不稳定，最终 TLS 没有成功连接。

新增确认点：

- TCP direct 候选有两道独立过滤：
  - `connectivity/tcp_candidates.go` 的 gather-time listen bucket 分类；
  - `internal/punchdecision/decision.go` 的 decision-time `filterTCPDirectIPv4Addrs`。
- 只绕过 gather 层不足以验证 direct_tcp4。
- 下一步若要验证 direct_tcp4，需要再做一个明确标注的临时实验：同时绕过 decision-time private IPv4 direct 过滤，让 `PeerTCPDirectAddrs` 真正携带 `192.168.4.5:<port>` 到 attempt 层。

状态：

- `gather 层 private-as-direct 是否生效`：confirmed。
- `decision 层是否二次过滤 private IPv4 direct`：confirmed。
- `direct_tcp4 是否真实可用`：本节结束时仍未验证；后续见 4.16.9。

#### 4.16.9 实验记录：同时绕过 gather + decision 后验证 direct_tcp4

目标：在 WSL2 mirrored 环境下验证 `direct_tcp4` 本身是否可用，并观察 direct TCP 是否也存在多候选 TLS wait-all 风险。

临时诊断条件：

- `connectivity/tcp_candidates.go`：临时允许 private IPv4 进入 `tcp_direct`。
- `internal/punchdecision/decision.go`：临时允许 private IPv4 继续留在 `PeerTCPDirectAddrs`。
- 这两个改动都是实验绕过，用于命中 direct_tcp4；不是推荐修复。
- 实验后已恢复这两个临时绕过，恢复后 `go test ./connectivity ./internal/punchdecision ./dataplane` 通过。

##### 样本 A：Windows -> Linux，8s election timeout

报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-directtcp4-decision-bypass-20260514-01.md
```

结果：

```text
task_id=GK7NA7YYZVYAOZ7NYCVQEH73GA
reason_code=OK
data_proto=tls
attempt_path=direct_tcp4
path_family=tcp4
hello=ok
ping=ok
```

关键日志：

```text
diagnostic tcp decision material:
  visitor_peer_tcp_direct=[
    ... 10.255.255.254:45977
    172.17.0.1:45977
    192.168.4.5:45977
  ]

diagnostic attempt candidate summary:
  peer_tcp_v4=3
  peer_tcp_v4_addrs=[
    10.255.255.254:45977
    172.17.0.1:45977
    192.168.4.5:45977
  ]

diagnostic direct tcp4 attempt start:
  candidates=[
    10.255.255.254:45977
    172.17.0.1:45977
    192.168.4.5:45977
  ]

diagnostic tcp direct ok:
  path=direct_tcp4
  conns=2
  elapsed_ms=400
```

TLS 证据：

```text
tcp tls handshake ok:
  remote=192.168.4.5:45977
  elapsed_ms=828

tcp tls handshake failed:
  remote=10.255.255.254:45977
  elapsed_ms=4601

tcp tls converge handshakes ready:
  candidates=2
  successes=1

tcp tls election leader signal ok:
  remote=192.168.4.5:45977
```

结论：

- `direct_tcp4` 本身可用；当前正常环境未命中 direct_tcp4 的原因是候选过滤，不是 direct TCP 网络层一定失败。
- direct_tcp4 成功路径实际选择的是 `192.168.4.5:<port>`。
- 同一次 direct_tcp4 attempt 里也可能产生慢失败候选，例如 `10.255.255.254:<port>`，并且 TLS converge 会等它失败后才进入 election。

##### 样本 B：Linux -> Windows，8s election timeout

报告：

```text
/tmp/miopunch-link-lab/run/reports/linux-to-windows-directtcp4-decision-bypass-20260514-01.md
```

结果：

```text
task_id=V6DRKG42MQ67YM437ZBXRZSPHQ
reason_code=OK
data_proto=tls
attempt_path=direct_tcp4
path_family=tcp4
hello=ok
ping=ok
```

关键日志：

```text
diagnostic attempt candidate summary:
  peer_tcp_v4=2
  peer_tcp_v4_addrs=[
    172.17.128.1:51779
    192.168.4.5:51779
  ]

diagnostic direct tcp4 attempt start:
  candidates=[
    172.17.128.1:51779
    192.168.4.5:51779
  ]

diagnostic tcp direct ok:
  path=direct_tcp4
  conns=2
  elapsed_ms=211
```

TLS 证据：

```text
active side:
  tcp tls handshake ok:
    remote=192.168.4.5:51779
    elapsed_ms=7
  tcp tls converge handshakes ready:
    candidates=2
    successes=2

passive side:
  tcp tls handshake ok:
    remote=192.168.4.5:46363
    elapsed_ms=1
  tcp tls handshake failed:
    remote=10.255.255.254:46363
    elapsed_ms=4595
  tcp tls converge handshakes ready:
    candidates=3
    successes=2
```

结论：

- 反方向也能命中并完成 direct_tcp4。
- direct_tcp4 在两端都可能带入多条 conn；其中一条可用 conn 很快完成 TLS，另一条 interface/private 候选可能晚几秒失败。
- 这强化了“TLS wait-all 风险不是 punching_tcp4 专属”的判断。

##### 样本 C：Windows -> Linux，2s election timeout 压力样本

目的：验证 direct_tcp4 在短 election timeout 下是否稳定复现之前的 follower timeout。

临时诊断条件：

- `dataplane/tls_stream.go`：把 `tlsElectionTimeout` 从 8s 临时降到 2s。
- 其他 direct_tcp4 绕过保持不变。
- 实验后已恢复为 8s。

前置噪声样本：

```text
task_id=55PWFT7ZJ43PHHMVRKQD5M4AHY
stage=PunchAttempt
reason_code=UNAVAILABLE
fact=mqtt broker skipped: broker.emqx.io:1883: future canceled
fact=dial peer: nil NatHoleResp
```

这个样本没有进入链路层，只记录为 MQTT/broker 噪声，不用于 TCP 结论。

有效样本报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\windows-to-linux-directtcp4-election2s-20260514-02.md
```

结果：

```text
task_id=4O3DHWUDRVCXBDXGNI4FZDZOFQ
reason_code=OK
data_proto=tls
attempt_path=direct_tcp4
path_family=tcp4
hello=ok
ping=ok
```

关键日志：

```text
diagnostic direct tcp4 attempt start:
  candidates=[
    10.255.255.254:47394
    172.17.0.1:47394
    192.168.4.5:47394
  ]

diagnostic tcp direct ok:
  path=direct_tcp4
  conns=2
  elapsed_ms=400

tcp tls handshake ok:
  remote=192.168.4.5:47394
  elapsed_ms=2032

tcp tls handshake failed:
  remote=10.255.255.254:47394
  elapsed_ms=4601

tcp tls election deadlines:
  election_remaining_ms=2000

tcp tls election leader signal ok:
  remote=192.168.4.5:47394
```

结论：

- 2s election timeout 在这次 direct_tcp4 样本里没有复现失败。
- 但核心风险仍然成立：TLS 成功连接约 2.0s 已经出现，leader 仍等到慢失败候选约 4.6s 结束后才进入 election。
- 因此这里能确认的是 `direct_tcp4` 也存在 wait-all latency 风险；不能确认 direct_tcp4 在 2s election 下必然失败。
- 之前 auto / punching_tcp4 的 2s 失败仍是独立 confirmed 样本，不应被这次 direct_tcp4 成功抵消。

当前定位结论：

- 正常 production 语义下，WSL2 mirrored 的 private IPv4 不会进入 `direct_tcp4`：
  - gather 层把 private IPv4 归入 assisted；
  - decision 层也会丢 private IPv4 direct。
- 若临时放开两层过滤，`direct_tcp4` 能在 Windows -> Linux 和 Linux -> Windows 双向成功。
- `direct_tcp4`、`punching_tcp4` 都可能产生“可用 conn 很快成功 + 其他候选慢失败”的组合。
- 当前 TCP dataplane 的主要调度风险更集中在 TLS converge/election：它会先等待所有 TLS handshake 完成或失败，再进入 winner election。
- 后续修复方向不应只是把 timeout 调大；更应该围绕“首个/足够成功后进入 bounded settle，再选 winner，并取消剩余慢候选”来设计。

#### 4.16.10 实验记录：session reuse 四格矩阵与被动侧 topology 缺口复验

目标：验证 `ping -t` / `ping -u` 是否会被已有 session 污染，并确认这个问题是否双向稳定存在。每组实验前都重启双方 daemon，清空 in-memory session，再跑：

```text
seed fresh session -> 反协议 ping
```

代码事实：

```text
internal/task/poc_dial.go

reuseKey := dataplane.SessionKey{
  RemotePeerID: peerID,
  Protocol:     dataplane.Protocol(cfg.DataProto),
  SecurityID:   sid,
}
sess, ok := m.sessions.Find(reuseKey)
if !ok {
  reuseKey.Protocol = dataplane.ProtocolTLS
  sess, ok = m.sessions.Find(reuseKey)
}
```

```text
dataplane/session.go

SessionManager.Find 只有在 key.PathFamily 非空且不是 unknown 时，
才会按 path_family 过滤。
```

当前 `dialPeerStream` 的 reuse key 没有把本次 `-t` / `-u` 转成 `PathFamilyTCP4` / `PathFamilyUDP4` 约束，因此已有 session 可以跨用户指定的网络类型复用。

##### 矩阵结果

| 方向 | seed | 后续命令 | 后续结果 |
| --- | --- | --- | --- |
| Windows -> Linux | `ping -u` fresh UDP | `ping -t` | 复用 UDP/QUIC |
| Windows -> Linux | `ping -t` fresh TCP | `ping -u` | 复用 TCP/TLS |
| Linux -> Windows | `ping -u` fresh UDP | `ping -t` | 复用 UDP/QUIC |
| Linux -> Windows | `ping -t` fresh TCP | `ping -u` | 复用 TCP/TLS |

##### Windows -> Linux：UDP 后 `-t`

seed 报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\matrix-w2l-seed-udp-then-tcp-seed-20260514-01.md

task_id=WXU3YKJOBEKX6VN3W3E5FR5SWM
data_proto=quic
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

后续 `ping -t` 报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\matrix-w2l-seed-udp-then-tcp-reuse-20260514-01.md

task_id=HERFADFJGJEDUCWETJECK6TW2M
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
ping=ok
```

结论：用户请求 TCP-only，但没有发生 fresh TCP attempt。

##### Windows -> Linux：TCP 后 `-u`

seed 报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\matrix-w2l-seed-tcp-then-udp-seed-20260514-01.md

task_id=SXCM5MKAOHIZUHJQ7B4G777NHM
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

后续 `ping -u` 报告：

```text
C:\Users\stati\AppData\Local\Temp\miopunch-link-lab\run\reports\matrix-w2l-seed-tcp-then-udp-reuse-20260514-01.md

task_id=FCG65WYRQ455BFHD42I4R637NQ
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
ping=ok
```

结论：用户请求 UDP-only，但没有发生 fresh UDP attempt。

##### Linux -> Windows：UDP 后 `-t`

seed 报告：

```text
/tmp/miopunch-link-lab/run/reports/matrix-l2w-seed-udp-then-tcp-seed-20260514-01.md

task_id=2CXYID56HSPQDKJBUTGK7U3ZAA
data_proto=quic
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
```

后续 `ping -t` 报告：

```text
/tmp/miopunch-link-lab/run/reports/matrix-l2w-seed-udp-then-tcp-reuse-20260514-01.md

task_id=XDWPJ3K3TOA5NUUL7TF3O2EF2M
data_proto=quic
session_reused=true
path_family=udp4
hello=ok
ping=ok
```

结论：Linux 侧同样会让 TCP-only 命令复用 UDP session。

##### Linux -> Windows：TCP 后 `-u`

seed 报告：

```text
/tmp/miopunch-link-lab/run/reports/matrix-l2w-seed-tcp-then-udp-seed-20260514-01.md

task_id=CF6YGW7I6BRATJT53AOH6VSQ7E
data_proto=tls
attempt_path=punching_tcp4
path_family=tcp4
hello=ok
ping=ok
```

后续 `ping -u` 报告：

```text
/tmp/miopunch-link-lab/run/reports/matrix-l2w-seed-tcp-then-udp-reuse-20260514-01.md

task_id=PXNYTZ647JD4IPYKAL6DK4XWQQ
data_proto=tls
session_reused=true
path_family=tcp4
hello=ok
ping=ok
```

结论：Linux 侧同样会让 UDP-only 命令复用 TCP session。

##### 同场 topology 证据：被动侧仍无 runtime evidence

最后一组 Linux -> Windows TCP seed 后，Linux 主动侧 topology：

```text
neighbors.active=[
  {
    peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
    data_proto=tls
    path_family=tcp4
    healthy=true
  }
]

attempts=[
  { attempt_path=punching_tcp4, outcome=ok },
  { attempt_path=session_reuse, outcome=ok }
]

payloads=[
  ping=ok
  ping=ok
]
```

同一时间 Windows 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

但 Windows 被动侧日志存在真实 accept 证据：

```text
pocacceptor incoming attempt:
  sid=2cf829bf2fd85bc2163d2e036162cfc3

pocacceptor connectivity attempt ready:
  path=punching_tcp4
  tcp_conns=2
```

结论：

- session reuse 污染已经四格 confirmed。
- `session_reused=true` 的报告不能用于判断当前 TCP/UDP 打洞能力；它只能证明已有 session 还能打开逻辑 stream。
- `-t` / `-u` 当前只是影响 fresh attempt 的目标网络；在 reuse 阶段没有强制 path family。
- 被动侧 topology gap 再次复现，且 TCP/UDP 都受影响：被动 acceptor 有日志证据，但没有写入 task manager topology 的 active/attempt/payload runtime evidence。

#### 4.16.11 实验记录：TCP session idle close 与 keepalive 语义

目标：在 4.16.10 最后一组 Linux -> Windows TCP session 成功后，不再发送业务流量，等待超过 2 分钟，确认 session 是如何关闭的。

代码事实：

```text
dataplane/session.go
  DefaultSessionIdleTimeout = 2 * time.Minute

dataplane/session_transport.go
  yamuxPeerSession / quicPeerSession 都有 idle closer：
    if s.idleFor() >= timeout {
      _ = s.Close(CloseReasonIdleTimeout)
    }
```

`lastActivity` 来自 miopunch 逻辑 stream activity，不是 QUIC/TCP 底层 keepalive。底层 transport keepalive 即使存在，也不会自动刷新 `sessionBase.lastActivity`。

实验前状态：

```text
Linux -> Windows TCP seed:
  task_id=CF6YGW7I6BRATJT53AOH6VSQ7E
  data_proto=tls
  attempt_path=punching_tcp4
  path_family=tcp4
  hello=ok
  ping=ok

Linux -> Windows 后续 UDP-only 命令：
  task_id=PXNYTZ647JD4IPYKAL6DK4XWQQ
  data_proto=tls
  session_reused=true
  path_family=tcp4
  hello=ok
  ping=ok
```

等待后 Linux 主动侧 topology：

```text
neighbors.active=[]

neighbors.unhealthy=[
  {
    peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
    data_proto=tls
    path_family=tcp4
    close_reason=idle_timeout
  }
]

neighbors.reconnect_attempts=[
  {
    peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
    reason_code=idle_timeout
    retry_budget=1
    stop_condition=session_closed
  }
]
```

同一时间 Windows 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

Windows 被动侧日志：

```text
pocacceptor accept stream failed:
  proto=tls
  sid=2cf829bf2fd85bc2163d2e036162cfc3
  path_family=tcp4
  err=EOF
```

当前定位结论：

- 主动侧已经明确记录 `close_reason=idle_timeout`，这是 miopunch 应用层 session idle timeout。
- 这不是 TCP 网络层“自然断开”的直接证据，也不是底层 keepalive 失败的直接证据。
- 被动侧仍没有 topology runtime evidence：它既没有 active，也没有 unhealthy；只能从 acceptor 日志看到对端关闭后的 EOF。
- 当前实现语义是：一次 ping 成功后 session 保留约 2 分钟；如果没有新的 miopunch 逻辑 stream activity，就会 idle close。
- 因此“桌面端打通后等一会儿没消息/断了”目前可以被解释为应用层 idle 策略，而不是长期 keepalive 机制已经生效。

#### 4.16.12 实验记录：被动侧 topology 缺口的代码级定位

目标：把 4.9、4.11、4.16.10 里反复出现的“发起侧成功、被动侧 topology 无变化”定位到具体代码边界，而不是只停留在现象层。

代码事实 1：主动拨号路径会进入 `task.Manager.sessions` 和 topology runtime。

```text
internal/task/poc_dial.go
  dialPeerStream:
    dataplane session 建立成功后：
      m.sessions.Put(sess)

    OpenStream 成功后：
      m.recordTopologyAttempt(...)

  ping task 后续还会：
      m.recordTopologyPayload(...)
```

这解释了为什么主动侧 topology 会有：

```text
neighbors.active=[...]
attempts=[...]
payloads=[...]
```

代码事实 2：被动 acceptor 没有使用 `task.Manager.sessions`。

```text
internal/pocacceptor/acceptor.go
  serveOnce:
    reg := newPeerSessionRegistry()

  UDP:
    ListenSessionsWithQUICTransport(...)
    startAcceptLoop(...)
    servePeerSession(..., reg)

  TCP:
    ServeTLSSession(...)
    servePeerSession(..., reg)
```

`peerSessionRegistry` 只提供 `Add/Remove/ClosePeer/Replace`，用途是按 peer 管理和关闭 acceptor 内部 session；它没有接入 `task.Manager.sessions`，也没有写 topology runtime evidence。

代码事实 3：被动 session 在 hello 前没有可靠 remote peer key。

```text
internal/pocacceptor/acceptor.go
  dpCfg := dataplane.Config{
    Proto: ...
    SecurityID: sid
    PathFamily: ...
    // 没有 RemotePeerID
  }

  servePeerSession:
    key := sess.Key()

  serveAcceptedShellStream:
    handleHello(...)
    bindPeer(helloCtl.PeerID)
```

也就是说，被动侧 `PeerSession` 创建时 `SessionKey.RemotePeerID` 是空的；remote peer 只有在后续 logical stream 的 hello metadata 里才知道。当前代码只把 hello 得到的 peer id 绑定到 `peerSessionRegistry`，没有更新 dataplane session key，也没有把它注册进 topology 可见的 session manager。

这点很关键：即使以后想把被动 session 直接放入 `SessionManager`，也不能简单在 `ServeTLSSession` / `ListenSessions.Accept` 后立刻 `Put`；因为这时 key 还缺 remote peer id，`topology.go` 又会过滤掉 `RemotePeerID == ""` 的 session。

现场复验：2026-05-14 22:15 fresh Linux -> Windows UDP。

命令：

```bash
/tmp/miopunch-link-lab/bin/miopunch-linux \
  --localapi unix:/tmp/miopunch-link-lab/run/linux/localapi.sock \
  --format json \
  --report /tmp/miopunch-link-lab/run/reports/passive-topology-codepath-l2w-udp-20260514-01.md \
  ping -u FI5NWWE3G67A3MZ4FH5NTDWYWI
```

结果：

```text
task_id=3EWRPWVPA4C2SM5EROR325PRBM
data_proto=quic
attempt_path=punching_ipv4
path_family=udp4
hello=ok
ping=ok
session_reused 未出现
```

Linux 主动侧 topology：

```text
neighbors.active=[
  { peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI, data_proto=quic, path_family=udp4, healthy=true }
]

attempts=[
  { peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI, attempt_path=punching_ipv4, data_proto=quic, path_family=udp4, outcome=ok }
]

payloads=[
  { peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI, evidence=ping=ok }
]
```

Windows 被动侧 topology：

```text
neighbors.active=[]
attempts=[]
payloads=[]
```

Windows 被动侧日志：

```text
2026-05-14 22:15:18
pocacceptor incoming attempt:
  peer_id=FI5NWWE3G67A3MZ4FH5NTDWYWI
  sid=2cf829bf2fd85bc2163d2e036162cfc3
  dial_id=1778768112923565352e31a0a9

2026-05-14 22:15:19
pocacceptor connectivity attempt ready:
  path=punching_ipv4
  tcp_conns=0
  protocol=quic
  selected_view=cn
  selected_reason=stun_rtt

pocacceptor connectivity attempt delegated:
  path=punching_ipv4
```

当前定位结论：

- 这不是 UDP 没通；发起侧已经完成 punching、QUIC、hello、ping。
- 被动侧日志证明 acceptor 收到了 signaling attempt，并完成了 connectivity attempt。
- 被动侧 topology 空白是结构性观测缺口：passive accept session 没有进入 `task.Manager.sessions`，也没有调用 `recordTopologyAttempt` / `recordTopologyPayload`。
- 更深一层的约束是：被动 session 创建时没有 `RemotePeerID`，只能在 hello 后通过 metadata 绑定 peer，因此未来修复不能只做一个无条件 `sessions.Put(sess)`。

后续验证/修复前置判断：

- 如果只补 topology evidence，应该在 `serveAcceptedShellStream` 的 hello 成功之后上报 passive session/stream/payload，因为此时 remote peer id 才可靠。
- 如果要让被动 session 也可被后续主动命令复用，需要设计“hello 后绑定 remote peer id 并注册 session”的机制；这比单纯补 UI evidence 风险更高，应该单独验证复用方向、权限、关闭原因和 idle 语义。
- 当前最小正确目标仍是先补观测面：让被动侧 topology 能表达“我收到并服务了一个 passive stream”，不要让用户把空 topology 误判成链路没通。

#### 4.16.13 实验记录：TCP TLS wait-all 的代码级定位

目标：把前面 TCP 日志里反复出现的“已有 TLS 成功连接，但仍等待慢失败 candidate”定位到具体控制流。

代码事实：

```text
dataplane/tls_stream.go
  convergePinnedTLS:
    handshakeCtx := context.WithTimeout(ctx, 6*time.Second)
    resultCh := make(chan tlsCandidate, len(candidates))

    for each candidate:
      go HandshakeContext(handshakeCtx)
      if handshake ok:
        resultCh <- tlsCandidate{...}
      if handshake failed:
        close candidate
        return

    go func() {
      wg.Wait()
      close(resultCh)
    }()

    for res := range resultCh {
      successes = append(successes, res)
    }

    // resultCh close 之后才进入 election
    convergePinnedTLSElection(...)
```

关键点：

- 成功的 candidate 会进入 `resultCh`。
- 失败的 candidate 不进入 `resultCh`。
- 但 `resultCh` 只有在所有 handshake goroutine 都退出后才关闭。
- 因此只要存在一个慢失败 candidate，主流程就不会进入 `convergePinnedTLSElection`，即使另一个 candidate 已经 TLS handshake 成功。

现场日志与代码对齐样本 1：Linux -> Windows TCP。

```text
task_id=CF6YGW7I6BRATJT53AOH6VSQ7E

22:06:27.380
tcp tls converge start:
  role=visitor
  candidates=2

22:06:27.416
tcp tls handshake ok:
  remote=192.168.4.5:57132
  elapsed_ms=36

22:06:33.381
tcp tls handshake failed:
  remote=104.28.225.47:32781
  elapsed_ms=6001
  err=context deadline exceeded

22:06:33.381
tcp tls converge handshakes ready:
  candidates=2
  successes=1

22:06:33.381
tcp tls election leader signal start
```

这与代码完全一致：36ms 已经有成功连接，但 `converge handshakes ready` 必须等 6001ms 的慢失败候选退出后才打印，随后才进入 election。

现场日志与代码对齐样本 2：Windows -> Linux TCP。

```text
task_id=SXCM5MKAOHIZUHJQ7B4G777NHM

22:04:57.449
tcp tls converge start:
  role=visitor
  candidates=2

22:04:57.449
tcp tls handshake failed:
  remote=104.28.225.48:62396
  elapsed_ms=0

22:05:00.015
tcp tls handshake ok:
  remote=192.168.4.5:47324
  elapsed_ms=2566

22:05:00.015
tcp tls converge handshakes ready:
  candidates=2
  successes=1
```

这个样本里慢的是成功候选，不是失败候选；但它仍说明 election 只有在当前所有 handshake goroutine 都已经结束后才开始。

当前定位结论：

- TCP wait-all 不是日志错觉，而是 `convergePinnedTLS` 的显式控制流。
- 该逻辑把“候选收敛”和“winner election”串行化：先等所有 handshake 完成/失败，再选 winner。
- 在 WSL2 mirrored 场景里，经常出现一个可用的 `192.168.4.5:<port>` candidate 和一个公网/STUN/接口 candidate 并存；后者可能慢失败，从而拖慢甚至拖死 dataplane 建立。
- 把 `tlsElectionTimeout` 从 2 秒改到 8 秒只能掩盖部分现象；它没有改变 handshake wait-all 的根因。
- 更合理的后续修复方向仍是：首个合格成功后进入 bounded settle window，或者在满足最小成功条件后取消/关闭慢候选，再做 election。

仍需进一步验证：

- follower election 内部也有类似“range outCh 直到所有 reader 结束”的结构；当前成功样本中 winner 读到后会关闭其他成功连接，通常能让其他 reader 退出，但还需要单独构造多成功候选样本确认不会被第二阶段拖住。
- 多成功候选时 leader 选择第一个成功的顺序是否稳定、是否会偏向较差路径，目前还没有专门实验。

## 5. 当前问题拆分

### A. TCP 真实失败问题

当前已经确认：TCP punching 本身能建立，Windows -> Linux 的关键失败点在 TCP TLS election 调度，不是 TCP 网络层彻底不通。

下一步要点：

- 所有 TCP 实验必须明确区分：
  - fresh TCP attempt
  - reused session
  - punched path established
  - dataplane handshake established
  - hello/governance passed
  - ping payload passed
- TCP TLS election 的候选修复应优先验证“首个成功后快速进入 election / bounded settle window”，而不是只把 timeout 改大。

### B. UDP 双向状态问题

当前最强解释：被动侧 acceptor 没有向 topology runtime 写证据。

下一步要点：

- 需要决定 topology 应该如何表达被动 accept：
  - 是否记录 `attempt_path=passive_accept_udp4/tcp4`
  - 是否记录 payload/stream accepted
  - 是否在桌面 topology 中区分 active dial 与 passive accept

推荐修复结构：

```text
pocacceptor passive accept
  -> PassiveSessionObserver
  -> task.Manager topology runtime
  -> desktop/topology snapshot
```

具体方向：

- 不先改 UDP punching 算法；当前证据更支持“链路已通但被动侧没写 runtime evidence”。
- 给 `internal/pocacceptor` 增加一个被动连接观察接口，例如 `PassiveSessionObserver`。
- acceptor 在关键时机上报：
  - session accepted；
  - stream accepted；
  - hello/ping/payload observed；
  - session closed，带 close reason。
- `task.Manager` 实现该 observer，把被动 evidence 写入现有 topology runtime。
- topology 里显式区分主动和被动：
  - `attempt_way=passive_accept`
  - `attempt_path=passive_accept_udp4` 或 `passive_accept_tcp4`
  - `outcome=ok|closed|fail`
  - payload evidence 可记录 `passive_stream_accepted`、`ping_observed` 等。

当前不推荐的修法：

- 不建议第一步就把被动 acceptor session 直接塞进主动侧 `SessionManager` 复用。
- 原因：那会把“UI 看到被动连接存在”和“主动命令可以复用这条 session”混成一个语义，容易引入复用方向、权限、stream kind、生命周期上的新问题。
- 更稳妥的第一步是补观测面；确认 UI/topology 能反映被动连入后，再设计是否统一 session registry。

验收标准：

- Windows 主动发起 UDP，Linux 被动侧 topology 能看到 passive accept evidence。
- Linux 主动发起 UDP，Windows 被动侧 topology 能看到 passive accept evidence。
- 被动侧 topology 不能再因为没有主动 task 而显示完全无变化。
- 日志和 topology 能区分“链路没通”和“链路通了但没有主动 payload task”。

### C. 长期保活问题

当前实现不是“连接成功后长期保活”。

下一步要点：

- 需要定义产品语义：
  - session idle 2 分钟关闭是否预期？
  - selected neighbors 是否应该后台周期性 maintain？
  - keepalive 是应用层 ping stream，还是 transport 层 native keepalive？
  - UI 应显示“可重新拨号的邻居”还是“当前 live session”？

已识别原因：

- `DefaultSessionIdleTimeout=2m` 是 miopunch 应用层 session idle timeout。
- idle closer 看的是 `PeerSession` 的逻辑 stream activity。
- QUIC transport keepalive 不会更新 miopunch `lastActivity`，因此不能阻止 miopunch session 被 `idle_timeout` 关闭。
- `maintain-neighbors` 当前是一次性 task，不是后台持续 keepalive loop。

推荐修复结构：

```text
selected neighbors
  -> background neighbor maintainer
  -> application-level heartbeat/ping stream
  -> updates PeerSession lastActivity
  -> on failure: mark unhealthy and optional redial with backoff
```

具体方向：

- 保留 session idle timeout，避免临时 session 和失败 session 永久占资源。
- 对 selected neighbors 增加后台 maintainer，而不是依赖 QUIC 底层 keepalive。
- maintainer 周期性发起轻量应用层逻辑流量，例如 hello/ping/heartbeat stream。
- 这个流量必须走 `PeerSession.OpenStream` / `AcceptStream`，从而更新 `lastActivity`。
- 如果 session 已断，maintainer 可以按 backoff/jitter 触发 redial。
- UI/topology 应区分：
  - `selected`：用户或策略希望保持连接；
  - `active`：当前有 live session；
  - `last_seen`：最近一次成功应用层 heartbeat/payload；
  - `last_error`：最近一次维护失败原因。

当前不推荐的修法：

- 不建议只把 `DefaultSessionIdleTimeout` 调大。
- 不建议直接关闭 idle timeout。
- 不建议只打开或调大 QUIC keepalive，因为这不会更新 miopunch 应用层 activity。

验收标准：

- selected neighbor 在无用户操作时，超过 2 分钟仍能保持 active，或在断开后由 maintainer 明确重拨并记录结果。
- 未 selected 的临时 session 仍会按 idle timeout 关闭。
- UI/topology 能解释“当前 active”“最近见过”“正在维护失败重试”三种不同状态。
- session close 日志能明确区分 `idle_timeout`、transport fatal、maintainer redial 失败。

### D. 治理/成员状态问题

当前最像真正的 bug：非 admin peer 能生成会被对端拒绝的 approve decl。

下一步要点：

- `invite` / `approve` 生成前应检查本机是否 owner/admin。
- 错误信息应提前且明确，不要等到对端 hello 后才表现为连接失败。
- 需要清理或隔离历史 state，避免 `J2AJ...` admin 与当前 `BD5...` Windows peer 混用。

### E. 调试闭环问题

当前实验可跑，但 Windows 独立 daemon 控制还不干净。

下一步要点：

- 短期：继续驱动现有 Windows daemon，并明确记录污染风险。
- 中期：修复 Windows `--localapi npipe:...` 让 WSL2 能启动/控制隔离 Windows daemon。
- 长期：做一个 lab/debug script，一次性拉起双方、生成 invite、approve/join、跑 UDP/TCP/fresh/reuse/idle 测试并收集报告。

## 6. 排查逻辑图

```text
用户看到“连不上/断开”
        |
        v
先拆阶段，而不是直接归因到网络
        |
        +--> 是否发起了 fresh attempt？
        |       |
        |       +--> 否：可能是 session_reused，不能证明 TCP/UDP 当前可用性
        |       +--> 是：继续看 attempt_path / path_family
        |
        +--> 是否已有 punched path？
        |       |
        |       +--> 无：看 connectivity.Attempt / STUN / punching logs
        |       +--> 有：继续看 dataplane handshake
        |
        +--> dataplane 是否建立？
        |       |
        |       +--> TCP: tls/yamux
        |       +--> UDP: quic/kcp
        |
        +--> hello/governance 是否通过？
        |       |
        |       +--> FORBIDDEN / issuer not admin：治理问题，不是打洞失败
        |
        +--> payload 是否通过？
        |       |
        |       +--> ping=ok：链路可用
        |
        +--> 等待后是否 idle close？
                |
                +--> idle_timeout：当前应用层 session idle 策略触发
```

## 7. 后续优先级建议

1. 先修 `-t` / `-u` 的 session reuse 语义，让后续 TCP/UDP 验证样本可信。
2. 修 TCP TLS converge / election 调度，避免慢失败 candidate 拖住已成功 TCP path。
3. 再处理 auto 下 TCP dataplane 失败后的 UDP fallback，避免 TCP-first 把可用 UDP 路径挡住。
4. 补被动侧 topology evidence，让桌面能看到“对端主动连进来”的事实。
5. 明确定义 keepalive 产品语义，并决定 selected neighbors 是否需要后台应用层 maintainer。
6. 修治理前置校验和 Windows isolated daemon；这两项会影响实验闭环，但不作为本轮链路层主线第一批。

## 8. 暂不做的事

- 暂不把所有现象归为一个根因。
- 暂不直接改打洞算法。
- 暂不把 desktop UI 当作唯一真相来源。
- 暂不忽略治理状态污染；它目前已经实质影响 UDP/TCP 判断。

## 9. 修复行动记录

本节从 2026-05-14 开始记录实际修复动作。原则是小批次推进：每批只修一组互相关联的问题，同时更新本文档、做 focused validation，并 commit 该批变更。全量 gate 暂不作为本轮每批修复的阻塞条件；本轮优先以 focused test、编译和 Windows/WSL2 现场实际运行为准，等链路层问题排查完后再统一跑完整验证。

### 9.1 Batch 1：收紧 `-t` / `-u` 的 session reuse 语义

修复目标：

- `ping -t` / `tcp_only` 不应复用已有 UDP/QUIC 或 UDP/KCP session。
- `ping -u` / `udp_only` 不应复用已有 TCP/TLS session。
- `auto` 保持原有产品语义：仍可先按配置的 UDP dataplane protocol 找可复用 session，再 fallback 到 TLS session。
- 报告中如果仍出现 `session_reused=true`，它应表示“复用的 session 符合本次网络策略约束”。

代码方向：

- 在 `internal/task/poc_dial.go` 的 reuse 阶段，把当前任务的 `P2PNetwork` 转成可复用的 `PathFamily` 约束。
- `tcp_only` 只查 `tcp6` / `tcp4` + `tls`。
- `udp_only` 只查 `udp6` / `udp4` + 当前 `DataProto`。
- `auto` 使用 `PathFamilyUnknown`，保持既有“不按 path family 过滤”的复用行为。

验证目标：

- focused unit test 覆盖：
  - UDP session 后 `tcp_only` 不命中；
  - TCP session 后 `udp_only` 不命中；
  - 同时有 UDP/TCP session 时，`tcp_only` 选 TCP、`udp_only` 选 UDP；
  - `auto` 仍可 fallback 到 TLS session。
- focused validation 先跑 `go test ./internal/task` 和一次编译；完整 lab gate 留到后续统一验证。

当前状态：

- 已实现 `findReusableSession`，复用查询会按 `P2PNetwork` 选择 path family 约束。
- 已新增 `TestFindReusableSessionHonorsP2PNetwork`，覆盖上述 reuse 污染矩阵。
- 已通过：
  - `go test ./internal/task`
  - `go build ./cmd/miopunch`
- 已用 `d5fe895` 的临时 worktree 构建 Linux/Windows 二进制，并在真实 Windows/WSL2 mirrored 环境验证：
  - Windows -> Linux：先 `ping -u` 成功得到 `punching_ipv4` / `udp4` / `quic`，随后同一 daemon 执行 `ping -t`，未出现 `session_reused=true`，而是 fresh `punching_tcp4` / `tcp4` / `tls` 并 `hello=ok` / `ping=ok`。
  - Linux -> Windows：先 `ping -t` 成功得到 `punching_tcp4` / `tcp4` / `tls`，随后同一 daemon 执行 `ping -u`，未出现 `session_reused=true`，而是 fresh `punching_ipv4` / `udp4` / `quic` 并 `hello=ok` / `ping=ok`。
  - 额外观察：Windows -> Linux fresh TCP 在一次重启后仍复现过旧的 `dial peer: session shutdown`，说明 TCP TLS election 问题仍需 Batch 2 单独修复。
- 未跑完整 gate；本批按本轮策略留到链路层修复完成后统一验证。

### 9.2 Batch 2：修正 TCP TLS converge / election 的时序

修复目标：

- 不再等待所有 TCP candidate 的 TLS handshake 全部结束才进入 election。
- 允许首个成功 TLS conn 先触发一个短 settle window，然后尽快发 winner signal。
- 对仍在握手中的慢失败 candidate，settle 到期后取消并关闭，避免拖过 follower election 窗口。
- 保持 `punching_tcp4` / `direct_tcp4` 的诊断可读性，但不再依赖调大 election timeout 来掩盖时序问题。

代码方向：

- 在 `dataplane/tls_stream.go` 引入 handshake timeout 与短 settle window。
- 收集到首个成功 TLS conn 后，取消剩余慢握手的继续等待。
- `transport.tls_converge` 事件补充 `first_success_elapsed_ms` 与 `settle_window_ms`，便于后续排查。
- 新增单测覆盖“好 conn + 慢 conn”场景，确保不会再被慢候选拖死。

验证目标：

- `go test ./dataplane` 通过。
- 真实 Windows/WSL2 mirrored 环境里，Windows -> Linux fresh TCP 从 `dial peer: session shutdown` 变成 `attempt_path=punching_tcp4` / `path_family=tcp4` / `hello=ok` / `ping=ok`。
- 同一 daemon 后续 `ping -u` 不再复用 TCP session，而是 fresh UDP path。

当前状态：

- `go test ./dataplane` 已通过。
- 真实环境已验证：
  - Windows -> Linux fresh TCP 成功；
  - Windows 同一 daemon 后续 `ping -u` 成功且未复用 TCP。
- 已提交：`e76698e fix tcp tls convergence timing`。
- 未跑完整 gate；本批按本轮策略留到链路层修复完成后统一验证。

### 9.3 Batch 3：auto fallback 与被动侧 topology evidence

修复目标：

- 被动 acceptor 接收到对端主动连入时，也要把 session、attempt、payload evidence 写进 `task.Manager` 的 runtime topology。
- `auto` 模式下，如果 TCP punching 已经拿到 TCP conn，但 TCP/TLS dataplane handshake 失败，不能直接让任务失败；应退回 UDP 路径。
- UDP fallback 不能只在主动侧本地重跑 `connectivity.Attempt`，必须重新走一轮 MQTT candidate exchange，让被动侧也进入 UDP-only attempt。

代码方向：

- 新增 `internal/task/passive_topology.go`，提供 `RegisterPassivePeerSession`、`ClosePassivePeerSession`、`RecordPassiveTopologyAttempt`、`RecordPassiveTopologyPayload`。
- `internal/pocacceptor/acceptor.go` 增加 `RuntimeEvidenceSink`，在被动 session 通过 hello 后登记 active session，在 ping payload 通过后记录 `ping=ok`。
- `cmd/miopunch/up*.go` 把当前 `Manager` 传给 acceptor，统一桌面/topology 的 runtime evidence 来源。
- `internal/task/poc_dial.go` 的 auto fallback 改成：
  - 先按 auto 正常 exchange / attempt；
  - 如果 TCP dataplane 失败且本次是 `auto`，记录 `auto_fallback=udp_after_tcp_dataplane_error`；
  - 重新发起一轮 `P2PNetwork=udp_only` 的 MQTT exchange；
  - 用第二轮 response 再做 UDP punching 与 QUIC dataplane。

验证记录：

- focused validation 已通过：
  - `go test ./connectivity ./dataplane`
  - `go test ./internal/task ./internal/pocacceptor ./cmd/miopunch`
  - `go build ./cmd/miopunch`
- 真实 Windows/WSL2 mirrored 环境使用本地 MQTT broker `192.168.4.5:1883` 做稳定 signaling；这是验证基础设施替换，不改变被测链路层代码。
- 被动侧 topology evidence 已在真实环境验证：
  - Windows -> Linux `ping -u UANSTBEARWBWOEMGHHY4454E5I` 成功；
  - Linux topology 文件 `/tmp/miopunch-batch3/run/linux/topology-after-w2l-udp-06.json` 显示 active session：
    `peer_id=PCALYYSAMQFDPCBKRS4AOFVNIQ`、`data_proto=quic`、`path_family=udp4`、`healthy=true`；
  - 同一 topology 显示 attempt：`attempt_path=passive_accept_udp4`、`outcome=ok`；
  - payload 显示 `evidence=ping=ok`。
- auto fallback 的第一版真实 fault 验证失败，暴露了关键因果：
  - Windows 主动侧已记录 `auto_fallback=udp_after_tcp_dataplane_error`；
  - 但 UDP fallback 失败为 `wait detect message error: context deadline exceeded`；
  - 原因是只在主动侧本地重跑 UDP attempt，被动侧没有第二轮 UDP-only exchange。
- 修正为第二轮 MQTT exchange 后，真实 fault 验证通过：
  - fault 只用于验证：Linux 被动侧临时二进制在 TCP punching 成功后关闭 TCP dataplane candidates，UDP listener 保持正常；该临时代码未进入工作树最终 diff。
  - Windows -> Linux plain `ping UANSTBEARWBWOEMGHHY4454E5I` 输出：
    `auto_fallback=udp_after_tcp_dataplane_error`、
    `auto_fallback_tcp_error=tls handshake failed for all candidates`、
    `data_proto=quic`、
    `quic_cc=bbr`、
    `attempt_path=punching_ipv4`、
    `path_family=udp4`、
    `hello=ok`、
    `ping=ok`。
  - 主动侧报告：`/mnt/c/Users/stati/AppData/Local/Temp/miopunch-batch3/run/windows/windows-to-linux-auto-fallback-batch3-fault-02.json`。
  - 被动侧 topology：`/tmp/miopunch-batch3/run/linux/topology-after-w2l-auto-fallback-fault-02.json`，显示 `passive_accept_udp4` 与 `ping=ok`。
- 验证后已把两侧 daemon 恢复为非 fault 的当前二进制。

当前状态：

- auto fallback 和被动侧 topology evidence 均已在真实 Windows/WSL2 mirrored 环境验证。
- 未跑完整 gate；本批按本轮策略留到链路层修复完成后统一验证。

### 9.4 Batch 4：运行时真相与应用层 keepalive

本批开始处理剩余问题里的第一组：`maintain-neighbors` / topology active 状态分裂，以及应用层 keepalive / idle timeout。它们应放在同一条主线里看，因为 keepalive 的前提是 active 状态本身可信；如果 topology 继续把“transport 已建立但 hello 已失败”的 session 当作 active，后续 keepalive 只会把错误状态维持得更久。

#### 9.4.1 当前问题边界

已确认的问题：

- fresh dial 建立 dataplane session 后，当前主动侧会先 `m.sessions.Put(sess)`，随后才 `OpenStream`、hello、ping。
- 如果 hello / governance / capability handshake 失败，task 会失败，logical stream 会关闭，但刚放进 `Manager.sessions` 的 transport session 不一定立即关闭。
- `TopologySnapshot()` 和 desktop active neighbors 来自 `Manager.sessions.ListAllSummaries()`，因此会短时间显示“active”，即使本次应用层握手已经失败。
- `maintain-neighbors` 同时报告 child ping 成败与最终 topology active 数量，二者可能不一致。
- QUIC 底层 keepalive 不会更新 miopunch 应用层 `lastActivity`；无逻辑 stream activity 时，session 仍会被 `DefaultSessionIdleTimeout=2m` 收掉。

#### 9.4.2 修复原则

本批先定义 runtime truth：

- topology 的 active session 应表达“当前 transport session 仍 healthy，并且至少经过一次应用层握手 / payload 成功证明”。
- hello / capability handshake / ping 失败时，不能留下一个会误导 topology 的 healthy session。
- keepalive 必须是应用层行为，不能依赖 QUIC 底层 keepalive 来刷新 miopunch session activity。
- keepalive 失败应关闭对应 session，并写入 topology / recovery evidence，让 UI 和 task report 看到同一个事实。

#### 9.4.3 修复顺序

第一步：收紧 session 写入和失败清理。

- 主动 fresh dial 只有在 stream 打开并完成应用层 hello / payload 成功后，才应被 topology 视为可靠 active。
- 如果当前代码结构需要先把 session 放入 manager 以支持 stream 生命周期，也必须在 hello / ping 失败路径显式关闭并移除该 session。
- 被动侧 Batch 3 新增的 passive evidence 继续保留，但同样以 hello 成功后的 `bindPeer` 作为登记点，不提前记录未知 peer。
- `maintain-neighbors` 的最终 active 数量应避免把本轮已失败 peer 误报为 active。

第二步：实现应用层 keepalive / neighbor maintainer。

- 对 selected 或 active neighbors 周期性打开轻量 `ping` stream。
- 成功：记录 keepalive evidence，并通过逻辑 stream activity 刷新 session activity。
- 失败：关闭 session，记录 close reason / recovery evidence。
- 这个 maintainer 应有明确生命周期，随 daemon context 停止；不要启动无法停止的后台 goroutine。

#### 9.4.4 实现记录

- `internal/task/poc_dial.go` 调整 fresh dial lifecycle：
  - fresh dataplane session 不再在 `OpenStream` 前直接进入 `Manager.sessions`；
  - `dialResult` 带回底层 session；
  - task 只有在应用层操作成功后调用 `markDialedSessionLive`；
  - 失败路径调用 `closeDialedSession`，避免 topology 留下假 active。
- `internal/task/ping.go` / `sh_ls.go` / `sh_attach.go` 接入上述生命周期：
  - `ping` 只有 `hello=ok` + `ping=ok` 后登记 active；
  - hello 失败会记录 `outcome=fail` / `stop_condition=hello_failed` 的 topology attempt；
  - shell list / attach 只有远端应用层响应 OK 后登记 active。
- `internal/task/session_keepalive.go` 新增 daemon-level application keepalive：
  - `StartSessionKeepalive(ctx)` 由 `miopunch up` 启动，随 daemon context 停止；
  - 每 30 秒扫描 active sessions，只对 idle 超过 45 秒的本地主动 dial session 打开轻量 ping stream；
  - 成功记录 `keepalive=ok` payload，并通过 stream activity 刷新 session；
  - 失败关闭 session 并记录 `attempt_path=keepalive` / `outcome=fail`；
  - passive accepted session 不主动发 keepalive，避免被动侧对 inbound session 误判失败；它由对端 keepalive 的入站 ping 刷新 activity。
- `internal/task/hello.go` 抽出 `buildShellStreamOpen`，让普通 task 和 keepalive 共享同一套 stream-open hello metadata。
- 单测新增：
  - dialed session 未标记 live 时失败会关闭，不进入 active；
  - keepalive 成功记录 `keepalive=ok`；
  - keepalive 失败关闭 active session；
  - passive session 会被 keepalive loop 跳过。

#### 9.4.5 验证记录

focused validation：

- 已通过：
  - `go test ./connectivity ./dataplane`
  - `go test ./internal/task ./internal/pocacceptor ./cmd/miopunch`
  - `go build ./cmd/miopunch`

真实 Windows/WSL2 mirrored 环境验证：

- 成功 keepalive 样本：
  - Windows -> Linux `ping -u UANSTBEARWBWOEMGHHY4454E5I` 成功；
  - 等待 75 秒后，Windows topology 仍显示 active：
    `peer_id=UANSTBEARWBWOEMGHHY4454E5I`、`data_proto=quic`、`path_family=udp4`、`healthy=true`；
  - Windows payloads 出现 `keepalive=ok`；
  - Linux passive topology 仍显示 active，并记录来自对端 keepalive 的入站 `ping=ok`；
  - 验证文件：
    `/tmp/miopunch-batch4/run/windows-topology-after-keepalive-02.json`、
    `/tmp/miopunch-batch4/run/linux/topology-after-keepalive-02.json`。
- 中间验证曾发现并修正一个 keepalive 方向问题：
  - 第一版让 passive accepted session 也主动发 keepalive，Linux passive 侧误关闭 inbound session；
  - 修正为 keepalive loop 跳过 `passivePeerSession` 后，第二轮真实样本通过。
- hello failure cleanup 样本：
  - Linux 使用临时 fault binary 拒绝 stream-open hello；该 fault code 未进入最终工作树；
  - Windows -> Linux `ping -u` 返回 `reason_code=BAD_REQUEST`，fact 包含 `fault rejected hello`；
  - Windows topology 之后显示 `active=[]`；
  - attempts 同时包含 transport 成功样本和应用层失败样本：
    `outcome=fail`、`stage=CapabilityHandshake`、`reason_code=UNAVAILABLE`、`stop_condition=hello_failed`；
  - 验证文件：
    `/mnt/c/Users/stati/AppData/Local/Temp/miopunch-batch4/run/windows/windows-to-linux-hello-reject-batch4-02.json`、
    `/tmp/miopunch-batch4/run/windows-topology-after-hello-reject-02.json`。
- `maintain-neighbors` 一致性样本：
  - 同一 hello fault 环境下运行 Windows `maintain-neighbors -u`；
  - 结果：
    `maintain_neighbors_succeeded=0`、
    `maintain_neighbors_failed=1`、
    `active_neighbors=0`；
  - 验证文件：
    `/mnt/c/Users/stati/AppData/Local/Temp/miopunch-batch4/run/windows/windows-maintain-neighbors-hello-reject-batch4-01.json`。
- 验证后已把两侧 daemon 恢复为非 fault 的当前二进制。

当前状态：

- 运行时 active truth 与应用层 keepalive 已在真实 Windows/WSL2 mirrored 环境验证。
- 未跑完整 gate；本批按本轮策略留到链路层修复完成后统一验证。
