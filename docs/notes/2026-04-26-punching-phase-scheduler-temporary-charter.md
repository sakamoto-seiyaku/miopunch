# Punching phase scheduler 临时纲领（2026-04-26）

> 状态：临时讨论记录。
> 目的：记录 F-001 引出的 UDP/TCP punching 执行模型讨论结论。
> 边界：本文不是正式设计文档，也不是 OpenSpec change；后续 F-002/F-003/F-005 等问题讨论完成后，再统一整理进正式设计或变更提案。

## 背景

MNT-01 场景一复测中，真实双节点 UDP NAT2/NAT3 组合没有稳定打通。代表失败形态是两端已经完成 MQTT candidate exchange，并进入 `PunchAttempt`，但最终在 `wait detect message` 上超时。

这个问题不应被理解成 MQTT 专用问题，也不应被处理成某个 NAT pair 的临时修补。它暴露的是当前 UDP punching executor 的基础执行模型不够稳健：现有流程更接近“按 role 先发一批探测包，然后进入等待”，而 NAT2/NAT3 这类 filtering 受限但 mapping 稳定的场景，需要 receiver 低 TTL 开洞包与 sender 普通包在同一预算窗口内稳定重叠。

当前证据链已经基本闭合：

- NAT 分类只基于 `mapped_addrs` 的映射变化，因此 NAT2/NAT3 会被归到 `EasyNAT/BehaviorNoChange` 是可解释的。
- mode0 首选行为是 sender 无延迟，receiver 使用低 TTL。
- MQTT signaling 当前只提供公共 `start_at` barrier，保证两端都准备好后开始，但不表达 punching phase 内部的角色相位。
- WAN pcap 中可观察到 sender 普通包先到受限 NAT，receiver 低 TTL 包后到；sender 没有后续普通包补上，因此最终超时。

## 关键区分

本轮讨论确认需要严格区分两类时间语义。

### Exchange schedule

Exchange schedule 属于 signaling/backend 层。它回答：

- 双方什么时候拿到同一轮 `NatHoleResp`。
- 双方什么时候进入同一轮 start window。
- backend 的 `realtime`、`scheduled`、`store-and-poll`、`min_lead_time`、clock skew 等 profile 是否满足这一轮交换要求。

MQTT、NATS、DHT、Git、Email 等 backend 后续可能有完全不同的延迟和投递模型，但它们都不应该解释 NAT role，也不应该在 backend 内部硬编码 sender/receiver 的 NAT timing。

### Punching phase schedule

Punching phase schedule 属于 punching decision 和 executor 层。它回答：

- 本轮使用哪个 mode。
- 双方分别是什么 role。
- receiver 是否需要低 TTL。
- sender 是否需要 delay。
- 需要哪些 candidate、assisted、range port 或 random port probe。
- 在多长预算窗口内持续 probe。
- 何时判定成功并取消剩余 probe。

F-001 的核心错误是把 exchange readiness 当成 punching phase ordering。公共 `start_at` 只能保证两端同时进入 attempt，不能保证 receiver 低 TTL 开洞先于 sender 普通包形成有效重叠。

## 设计结论

### 不采用 backend 专用修复

不应在 MQTT session 中加入“如果本端是 sender 就 sleep 一段时间”的逻辑。这样虽然可能短期修复 F-001，但会把 NAT 打洞条件藏进某个 signaling backend。未来引入 NATS、DHT、store-and-poll backend 后，同一逻辑会被复制到多个 backend，形成设计债务。

正确边界是：

```text
backend:
  exchange snapshots
  deliver NatHoleResp
  publish or observe start window

punching decision:
  derive mode / role / ttl / delay / targets / budget

punching executor:
  execute phase plan
  probe within budget
  cancel on success
```

### 统一执行模型

UDP punching executor 应从“一次性顺序脚本”升级为 receive-first 的 punching phase scheduler。

目标模型：

```text
Punching round
  1. exchange 层完成本轮信息交换和 start window 对齐
  2. decision 层输出 backend-neutral phase plan
  3. executor 先创建 listen sockets
  4. executor 先启动 receive loop
  5. executor 按 role 和 delay 启动 send/probe loop
  6. send/probe loop 在预算内持续尝试
  7. 任一方向收到有效路径或连接建立后，取消剩余 probe
```

