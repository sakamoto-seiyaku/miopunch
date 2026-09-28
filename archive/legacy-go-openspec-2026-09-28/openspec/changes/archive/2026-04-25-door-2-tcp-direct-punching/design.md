## Context

Door 2（TCP 打洞）方向已在 `docs/decisions/door-2-tcp-punching-charter.md` 收敛：在不改变既有 POC 上层语义（`ping/sh_attach` 等）的前提下，引入与 UDP 并列的 `TCP direct + punching` 路径，并要求可解释、可观测、可回归。

仓库当前事实基线（实现已部分前置，但未形成 TCP 闭环）：

- wire：已存在可选 TCP 字段（`tcp_direct_addrs/tcp_mapped_addrs/tcp_stun_*` 与 response 的 `tcp_candidate_addrs` 等），但还没有形成从 gather→exchange→attempt 的 TCP 全链路闭环。
- STUN：已有 STUN over TCP client roundtrip；lab 的 STUN server 目前为 UDP-only。
- `connectivity.Gather/Attempt`：仍是 UDP-only（IPv6 direct → IPv4 direct → UDP punching）。
- POC 产品链路（非 lab）：`miopunch ping/sh` 与 acceptor 依赖 `connectivity` + `dataplane`，因此本变更必须覆盖 POC 路径，不是仅改 lab。

约束：

- 遵循 repo guardrails：优先 `miopunch` 命名；避免引入新的 `xtcp` 名称/路径/import。
- 必须支持策略开关与 fail-fast：`p2p_network=auto|udp_only|tcp_only`，并提供短 flag `-u/-t`。

## Goals / Non-Goals

**Goals:**

- 在 `gather → exchange → attempt → dataplane` 现有链路中并列加入 TCP 路径：
  - gather 产出 TCP direct candidates / TCP portmap / TCP STUN 观测
  - coordinator/analyzer 产出 `tcp_punching_enabled/tcp_detect_behavior`（mode0..4）
  - attempt 支持 TCP direct 与 TCP punching（simultaneous-open），并在 mode2/4 允许受控喷射
  - TCP 数据面固定为 `TLS 1.3 + stream`，包含 pinning 身份绑定（mTLS）
- 默认 `p2p_network=auto` 使用固定尝试顺序：`tcp6 → tcp4 → udp6 → udp4`，并允许 `udp_only/tcp_only` 强制限制。
- 在 `tcp_only` 时对 “对端不支持 Door 2 TCP” 做 fail-fast，并给出明确错误。
- lab 增加至少一个覆盖 mode2/4 喷射的回归用例，并保持现有 UDP 回归稳定。

**Non-Goals:**

- 不把 `QUIC/KCP` 承载到 `TCP` 上（它们是 UDP 协议）；TCP 路径不复用 QUIC/KCP。
- 不引入 TURN / relay / 数据面中继。
- 不引入 trickle candidates；继续使用一次性快照 exchange。
- 不做复杂的自动参数调优；v0 以固定护栏默认值为基线，后续用真实样本与 lab 收敛。

## Decisions

### 1) `p2p_network` 作为唯一策略开关（POC + Lab 同步）

**Decision:** 引入 `p2p_network=auto|udp_only|tcp_only`，并在 POC 与 lab CLI 同步落地短 flag：
`-u`=udp_only、`-t`=tcp_only（互斥），外加等价长 flag（例如 `--p2p-network`）。

**Why:** 需要把 TCP 能力带入 POC 产品链路；短 flag 用于常用实验/复现，长 flag 便于配置文件与脚本。

### 2) 能力协商放在 Both：`PeerHello` + NAT-hole exchange

**Decision:** 增加 `capabilities` 并同时放在：

- `PeerHello`（coordinator signaling 路径）
- `NatHoleVisitor/NatHoleClient`（MQTT 与 coordinator 都会走的 exchange 路径）

能力标识使用 `tcp_p2p_v0`。当 `p2p_network=tcp_only` 且任一对端缺失 `tcp_p2p_v0` 时，exchange 阶段直接 fail-fast。

**Why:** `PeerHello` 无法覆盖 MQTT signaling；将 capabilities 同步放进 NAT-hole exchange 才能做到 “tcp_only 明确报错” 的一致口径。

### 3) 端口约定：STUN 用 `P`，listen/punching 用 `P+100`

**Decision (固定为 v0 合约):**

- 选择基准端口 `P`：
  - 若用户 pin `ListenPort>0`：`P=ListenPort`，且 `P`（用于 TCP STUN bind）与 `P+100`（用于 TCP listen/punching）不可用则 fail-fast
  - 若未 pin：优先复用 UDP bind 出来的端口号作为 `P`；要求 `P+100<=65535` 且 `P+100` 可用于 TCP listen/punching，否则探测高端口选择 `P`
- TCP STUN 观测固定绑定本地端口 `P`（避免 NAT 端口变化判断被本机临时端口污染）
- TCP listen/punching 使用 `L=P+100`（双方约定）
- `tcp_mapped_addrs` 记录 STUN 观测到的原始映射端口（`P` 的映射），不做 `+100`
- coordinator 在派生真正用于 dial 的端口（`tcp_candidate_addrs` 与 `tcp_detect_behavior.candidate_ports`）时对 mapped port 做 `+100`；attempt 端不二次偏移

**Why:** 与 charter 对齐，避免复用 “连接过 STUN 的 TCP 源端口” 导致的 FIN/RST 干扰；同时让 NAT 分类在 TCP 下仍可解释。

