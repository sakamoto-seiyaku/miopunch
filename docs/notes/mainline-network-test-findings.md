# 主线网络测试发现问题记录

日期：2026-04-25

本文用于记录实现或运行主线网络测试时发现的项目代码问题。这里记录的问题不是测试设计本身的问题，也不在测试重构过程中顺手修复；后续按本清单单独排期、定位和修复。

## 记录原则

- 只记录项目实现、产品行为、诊断字段、状态落盘、恢复语义或权限边界中的问题。
- 不记录测试夹具、runner、环境依赖或用例编排本身的问题；这些应直接在对应测试变更中修正。
- 每条问题必须包含可复现条件、期望行为、实际行为、证据位置和建议后续动作。
- 能归类为场景 1/2/3 的问题，应标注对应场景和 case。
- 不在本文件中隐藏失败；若问题影响 required gate，应明确标注阻塞级别。

## 问题模板

### F-000：标题

- 场景：场景 1 / 场景 2 / 场景 3
- 影响：blocker / high / medium / low
- 状态：open / investigating / fixed / wontfix
- 复现条件：
- 期望行为：
- 实际行为：
- 证据：
- 初步判断：
- 后续动作：

## 最新复测摘要（2026-04-26）

- `./lab/host/labctl xtcp-connectivity-selftest`：通过，`pass=6 fail=0`；`p2-05-tcp-spraying` 当前可复现为成功，包含 `attempt.tcp_punching.ok` 与 payload 证据。证据：`lab/_artifacts/20260426T065218Z-xtcp-p2-05-tcp-spraying-tcp-quic-bbr/visitor.log`。
- `./lab/host/labctl mnt01-selftest`：失败，`pass=21 fail=1`；UDP NAT2/NAT3/NAT4-regular 相关失败仍存在，TCP4 direct/portmap 本轮未复现失败，`mnt01-self-ipv6-udp4-fallback` 已确认为测试用例初始条件与 `direct_ipv4` 期望不匹配。证据：`lab/_artifacts/20260426T065238Z-mnt01-selftest-aggregate/summary.json`。
- `./lab/host/labctl mnt01-smoke`：通过，`pass=8 fail=0`；KCP transport 仍以 `diag-fail-allowed` 形式复现为 `hello=ok` 后读取 ping response 超时。证据：`lab/_artifacts/20260426T065845Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`。

## Open Findings

### F-001：MNT-01 真实双节点 UDP NAT2/NAT3 组合未稳定打通