这个模型不是 NAT2/NAT3 特例。NAT2/NAT3 只是最早暴露了旧模型中“首包顺序决定成败”的问题。

## UDP 与 TCP 的关系

本轮讨论确认，TCP punching 应继续遵循“复用 UDP 整体流程，只在 TCP 必须不同的地方特殊处理”的原则。

更准确的表述是：

> UDP 和 TCP 共享 punching phase scheduler；UDP/TCP 各自实现 protocol probe adapter。

共享部分包括：

- round/start window 由 exchange 层提供。
- phase plan 由 punching decision 输出。
- executor 先打开接收窗口。
- executor 按 role/delay 启动发送窗口。
- probe 在预算内持续运行。
- 成功后取消剩余 probe。
- 诊断字段统一记录 mode、role、delay、budget、targets 和结果。

协议 adapter 差异包括：

| 维度 | UDP adapter | TCP adapter |
|---|---|---|
| probe 单元 | 发送加密 SID datagram | 发起 TCP connect |
| 接收窗口 | `ReadFromUDP` 等 detect/response | `AcceptTCP` 等 established connection |
| 成功判定 | 解出正确 SID，必要时回 response | TCP connection established |
| TTL | receiver 可使用低 TTL 开洞 | 无 UDP 低 TTL detect 语义 |
| 端口策略 | UDP 可用较大包量和端口范围 | TCP 受 source port、TIME_WAIT、RST、conntrack 影响 |
| TCP 特化 | 不适用 | `+100` candidate、较小 spraying 规模、reuse addr、settle window |

因此，TCP 不能原封不动复用 UDP 的 send function，但可以并且应该复用同一个 phase scheduler 心智模型。

当前 TCP punching 实现已经更接近这个模型：它先启动 accept loop，再按 `SendDelayMs` 启动 dial jobs，并在总预算内按 interval 重复 dial，成功后通过 settle window 取消剩余尝试。后续 UDP 修复应向这个统一模型靠拢，而不是让 TCP 回退到 UDP 当前的一次性发包模型。

## 设计不变量

后续正式变更应遵守这些不变量：

- 不在 MQTT、NATS、DHT、Git、Email 等 backend 中编码 NAT role timing。
- 不把 NAT2/NAT3 写成 punching executor 的特殊分支。
- 不改变 NAT 分类本身，除非后续单独引入 filtering-aware 探测设计。
- 不改变 candidate 生成和 direct path 优先级。
- 不用 backend delay 替代 `DetectBehavior.SendDelayMs`。
- 所有 probe 必须有预算、有取消、有诊断。
- UDP/TCP 的差异应收敛在 protocol probe adapter，不应分裂成两套产品流程。
- TCP 的 `+100`、spraying 规模、RST/TIME_WAIT 风险控制属于 TCP adapter 责任。
- UDP punching 输出仍是可用 UDP path；dataplane transport 继续拥有 payload exchange 语义。

## 对 F-001 的解释

F-001 应归类为 punching phase scheduler 设计缺口，而不是单纯 MQTT signaling 缺口。

现有 MQTT `start_at` 做到了“双方准备好后同时开始”，但没有表达“本轮 punching 内部 receiver 先开洞、sender 后发普通包”的相位计划。原始 FRP coord 流程中存在 role-aware response/start timing，这在 NAT2/NAT3 下不是可忽略实现细节，而是打洞成立条件的一部分。

迁移到 MQTT-only、per-peer SID、短生命周期 decision engine 后，这个隐含相位没有被建模为产品契约，于是 mode0 在 NAT2/NAT3 下表现为首包顺序敏感。

正确修复方向不是恢复一个产品 coord 依赖，而是把该相位显式纳入 backend-neutral punching phase plan，并由 executor 统一执行。

## 对未来多 backend 的影响

Door 3 后续会引入多种 signaling backend。部分 backend 是 realtime push，例如 MQTT/NATS；部分 backend 可能是 scheduled 或 store-and-poll，例如 DHT/Git/Email。

因此，设计上应保持：

```text
backend profile:
  can this backend deliver the round plan before the start window?

punching phase plan:
  once both peers enter the window, how should probes be scheduled?
```

慢 backend 可以要求更大的 `min_lead_time` 和 start window，但不应改变 punching phase 内部的 NAT role 语义。若某个 backend 无法满足 start window 或 clock skew 要求，该 round 应在 exchange 层失败或切换 backend，而不是让 punching executor 根据 backend 类型改变 NAT 行为。