### 4) TCP gather 扩展：listener + portmap + STUN（best-effort）

**Decision:** `connectivity.Gather` 扩展为在 `p2p_network` 允许 TCP 时并行产出：

- `tcp_direct_addrs`：本机接口地址 + `L=P+100`（按 family 限额去重）
- TCP portmap：对 `L` 做 UPnP/NAT-PMP 的 TCP 映射，映射结果并入 `tcp_direct_addrs`
- TCP STUN：按 endpoint scheme（dual/udp/tcp）过滤，输出 `tcp_mapped_addrs` 与（内置 buckets 下的）`tcp_stun_cn/global`

**Why:** TCP portmap 是低噪音优先策略；TCP STUN 仅用于 punching 可行性评估与 mode 推断，失败应可解释并可跳过。

### 5) Attempt 编排：固定顺序、每个 network 先 direct 再 punching

**Decision:** `connectivity.Attempt` 以固定顺序推进：

- `auto`：`tcp6 → tcp4 → udp6 → udp4`
- `udp_only`：跳过所有 TCP
- `tcp_only`：跳过所有 UDP

对每个 network：

1) Direct（含 portmap candidates）  
2) Direct 失败后，若控制面下发 punching 行为则进入 punching；否则跳过并记录原因

AttemptResult.Path 使用稳定值：
`direct_tcp6|direct_tcp4|punching_tcp4|direct_ipv6|direct_ipv4|punching_ipv4`。

**Why:** 保持可解释的阶段推进；确保 `auto` 下 TCP punching 发生在回退到 UDP 之前。

### 6) TCP punching 基线：simultaneous-open + winner 收敛

**Decision:** TCP punching v0 采用 simultaneous-open（双方 `listen + dial`）：

- socket 复用选项：Unix `SO_REUSEADDR + SO_REUSEPORT`；Windows `SO_REUSEADDR`
- dial 必须绑定到与 listener 相同的本地端口（`LocalAddr.Port=L`），否则不视为 punching（退化成普通 outbound TCP）
- 允许多条连接同时成功；在 TLS pinning 完成后做 winner election 收敛（`SettleWindow` 内收敛到唯一 winner，其它连接关闭）
- election 角色固定：`visitor` 为 leader，`client` 为 follower（与 accept/dial 来源无关）

**Why:** simultaneous-open 是 TCP punching 的核心形态；winner 收敛避免双方各选各的导致最终断链。

### 7) mode2/4 喷射：允许但必须受护栏约束与可解释

**Decision:** `tcp_detect_behavior` 沿用 mode0..4 语义，但 TCP 成本更高：

- mode0/1/3：deterministic（candidate ports / small range guess），不 random 喷射
- mode2/4：允许 `SendRandomPorts`（随机目的端口）与 `ListenRandomPorts`（额外本地端口 listen+dial 复用），但必须满足：
  - 触发理由：analyzer 选择 mode2/4
  - 预算护栏（v0 默认固定）：`MaxConcurrency=64`、`TotalBudget=5s(auto)/10s(tcp_only)`、`DialTimeout=1500ms(auto)/2500ms(tcp_only)`、`SettleWindow=200ms`
  - 初始规模（v0 默认固定）：`SendRandomPorts=128`、`ListenRandomPorts=32`
  - 输出可解释：事件与 report 记录触发原因、预算、实际尝试规模、是否被限额/早停、winner

**Why:** 避免“无理由默认喷射”；同时保留在 hard 场景下的 best-effort 可能性。

### 8) TCP 数据面：TLS 1.3 + pinning（mTLS）

**Decision:** 当选中 TCP 路径时，数据面固定使用 `TLS 1.3 + stream`：

- pinning：用 `HKDF(secret_key, sid, role)` 派生每会话 `ed25519` 证书私钥 seed，生成 self-signed cert
- mutual verify：双方根据对端 `role` 计算期望公钥指纹并校验（mTLS）
- 角色固定：`visitor` 作为 TLS client，`client` 作为 TLS server

**Why:** TCP 需要“加密 + 身份绑定”的安全口径；pinning 避免“TLS 只加密不认证”的 MITM 退化，且不额外扩展 wire 交换字段。

### 9) Lab 回归策略：新增 TCP 用例，同时稳定既有 UDP 用例

**Decision:**

- STUN server 增加 TCP listener（同端口 UDP+TCP）
- 新增至少 1 个覆盖 mode2/4 喷射的用例，并要求 `transport.payload_exchanged`
- 为避免 TCP-first 默认顺序改变既有 UDP 回归：lab runner 的 legacy cases 通过显式 `-u` 固定走 UDP；TCP 用例显式 `-t` 或 `--p2p-network=auto`

**Why:** 回归必须可解释且稳定；TCP 引入不应破坏现有 UDP 基线用例。

## Risks / Trade-offs

- [跨平台 socket 行为差异] → 以 `SO_REUSE*` 的 build-tag 分层实现，lab + 真实样本逐步收敛；v0 护栏默认值偏保守。
- [喷射噪音与资源压力] → mode2/4 必须受 `MaxConcurrency/TotalBudget/DialTimeout` 约束，并强制 early-stop 与可解释输出。
- [TCP STUN 不可用导致误判] → `tcp_only` 下若 TCP STUN 不可用则跳过 punching 并明确报错；`auto` 下可回退 UDP。
- [与现有 UDP-only API 的结构冲突] → `connectivity` 输出与 `dataplane` 边界需显式区分 UDP path 与 TCP stream，避免隐式 nil/类型混用。