- 场景：场景 1 / MNT-01
- 影响：high
- 状态：open（设计结论已收敛；进入 `fix-punching-phase-scheduler`）
- 复现条件：`./lab/host/labctl mnt01-selftest`，真实 `miopunch up` 双节点、MQTT-only、fixture 使用 per-peer SID；代表失败 case 包括 `mnt01-udp-udp-nat2-x-udp-nat1`、`mnt01-udp-udp-nat3-x-udp-nat1`、`mnt01-udp-udp-nat2-x-udp-nat2`、`mnt01-udp-udp-nat3-x-udp-nat2`、`mnt01-udp-udp-nat3-x-udp-nat3`。
- 期望行为：NAT1/NAT2/NAT3 代表 UDP punching case 能证明 payload exchange，或输出更细的稳定诊断。
- 实际行为：case 进入 `PunchAttempt` 后超时，例如 `wait detect message error: read udp4 0.0.0.0:5001: i/o timeout`。
- 证据：`lab/_artifacts/20260426T045623Z-mnt01-mnt01-udp-udp-nat2-x-udp-nat1/attempt-1.md` 及同轮 MNT-01 selftest artifacts；2026-04-26 复测仍可复现，代表证据为 `lab/_artifacts/20260426T065245Z-mnt01-mnt01-udp-udp-nat2-x-udp-nat1/attempt-1.md`、`lab/_artifacts/20260426T065257Z-mnt01-mnt01-udp-udp-nat3-x-udp-nat1/attempt-1.md`、`lab/_artifacts/20260426T065424Z-mnt01-mnt01-udp-udp-nat3-x-udp-nat3/attempt-1.md`。MQTT pcap 中可还原决策输入/输出：两端 `mapped_addrs` 都是稳定端口，因此被判为 `EasyNAT/BehaviorNoChange`；下发结果为 `punching_enabled=true`、client `role=sender`、visitor `role=receiver ttl=7`。WAN pcap 中失败 case 只出现两包：`TTL=63` 的普通 sender 包先到受限 NAT，随后 `TTL=6` 的 receiver 低 TTL 包才出现；之后没有 response。
- 初步判断：
  - 这不是单纯测试用例设计错误，也不是没有进入 UDP punching。问题来自 FRP XTCP 流程移植到 MQTT-only / per-peer SID 产品路径后，控制面时序语义没有完整搬运。
  - 原版 FRP XTCP 的底层 NAT 分类也只根据 `mapped_addrs` 判断 mapping 是否变化；NAT2/NAT3 这类 endpoint-independent mapping 但 filtering 受限的场景，也会被归到 `EasyNAT/BehaviorNoChange`，因此底层 mode 表同样存在 mode0 首包时序风险。
  - 原版 FRP coord 流程有额外缓冲：当某端是 sender 时，coord 下发该端 `NatHoleResp` 前会延迟约 1 秒，让 receiver 先拿到指令、先发低 TTL 探测包，从而打开 NAT2/NAT3 的过滤状态；coord 还长期持有 analyzer，可根据成功 report 对 mode/index 做跨次学习。
  - 当前 MQTT signaling 路径不同：`AnalyzeOnce` 每次使用短生命周期 decision engine，没有跨次 analyzer 学习；`internal/signaling/mqtt/session.go` 通过 `ready`/`start_at` 让两端同步开始，但没有按 `DetectBehavior.Role` 对 sender 增加启动延迟。结果是 sender 普通包可能先到 NAT2/NAT3，被 address-dependent 或 address/port-dependent filtering 丢弃；receiver 低 TTL 包后到时只能打开后续过滤状态，但 sender 没有再发第二轮普通包，最终 `wait detect message` 超时。
  - 设计问题的核心是：我们移植了 FRP 的 punching kernel 与 mode 表，却把原 coord 中隐含的 role-aware response/start timing 当成了可忽略的信令细节。对 NAT2/NAT3，这个时序不是实现细节，而是打洞成立条件的一部分。
  - 这个问题可能影响 TCP punching/spraying 的设计口径。TCP 侧同样依赖 sender/receiver 的有效发包窗口重叠；此前 `p2-05-tcp-spraying` 临时修复中的 sender/receiver delay 对齐，和本问题属于同一类“控制面下发语义影响打洞相位”的问题。
- 设计结论：该问题已收敛为 punching phase scheduler 设计缺口，而不是 MQTT 专用问题。正确修复方向是 backend-neutral、receive-first、bounded probe window 的 phase scheduler：exchange/backend 只负责本轮 snapshot 与 start window，punching decision/executor 负责 mode、role、delay、TTL、targets、probe budget 与 cancel reason。
- 后续动作：进入 `fix-punching-phase-scheduler` change。该 change 应统一 UDP/TCP phase scheduler 语义，补齐 UDP receive-first + bounded retry，接入 success-only per-peer analyzer，并新增可复盘诊断事件。F-004 的测试期望修正也随该 change 收尾。

### F-002：MNT-01 真实双节点 TCP4 direct/portmap direct 未稳定打通