## F-005：TCP assisted candidate 语义对齐

F-002 的最新复测说明，`mnt01-self-tcp4-direct` 与 `mnt01-self-tcp-portmap` 当前虽然可以完成 payload exchange，但成功路径是 `punching_tcp4`，不是 `direct_tcp4`。更关键的是，复测暴露出 TCP 地址语义没有与 UDP 对齐：TCP 私网 listen 地址被放入 `tcp_direct_addrs`，随后被 decision 下发为 `peer_tcp_direct_addrs`，attempt 再以 `direct_tcp4` 分支尝试这些私网地址。

这不是 TCP `+100`、TCP STUN、simultaneous open 或 spraying 的问题，而是 TCP candidate 分层缺失。

UDP 已经有清楚的地址语义：

- `direct_addrs`：真正可直连的 direct candidate。
- `assisted_addrs`：私网/本地辅助地址，可作为 punching input，但不代表 direct path 成功。

TCP 应按同一原则建模：

- `tcp_direct_addrs` 只表达真正可直连 TCP candidate，例如公网 TCP direct、TCP portmap direct、可路由 IPv6 TCP direct。
- 私网/本地 TCP listen 地址应进入 `tcp_assisted_addrs`，不再进入 direct path。
- `tcp_assisted_addrs` 属于 TCP punching input，而不是 direct candidate。

### 临时结论

当前讨论确认以下临时契约。

#### 字段契约

新增显式 `tcp_assisted_addrs` 语义，而不是复用 UDP `assisted_addrs` 或继续把私网地址塞进 `tcp_direct_addrs`。

request 侧：

- `tcp_direct_addrs`：本端提供的真实 TCP direct 入口。
- `tcp_assisted_addrs`：本端提供的 TCP assisted/private punching input。
- `tcp_mapped_addrs`：TCP STUN 观测结果，仍记录 base port `P` 的映射，不在 gather 侧做 `+100`。

response 侧：

- `peer_tcp_direct_addrs`：对端真实 TCP direct 入口，只供 `direct_tcp4/direct_tcp6` 使用。
- `tcp_assisted_addrs`：对端 TCP assisted/private punching input，只供 `punching_tcp4` 使用。
- `tcp_candidate_addrs`：由 TCP STUN 或 TCP view selection 派生的 punching candidate，decision 侧按 TCP `P+100` 约定输出可尝试目标。

response 侧字段命名使用 `tcp_assisted_addrs`，不使用 `peer_tcp_assisted_addrs`，以便和 UDP response 的 `assisted_addrs` 保持一致：它表达“本端可以尝试的对端 assisted targets”。

本轮不考虑旧节点向前兼容。若新语义下仍收到明显不合规的私网 IPv4 `tcp_direct_addrs`，decision 应丢弃并记录诊断，不自动降级到 `tcp_assisted_addrs`，以避免掩盖 gather 或测试 fixture 的生产者错误。

#### 地址分类

`tcp_direct_addrs` 的分类按来源优先，而不是只按 IP 是否公网判断：

- IPv4 公网本机 TCP listen 地址可以进入 `tcp_direct_addrs`。
- IPv4 私网、CGNAT、本地 listen 地址进入 `tcp_assisted_addrs`，不进入 `tcp_direct_addrs`。
- IPv6 继续沿用现有 IPv6 candidate filter。可路由 IPv6、以及 lab 中已通过路由建模的 ULA IPv6，仍作为 `direct_tcp6` candidate。
- TCP portmap/UPnP/NAT-PMP 显式映射结果可以进入 `tcp_direct_addrs`。
- TCP portmap 结果允许 `100.64.0.0/10` 这类 lab/CGNAT 同域映射入口，但应拒绝 loopback、link-local、unspecified、RFC1918 等明显不应作为跨域 direct 的地址。

`tcp_assisted_addrs` 限定为 IPv4，使用 TCP listen/punching 端口 `L=P+100`，并受现有 `DisableAssistedAddrs` 开关控制。关闭 assisted 后，UDP `assisted_addrs` 与 TCP `tcp_assisted_addrs` 都不应交换私网辅助地址。

#### Decision 行为

