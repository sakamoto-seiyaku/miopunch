# Design: xtcp-connectivity

## Summary

`xtcp-connectivity` 在 `P1 xtcp-kernel` 之上增加一层最小的“连通性编排”：
把 `IPv6 / port mapping helpers / IPv4 punching` 统一为候选来源，并以固定顺序尝试建链。

本阶段刻意不引入 `relay`、不做候选增量更新（trickle）、不做完整端口映射生命周期体系，
只追求：成功率提升 + 失败可解释 + 可回归。

## Control Plane Schema (minimal)

目标：用最小字段改动表达 “direct candidates snapshot”，避免污染 `P1 punching` 的既有字段语义。

- `NatHoleVisitor` / `NatHoleClient`
  - 新增：`direct_addrs []string`
    - 同时承载 `IPv6 direct` 与 `IPv4 portmap direct` 候选
    - 地址格式：`netip.AddrPort.String()`（IPv6 需带 `[]`）
    - 上限：`<= 8`（其中 `IPv6 <= 4`，`direct(v4) <= 4`；`direct(v4)` 可包含多个 `portmap` 候选）
  - 既有 `mapped_addrs/assisted_addrs` 仍然仅用于 `P1 IPv4 punching`（STUN 派生 + LAN assisted），不塞入任何 v6/portmap 候选。
  - `mapped_addrs` 上限建议 `<= 4`；punching 可用最低条件为 `len(mapped_addrs) >= 2`。

- `NatHoleResp`
  - 新增：`peer_direct_addrs []string`（来自对端上报的 `direct_addrs`，一次快照返回）
  - 新增 punching 闸门字段（避免把 “punching 不可用” 当作 exchange 致命错误）：
    - `punching_enabled bool`
    - `punching_error string`
  - 约束：`Error` 仅表示会话级致命错误（鉴权/不存在/协议错误），不用于表达 “STUN 缺失导致 punching 不可用”。

## Architecture (minimal)

### Roles

- **Coordinator**：负责会话编排与信息交换；不承担复杂策略决策。
- **Endpoint (client/visitor)**：负责 `Prepare/Gather`、候选聚合与 `Attempt policy` 执行。

### Phases

1. **Prepare/Gather (exchange 之前)**
   - 采用 `Tailscale` 风格的双 socket：分别 bind `UDP6` 与 `UDP4`。
   - 端口策略为 “尽量同端口（best-effort），但不强制”：
     - 优先尝试配置的本地 UDP 端口（若未配置则为 `0`）
     - 若某一族绑定失败，允许该族回退到 `0`（随机端口）或直接视为该族不可用
     - `UDP4` 绑定失败视为致命（因为 `IPv4 punching` 兜底仍依赖 UDP4）
   - 并发启动：
     - `IPv6` 候选收集（本地接口地址 + 可用性过滤，直接采用 `Tailscale` 的成熟裁剪规则）
     - `IPv4 port mapping`（`UPnP / NAT-PMP`）best-effort（`PCP` deferred）
     - `STUN`（沿用 `P1` discovery；仅在需要回落到 punching 时才依赖其结果；不把 STUN 变成 `IPv6/portmap` 的硬依赖）
   - helper 结果**不阻塞** exchange；只在 gather 窗口内“尽力而为”。
   - 默认预算（可配置）：
     - `gather_timeout = 1.5s`
     - `IPv6 gather` 为本地枚举与过滤（不需要单独网络超时）
     - `portmap` 为 **per-session A 档**：每个 session 的 gather 窗口内并发启动；只要在 `gather_timeout` 内出结果就纳入快照；晚到不纳入本次 exchange（no trickle），但必须完整记录其耗时与错误
     - `STUN` 仅当配置了 STUN server 时才执行，deadline 不超过 `gather_timeout`

#### IPv4 Port Mapping Semantics (P2(v1), A-mode)

目标：只把 `portmap` 作为“当次会话的额外直连候选来源”，不引入进程级长生命周期 socket，也不引入完整续租/重发现/多网关生命周期。

- `portmap` 以“本 session 的 UDP4 listen port（internal port）”为目标端口尝试映射。
- helper **可以产出 0..N 个** `direct(v4)` 候选（例如 `UPnP` 与 `NAT-PMP` 都成功、或同一协议多次尝试得到不同 external port）：
  - 但在 control plane 的 `direct_addrs` 中只携带裁剪后的最多 `direct(v4) <= 4`
  - 去重规则为 `netip.AddrPort` 完全相同去重
- `portmap` 不阻塞 `exchange`：当 `gather_timeout` 到期时，无论 helper 是否完成，都发送当前快照并进入 attempt。
- `portmap` late result 不参与本次会话（no trickle），但必须输出可机读观测事件（例如 `gather.portmap.result`）。
- `lease` 为短租约（经验值）：建议 `lease = min(5m, session_overall_timeout+2m)`。
- 会话结束后 best-effort unmap（失败不影响会话结果，但必须记录到观测事件）。