- 场景：场景 1 / MNT-01
- 影响：high
- 状态：open（测试修正依赖 `align-tcp-assisted-candidates`）
- 复现条件：`./lab/host/labctl mnt01-selftest`，代表失败 case 包括 `mnt01-self-tcp4-direct` 和 `mnt01-self-tcp-portmap`。
- 期望行为：TCP4 direct 或 NAT-PMP portmap direct 能证明 payload exchange，或输出明确 selected/failed path 诊断。
- 实际行为：历史运行中 case 完成 MQTT candidate exchange 后进入 `PunchAttempt`，最终 `context deadline exceeded`；2026-04-26 复测中 `mnt01-self-tcp4-direct` 与 `mnt01-self-tcp-portmap` 均已证明 payload exchange，但成功路径为 `attempt_path=punching_tcp4`，不是 `direct_tcp4`。
- 证据：历史证据为 `lab/_artifacts/20260426T045921Z-mnt01-mnt01-self-tcp4-direct/attempt-1.md`、`lab/_artifacts/20260426T045939Z-mnt01-mnt01-self-tcp-portmap/attempt-1.md`；最新复测证据为 `lab/_artifacts/20260426T065541Z-mnt01-mnt01-self-tcp4-direct/attempt-1.md`、`lab/_artifacts/20260426T065553Z-mnt01-mnt01-self-tcp-portmap/attempt-1.md`。
- 初步判断：
  - F-002 不能再简单归为产品侧 `wontfix`。它的 case 层结论仍然成立：当前两个 case 的名字和期望语义指向 TCP4 direct 或 TCP portmap direct，但 fixture 没有构造出可被 direct path 使用的公网 TCP 入口。
  - `tcp4-direct` case 只交换私网 `tcp_direct_addrs`，例如 `10.0.1.2:5100` 与 `10.0.2.2:5101`。在真实双 NAT namespace 间，这些私网地址不能作为跨 NAT direct TCP 目标；因此该 case 实际不具备验证 `direct_tcp4` 的前置条件。
  - `tcp-portmap` case 虽然启动 NAT-PMP 环境，但 lab 侧 `mlab-natpmpd` 当前是 UDP-only helper，TCP mapping 请求会被拒绝或不产出 TCP mapped direct candidate。换言之，这个 case 也没有生成可用于 `direct_tcp4` 的 `100.64.x.x:510x` 入口。
  - 产品代码在没有可用 TCP direct candidate 后继续 fallback 到 TCP punching 是符合当前路径选择语义的；最新复测的成功说明 fallback 可用，不能证明 TCP4 direct/portmap direct 已被验证。
  - 但 F-002 也暴露了一个产品设计缺口：UDP 有 `direct_addrs` 与 `assisted_addrs` 分层，私网地址可以作为 punching 的 assisted target；TCP 当前只有 `tcp_direct_addrs`，私网 TCP listen 地址会被归入 `direct_tcp4` 尝试，而不是作为 TCP punching 的 assisted/private candidate 单独建模。
  - 因此 F-002 的测试修正依赖先统一 TCP assisted 设计口径；直接把这两个 case 改名或改断言，会掩盖 TCP 流程没有完整复用 UDP 私网辅助打洞语义的问题。
- 设计结论：F-002 的 case 层问题成立，但它引出的产品设计缺口已归入 F-005：TCP 需要与 UDP 一样区分 direct candidate 与 assisted/private punching input。
- 后续动作：随 `align-tcp-assisted-candidates` change 修正。设计落地后，当前两个 case 不应继续声称验证 TCP4 direct/portmap direct；若目标是覆盖 fallback，则重命名为 TCP punching fallback 并断言 `attempt_path=punching_tcp4` 与完整 payload 证据。真正 direct 覆盖需另建具备真实 direct candidate 的 fixture 或补齐 TCP portmap helper。

### F-003：MNT-01 KCP transport specialty 已建立路径但 ping response 超时