TCP STUN 证据足够时，decision 继续使用 TCP NAT analysis 生成 mode、role、delay、candidate ports 和 random port 规模。

TCP STUN 证据不足但存在 TCP assisted/candidate 目标时，允许最小 best-effort TCP assisted punching，而不是直接禁用 TCP punching。该 fallback 不声称完成了 NAT feature analysis，只表达“有明确目标，可做 bounded accept+dial 尝试”。

assisted-only TCP punching 的行为为最小 mode0：

- `Mode=0`
- `SendDelayMs=0`
- `CandidatePorts=nil`
- `SendRandomPorts=0`
- `ListenRandomPorts=0`
- 本 response 有可拨目标的一侧 role 为 `sender`
- 没有可拨目标但仍参与 accept 的一侧 role 为 `receiver`
- 如果双方都有 assisted target，双方都可以是 sender，因为 TCP executor 已经是 receive-first，重复连接由 settle/cancel 收敛

#### Attempt 行为

attempt 顺序保持 `tcp6 direct -> tcp4 direct -> tcp4 punching -> udp6 direct -> udp4 direct -> udp4 punching`。`direct_tcp4` 只尝试 `peer_tcp_direct_addrs`，不尝试 `tcp_assisted_addrs`。

`punching_tcp4` 的 target builder 必须区分两个 bucket：

- exact targets：`tcp_assisted_addrs` 与 `tcp_candidate_addrs` 的原始目标。
- sprayable candidate IPs：仅来自 `tcp_candidate_addrs` 的 IP，可应用 `tcp_detect_behavior.candidate_ports` 与 random ports。

不能把 `tcp_assisted_addrs` 与 `tcp_candidate_addrs` 简单混成一个列表后统一扩展端口。否则会把 range port 或 random spraying 错误套到私网 assisted IP 上。

当 assisted target 与 STUN-derived candidate 同时存在时，`SendDelayMs` 作用于全部 TCP dial targets，避免 assisted 绕过同一轮 phase plan。TCP executor 应遵守 decision 下发的 phase budget，即以 `SendDelayMs + ReadTimeoutMs` 作为本轮 TCP punching 的执行窗口；本地可以有安全 clamp，但不应把 `ReadTimeoutMs` 只当日志字段。

assisted TCP punching 成功仍记为 `punching_tcp4`，不新增 `assisted_tcp4` path 名称。诊断需要增强：至少记录 assisted exact、candidate exact、candidate expanded 的目标计数，以及最终 winning target 的来源，避免后续再把 assisted fallback 误读成 direct 覆盖。

### 对 F-002 的影响

F-002 的 case 层问题仍然成立：当前 `tcp4-direct` 与 `tcp-portmap` case 名称指向 direct 验证，但 fixture 没有构造出可用于 `direct_tcp4` 的公网 TCP 入口。

但在测试修正前，需要先把 F-005 的地址语义定清楚并落地。否则直接改 case 名称或断言，会继续掩盖 TCP 私网地址被误解释为 direct candidate 的问题。

设计完成后的测试方向应分开：

- 现有 `mnt01-self-tcp4-direct` 不应继续声称验证 TCP4 direct。当前双 NAT fixture 中只有 `10.0.x.2:L` 私网 listen 地址，后续应改为 TCP assisted/fallback 语义，并断言 `punching_tcp4` 与 payload 证据。
- 真正的 TCP4 direct 覆盖需要另建具备真实公网 direct candidate 的 fixture，或由其他 direct-capable 拓扑承担，再断言 `direct_tcp4`。
- `mnt01-self-tcp-portmap` 若目标是验证 TCP portmap direct，lab NAT-PMP helper 必须真实支持 TCP mapping，并产出可用于 `direct_tcp4` 的 TCP portmap candidate；否则该 case 只能被归为 fallback/punching 覆盖。
- `tcp-portmap-direct` 一旦 fixture 能产出 TCP mapping，应断言 `direct_tcp4`，而不是继续 `diag-fail-allowed`。

## Cross-round success memory：FRP-style analyzer

F-001 与 F-005 的根因不在跨轮记忆：F-001 是 phase scheduler 的时序模型缺口，F-005 是 TCP candidate 分层缺失。但本轮讨论确认，迁移到 MQTT/task 路径后丢失 FRP 式长期 analyzer，也会降低长期运行 daemon 的自适应能力。

FRP 的参考模型是：