#### IPv6 Candidate Filtering (TS-style, minimal)

目标：避免把明显不可用的 IPv6 地址（例如 `fe80::`）塞进候选，且对“同子网海量地址”做硬裁剪，保证可复现与可测试。

- 只枚举 `UP` 且非 loopback 的接口地址；跳过明显问题接口（例如 `zt*`、`wt0`）。
- 丢弃：`loopback / unspecified / multicast / link-local`（`fe80::/10`）。
- `Global unicast` 优先；`ULA (fc00::/7)` 只在“没有任何 global IPv6”时才启用。
- 同一接口内、同一子网（由接口 prefix mask 归一）最多保留 `<= 2` 个 IPv6。
- 总 `IPv6` 候选上限为 `<= 4`（落到 `direct_addrs` 的 `IPv6 <= 4` 约束）。
- 候选以 `netip.Addr.Less` 稳定排序，便于测试断言与日志复盘。

参考：`tailscale/net/netmon/state.go:59` (`LocalAddresses`)。

#### UDP Socket Strategy (v4 vs v6, TS-style)

本阶段采用 “两个 socket（`udp4` + `udp6`）” 的工程化做法（socket 不贵），并沿用 `Tailscale` 的端口策略：

- **尽量同端口（best-effort）**：若配置了本地 UDP 端口，则 `udp4` 与 `udp6` 都优先尝试绑定该端口。
- **不保证同端口**：任一族绑定失败时允许回退到 `0`（随机端口）；因此对外交换的 `direct_addrs` 必须携带真实端口（例如 `[2001:db8::1]:54321`），不能假设 v4/v6 共享端口。
- **不做单 socket 双栈复用**：不追求 “一个 dual-stack socket 同时承载 v4/v6” 的技巧，避免跨平台 `v6only/v4-mapped` 语义差异。

参考：`tailscale/wgengine/magicsock/magicsock.go` 的 `pconn4/pconn6` 与 `bindSocket` 端口候选策略。

2. **Exchange (单次快照)**
   - 双端各自把“当前已收集到的候选快照”一次性交换。
   - 明确 no trickle：不会在会话中途追加/更新候选。

3. **Attempt (exchange 之后)**
   - 固定顺序尝试（每步有明确预算/超时）：
     1) `IPv6 direct`
     2) `IPv4 portmap direct`
     3) `IPv4 punching(mode0..4)`（沿用 `P1` 内核作为兜底）
   - 默认预算（可配置）：
     - `attempt_v6_timeout = 800ms`
     - `attempt_portmap_timeout = 800ms`
     - `punching` 沿用 `P1` 的 `DetectBehavior.ReadTimeoutMs` 与 session overall timeout

#### Attempting Multiple `direct(v4)` Candidates

`P2(v1)` 允许 `direct(v4) = 0..N`。为避免 “一个个串行试导致预算被耗尽”，建议采用最小的 fan-out：

- 在 `attempt_portmap_timeout` 窗口内，对所有 `direct(v4)` 候选并发发送轻量握手（复用 `NatHoleSid`），等待第一个成功响应。
- 成功后立即取消其他 attempt，并记录最终选路与取消原因。
- 观测事件必须包含：每个候选的 begin/end、是否收到响应、超时/取消原因。

## Compatibility & Change Boundaries

- 允许修改 `P1` glue code（`control plane` 消息字段、peer 端编排、事件输出）以接入新候选与 attempt policy。
- `xtcp/nathole` 视为 `P1` punching kernel 行为基线：
  - 除非是可证明的 bug fix 或必要的扩展点，否则不应改动其算法与默认行为。
  - 任意修改必须通过 `P0` 实验台回归证明 `P1` 既有矩阵结果不漂移。

## Observability Contract (P2 additions)

在 `P1` 阶段化事件流基础上，新增并稳定以下事件族：

- `gather.*`：`v6`、`stun`、`portmap` 的开始/结束、耗时、错误与产出候选摘要。
- `exchange.*`：本次发送/接收的候选快照摘要（数量、类型、是否缺失、时间戳/版本）。
- `attempt.*`：按候选记录 begin/end、超时/取消原因、最终选路与回退路径。

约束：事件必须可机读，测试能够断言阶段/顺序/耗时/失败原因。

## References (engineering, not copy)

- `Tailscale`：candidate/endpoint 聚合、port mapping helper 作为非阻塞候选来源、双栈优先级思想（不引入其 relay）；本阶段优先只借鉴其策略与诊断表达，不直接移植其 `net/portmapper`（除非后续证据证明最小实现不够用）。
- `MiniUPnP`：实验台侧的 `UPnP + NAT-PMP + PCP` 行为基线与错误码参考。
- `go-libp2p`：端口映射生命周期（本阶段不实现完整生命周期，但可借鉴错误分类与恢复思路）。
- `pion/ice`：candidate gather/priority 的经验（本项目不在 `P2` 转向完整 ICE）。