- 场景：场景 1 / MNT-01
- 影响：medium
- 状态：fixed by `add-peer-transport-sessions`
- 复现条件：`./lab/host/labctl mnt01-smoke`，case `mnt01-smoke-kcp-transport`，真实 `miopunch up` 双节点、MQTT-only、`data_proto=kcp`、UDP direct/punching 路径。
- 期望行为：KCP transport variant 在已完成 candidate exchange、`PunchAttempt` 和 hello handshake 后能证明 `ping=ok` payload exchange。
- 原实际行为：case 能进入 `PunchAttempt`，建立 `attempt_path=punching_ipv4`，并完成 `hello=ok`，但读取 ping response 超时；MNT-01 曾将该 specialty 作为 `diag-fail-allowed`，要求完整诊断证据而非静默通过。
- 证据：`lab/_artifacts/20260426T062455Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`；2026-04-26 复测仍可复现，证据为 `lab/_artifacts/20260426T065845Z-mnt01-mnt01-smoke-kcp-transport/attempt-1.md`。
- 初步判断：
  - 这不是 UDP punching 链路层失败。该 case 已经完成 candidate exchange、进入 `PunchAttempt`，并以 `attempt_path=punching_ipv4` 建立路径；`hello=ok` 也证明 KCP stream 至少能承载第一轮 capability/control exchange。
  - 失败发生在同一条已建立 stream 上的第二个控制操作：`miopunch ping` 写入 ping 后等待 response，最终超时。因此问题更接近 dataplane transport / shell protocol 的流生命周期语义，而不是 NAT hole punching 本身。
  - 旧的 one-shot KCP payload 实现会在写完 response 后短暂保持 UDP/KCP socket 存活，避免响应还未被对端读取就被关闭；当前 stream 化实现把 KCP 包装成裸 `io.ReadWriteCloser` 后，一次业务 op 的生命周期会直接绑定到底层 carrier/session 生命周期。
  - acceptor 侧在处理完 hello 与 ping 后返回，defer close 可能过早关闭 KCP stream 和底层 UDP socket；client 侧则仍在等待 ping response。
  - 该问题已收敛为 peer transport session / logical stream 分层缺口，而不是 KCP 专用 linger 问题。后续 TCP TLS、KCP、QUIC 和未来 CCK 等路径都应遵循 `punching path -> secure peer transport session -> logical streams -> payload protocol`。
- 设计结论：采用 on-demand live peer transport session。TCP/KCP 使用 TLS 1.3 identity binding + yamux；QUIC 使用 native QUIC streams；logical stream 使用 generic `kind+metadata`，不写死 shellproto；关闭 logical stream 不关闭 session。
- 修复动作：`add-peer-transport-sessions` 将 dataplane 升级为 peer transport session + generic logical stream；`mnt01-smoke-kcp-transport` 收紧为 `success-required`，必须证明 `hello=ok` + `ping=ok`。

### F-004：MNT-01 IPv6 到 UDP4 fallback 成功但 attempt_path 与验收期望不一致

- 场景：场景 1 / MNT-01 / `mnt01-self-ipv6-udp4-fallback`
- 影响：high
- 状态：wontfix（产品侧；转测试用例修正）
- 复现条件：`./lab/host/labctl mnt01-selftest`，case 使用 `--enable-ipv6 --block-forward-udp6 --expect-path direct_ipv4`。
- 期望行为：IPv6 forward UDP 被阻断后，UDP4 fallback 应证明 payload exchange，并给出与验收一致的 IPv4 fallback path 诊断。
- 实际行为：`miopunch ping` 返回成功，`hello=ok` 且 `ping=ok`，但 report 中为 `attempt_path=punching_ipv4`；runner 随后因缺少 `attempt_path=direct_ipv4` 判定该 required case 失败，使 `mnt01-selftest` 汇总为 `pass=21 fail=1`。
- 证据：`lab/_artifacts/20260426T065629Z-mnt01-mnt01-self-ipv6-udp4-fallback/attempt-1.md`、`lab/_artifacts/20260426T065238Z-mnt01-selftest-aggregate/summary.json`。
- 初步判断：这不是产品打洞失败。按当前 P2 设计，`direct_ipv4` 表示 IPv4 portmap direct candidate 成功；STUN 得到的 mapped address 属于 UDP punching candidate，成功后应记录为 `punching_ipv4`。本 case 期望 `direct_ipv4`，但未启动 NAT-PMP/portmap helper，且 STUN 仍启用，因此实际落到 `punching_ipv4` 并成功是符合当前产品语义的。
- 后续动作：在测试重组中修正该 case 的初始条件：若目标是验证“IPv6 不通后回落到 IPv4 portmap direct”，应启动 NAT-PMP helper，并建议禁用 STUN 以避免 punching 路径干扰；修正后期望 `attempt_path=direct_ipv4`、`hello=ok`、`ping=ok`。