- controller 长期持有 analyzer。
- 每一轮 nat hole session 是临时的。
- analyzer 只记录某个 NAT feature key 下成功过的 `mode/index`。
- 下一轮仍重新 gather/exchange，但推荐行为时会更偏向历史成功过的 `mode/index`。

miopunch 当前已有 `punchdecision.Engine` 与 `Analyzer`，并保留了 FRP 的 score 机制；coord controller 路径也已经用长期 `Engine` 和 `NatHoleReport` 调用 `ReportSuccess`。问题在于 MQTT/task 路径当前使用 `AnalyzeOnce`，每轮都会创建短生命周期 decision engine，因此无法跨轮学习。

### 临时结论

采用严格最小的 success-only memory，不把它扩展成复杂状态机。

- 记忆只属于单个 peer pair，不在不同 peer 之间共享。
- 产品路径的 memory key 应为 `remote_peer_id + protocol + existing_analysis_key`。
- 没有 `peer_id` 的 lab/standalone 路径，可以临时使用 `proxy_name` 或 SID 级诊断 key，但不应把这个 fallback 当成产品身份模型。
- `protocol` 必须进入 key，避免 UDP 与 TCP 的成功经验互相污染。
- 记录内容只包括 `protocol`、`analysis_key`、`mode`、`index`、`score`、`last_update_time`。
- 不记录 endpoint、candidate addr、mapped addr、winning target、major path、target source 或失败原因。
- 不持久化到磁盘；daemon 重启后重新学习。
- TTL 使用 2 小时，过期后清理。
- scoring 复用 FRP 行为：成功 `score += 2`，被推荐/使用 `score -= 1`，上限 `10`。
- 只记成功，不记失败；不做失败 cooldown、失败降级或失败原因推断。
- 只有做出 decision 的一侧学习；非 decision side 不应根据本地现象各自推断并写入 analyzer。

### 成功信号边界

这里的 success memory 属于 punching decision 层，成功信号应定义为 punching path 建立成功，而不是 data plane payload exchange 成功。

原因是 dataplane transport 由 KCP/QUIC/TLS stream 等上层负责。F-003 这类 payload/flush/drain/linger 问题不应反向污染 punching mode/index 学习；否则会把“洞已经打通但数据层失败”的结果错误解释成 punching 行为失败。

因此：

- UDP punching 成功：收到有效 SID detect/response 并形成可用 UDP path。
- TCP punching 成功：`punching_tcp4` 建立可用 TCP connection。
- direct path 成功不参与 NAT hole analyzer 学习，因为它没有使用 punching `mode/index`。
- data plane payload 失败仍应由 dataplane diagnostics 记录，不写入 analyzer 失败记忆。

### 执行边界

跨轮记忆只影响 punching behavior 的 `mode/index` 推荐，不改变本轮其它输入和执行边界。

每一轮仍必须重新执行：

- gather 本端 UDP/TCP listen、mapped、direct、assisted、portmap 信息；
- exchange 当前 round snapshot；
- 生成当前 round candidate/assisted target；
- 执行 backend-neutral phase plan。

跨轮记忆不得做这些事：

- 不复用旧 endpoint 或旧 candidate。
- 不跳过 gather 或 exchange。
- 不改变 major path 顺序。
- 不对 `direct_tcp4`、`punching_tcp4`、`udp4 direct`、`udp4 punching` 等 path source 单独打分。
- 不为 target source、candidate bucket 或 attempt path 另建 score。

后续实现上，MQTT/task 路径不应继续直接使用 per-round `AnalyzeOnce`，而应在 daemon 生命周期内持有长期 `punchdecision.Engine` 或等价 per-peer analyzer，并用上述 key 隔离 peer 与 protocol。

## F-003：peer transport session 与 logical stream 分层

F-003 的现象是 `mnt01-smoke-kcp-transport` 已经完成 candidate exchange、进入 `PunchAttempt`，并以 `attempt_path=punching_ipv4` 建立路径；随后 `hello=ok`，但读取 `ping` response 超时。

这不应归类为 UDP punching 链路失败。`hello=ok` 已经证明 KCP stream 至少承载了第一轮控制交换；失败发生在同一条 stream 的后续请求/响应阶段。当前代码中 `dataplane.DialStream` / `ServeStream` 返回裸 `io.ReadWriteCloser`，acceptor 在 `ping` 写完响应后返回并关闭 stream，等价于把一次业务 op 的生命周期直接绑定到底层 KCP/QUIC/TLS carrier 生命周期上。KCP 最先暴露问题，是因为 UDP/KCP 过早关闭会让最后一段响应更容易卡在 flush/retransmit/close 时序里；但根因不是 KCP 专用，而是 dataplane/session 抽象缺口。

旧 one-shot KCP payload 路径曾在写完 response 后短暂保持 UDP/KCP socket 存活，避免响应还没被对端读到就收到 early close/ICMP 等影响。该 linger 是一次性交换时代的补偿，不应成为 stream 化后的根本设计。

### 外部参考

FRP XTCP 的参考价值在于 tunnel session 分层，而不是具体安全模型。

- visitor 侧在 `makeNatHole()` 打洞成功后调用 `session.Init(listenConn, raddr)`。
- 后续用户连接通过 `session.OpenConn()` 获得 logical connection；如果已有可用 tunnel session，就复用 session 上的新 stream。
- KCP 路径是 `KCP -> yamux session -> Open/Accept stream`。
- QUIC 路径是 `quic.Conn -> OpenStream/AcceptStream`。
- proxy 侧对应地在 session 上循环 accept stream，并把每个 stream 交给 work connection handler。

gonc 的参考价值在于 secure negotiation 与 mux 的组合顺序。

- P2P 建路后先执行 `secure.DoNegotiation`。
- TCP P2P 默认把 `SecureLayer` 设置为 `tls13`。
- `:mux` 在 negotiated conn 上创建 `smux/yamux` session。
- 业务连接再通过 `OpenStream` / `AcceptStream` 进入 mux session。

因此，miopunch 不应继续把 punching 之后的连接直接暴露成单个业务流，而应把它提升为 peer transport session。

### 目标分层

本轮临时结论采用以下结构：

```text
punching path
  -> secure peer transport session
    -> mux/native stream layer
      -> generic logical stream(kind, metadata)
        -> payload protocol
```

协议对应关系为：

- TCP：`TCP carrier -> TLS 1.3 identity binding -> smux -> logical streams`。
- KCP：`UDP punching path -> KCP carrier -> TLS 1.3 identity binding -> smux -> logical streams`。
- QUIC：`QUIC native TLS 1.3 identity binding -> native QUIC streams`。

KCP 不使用 kcp-go optional block crypto 作为主安全层；QUIC 不再额外套一层 TLS；TCP 与 KCP 应尽量共享 TLS 1.3 identity binding 与 session/mux 设计。

### Session 生命周期

采用 on-demand live session，而不是 per-task-only，也不是本轮直接照搬 FRP proactive tunnel。

- daemon 内存持有 per-peer live `PeerTransportSession`。
- session key 至少包含 `remote_peer_id`、transport protocol、security identity 和 path family；具体 key 结构在正式设计中细化。
- 当 session 存活且认证仍有效时，新操作只打开新的 logical stream。
- 当 session 不存在、已关闭、认证失效或底层 transport fatal error 时，下一次操作重新执行 gather、exchange、punching 和 secure session 建立。
- 不持久化 session endpoint、candidate、mapped addr 或 winning target。
- 不复用已关闭 session 的旧 endpoint；session 死后从当前网络状态重新建路。
- daemon shutdown、peer membership revoke、identity/config change、idle timeout、transport fatal error 都可以关闭 session。

这个设计与 success-only cross-round memory 不冲突。success memory 只影响 punching decision 的 `mode/index` 推荐；live session reuse 只复用当前仍然存活且已认证的 transport session。

### Role 边界

必须区分三类 role：

- NAT role：`sender` / `receiver` / detect behavior，只属于 punching phase。
- session role：secure session、QUIC、smux 的 client/server 或 opener/acceptor role。
- stream role：logical stream opener 与 logical stream handler。

NAT sender/receiver 不应泄漏到 session/mux 设计里。当前 `ping` 是 task 侧打开 logical stream，acceptor 侧处理；未来若允许另一侧主动打开 stream，也必须由 session policy 和 stream kind policy 授权，而不是继承 NAT role。

### Stream 与授权边界

logical stream 必须是通用抽象，不得写死成 `shellproto ping/sh` 专用通道。