### F-005：TCP 私网地址缺少与 UDP assisted_addrs 对齐的 punching 语义

- 场景：场景 1 / MNT-01 / TCP punching design
- 影响：high
- 状态：open（设计结论已收敛；进入 `align-tcp-assisted-candidates`）
- 复现条件：`./lab/host/labctl mnt01-selftest` 中 `mnt01-self-tcp4-direct` 或 `mnt01-self-tcp-portmap`；真实双 NAT namespace、MQTT-only、`p2p_network=tcp_only`。
- 期望行为：TCP 打洞整体流程应复用 UDP 已有分层：真正可直连的公网/可路由地址进入 direct path；私网地址若要参与建链，应作为 assisted/private punching input，而不是被记为 direct path 语义。
- 实际行为：TCP gather 会把本机私网 TCP listen 地址放进 `tcp_direct_addrs`，decision 将其下发为 `peer_tcp_direct_addrs`，attempt 随后以 `direct_tcp4` 分支尝试这些私网地址；TCP 没有与 UDP `assisted_addrs` 等价的单独字段或语义层。
- 证据：F-002 最新复测的 MQTT pcap 中可见 `tcp_direct_addrs=["10.0.1.2:5100"]` / `["10.0.2.2:5101"]`，响应中 `peer_tcp_direct_addrs` 保留这些私网地址；同一轮 report 最终成功路径为 `attempt_path=punching_tcp4`。相关代码路径：`connectivity/gather.go` 生成 `tcpDirectCandidates`，`internal/punchdecision/decision.go` 下发 `PeerTCPDirectAddrs`，`connectivity/attempt.go` 先调用 `attemptTCPDirect(..., "direct_tcp4")`，而 UDP punching 使用 `AssistedAddrs` 作为辅助目标。
- 初步判断：
  - 这不是 TCP `+100`、spraying 或 TCP STUN 这些特化规则本身的问题，而是 TCP 流程没有完整对齐 UDP 的地址语义分层。
  - UDP 中 `direct_addrs` 与 `assisted_addrs` 的区分避免了把私网辅助地址误解释成 direct path；TCP 当前把私网 listen 地址放进 `tcp_direct_addrs`，会让测试、诊断和路径命名误以为这是 `direct_tcp4` 覆盖。
  - 该设计缺口会影响 F-002 的测试修正方式：在 TCP assisted 语义明确前，不能把 `tcp4-direct` 简单改成 required direct，也不应把 fallback 成功误记为 direct 验证完成。
- 设计结论：新增 `tcp_assisted_addrs` 语义；私网 TCP listen 地址不得进入 `tcp_direct_addrs`。`direct_tcp4` 只尝试真实 direct candidate；`punching_tcp4` 使用 assisted exact target 与 STUN-derived candidate target。TCP STUN 证据不足但存在 assisted target 时，允许最小 bounded mode0 assisted punching fallback，成功仍记为 `punching_tcp4`。
- 后续动作：进入 `align-tcp-assisted-candidates` change。该 change 同步修正 F-002 相关 case 命名、断言和 direct 覆盖前置条件。