- 每个 logical stream 打开时先声明 stable `kind` 和小型 structured `metadata`。
- stream-open 阶段完成 peer membership、revocation、kind、target、session 等授权。
- `shellproto` 只是当前 payload protocol，可作为 `kind=shell.v0` 的上层内容继续存在。
- 未来 `socks5.v0`、`http-forward.v0`、`file.v0` 等业务不需要伪装成 shellproto hello/ping/sh。

transport identity 与 stream authorization 也必须分开：

- secure peer transport session 证明这条 session 对端是谁，是否匹配预期 peer。
- stream-open authorization 判断该 peer 是否允许打开本次 logical stream。
- payload protocol 只负责该 kind 内部的业务数据和业务控制帧。

因此，当前 `hello` 的长期位置不应继续是 shellproto payload 内的固定前置帧，而应逐步迁移为 generic stream-open authorization。过渡阶段可以保留 shellproto hello 兼容现有任务，但正式设计不应把 logical stream 绑定到 shellproto。

### 关闭与 timeout 语义

关闭 logical stream 只结束当前业务请求/响应，不关闭 peer transport session。关闭 peer transport session 才关闭所有 logical streams、mux/QUIC session、secure session、carrier 和底层 UDP/TCP socket。

F-003 的根本修复不应是 `servePing` sleep，也不应是 KCP 专用 linger。正确方向是让 `ping` handler 写完 response 后只关闭 logical stream；底层 session 继续由 mux/QUIC 的 flush、FIN、flow control、keepalive 和 idle timeout 负责。

临时纲领只锁定 timeout 语义，不锁死具体默认值：

- session 必须有 keepalive 或等价活性检测。
- session 必须有 idle timeout，避免 daemon 无限持有无用 peer session。
- 每个 logical stream 必须有独立 deadline，不继承 punching round timeout。
- close reason 必须可诊断，至少区分 idle timeout、daemon shutdown、identity/config change、auth revoked、stream protocol error 和 transport fatal error。
- 具体 keepalive、idle timeout、stream deadline 默认值留到正式实现方案和 KCP/TCP/QUIC 测试阶段确定。

### 对 F-003 的解释

F-003 应归类为 dataplane peer session / logical stream 设计缺口。

当前实现把“打洞成功后的 carrier”和“单次业务操作 stream”合并成一个裸 `io.ReadWriteCloser`，导致 `ping` 这类短操作返回时可能关闭整个 KCP carrier。KCP 的 flush/drain/close 敏感性让这个缺口显性化；TCP TLS stream、QUIC stream 以及后续 CCK/TCP+TLS1.3 等路径也可能遇到同类生命周期问题，只是表现形式不同。

后续正式设计应把 F-003 从 KCP one-off bug 提升为统一 transport session 设计变更，并在实现上引入：

- peer session manager；
- TCP/KCP 上的 `TLS 1.3 + smux` session；
- QUIC native stream session；
- generic logical stream open envelope；
- stream-level authorization；
- session/stream close diagnostics。

## 后续待讨论问题

- F-002/F-005：TCP 私网 assisted candidate 对齐方向已临时记录；后续需在正式设计中同步字段契约、地址分类、decision/attempt 行为与测试重组。
- F-003：peer transport session / logical stream 分层已临时记录；后续需在正式设计中同步 session manager、stream-open auth、smux/QUIC stream 语义与测试收紧。
- UDP/TCP phase scheduler 是否需要抽象成显式内部类型，还是先保持在各自 executor 内但共享语义。
- `DetectBehavior` 是否需要新增更明确的 budget/probe interval 字段，还是继续复用当前 `ReadTimeoutMs` 和协议内默认节奏。
- Cross-round success memory 已临时记录；后续需在正式设计中同步 per-peer key、success signal 与 MQTT/task 长期 analyzer 接入方式。
- Lab diagnostics 是否需要新增事件来明确记录 receive loop start、probe loop start、first probe、first valid message、cancel reason。

## 临时结论

当前讨论已确认的方向是：

> 把 punching 从一次性顺序发包模型，提升为 backend-neutral、receive-first、bounded probe window 的 phase scheduler。UDP 和 TCP 共享这套 phase scheduler 语义，各自实现协议 probe adapter。

该结论后续应与 TCP assisted candidate、peer transport session / logical stream 分层等问题一起整理进正式设计文档。
