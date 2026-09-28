## Why

当前 `miopunch` 的 POC 主路径是 `UDP punching` + `QUIC/KCP over UDP` 数据面。在真实网络里，`UDP` 可能被封锁、被 QoS 限速或不稳定，而 `TCP`（尤其常见端口策略下）更可能可达；因此需要一条与 UDP 并列、可解释且可回归的 `TCP 直连 + 打洞` 路径来提升可用性。

## What Changes

- 新增 `p2p_network=auto|udp_only|tcp_only` 策略开关，并在 `auto` 下采用固定优先级：`tcp6 → tcp4 → udp6 → udp4`。
- 在 **POC 产品链路与 lab** 同步加入短 flag：`-u`（udp_only）与 `-t`（tcp_only），并提供等价长 flag（例如 `--p2p-network`）。
- 能力协商（fail-fast）：
  - 在 `PeerHello` 与 NAT-hole exchange（`NatHoleVisitor/NatHoleClient`）中携带 `capabilities`，引入 `tcp_p2p_v0`。
  - 当 `tcp_only` 且对端不支持 `tcp_p2p_v0` 时，exchange 阶段直接给出明确错误（避免黑盒 attempt 超时）。
- `connectivity.Gather` 扩展为同时产出 TCP 侧候选与观测（best-effort）：
  - `tcp4/tcp6` listener + direct candidates
  - TCP portmap candidates（UPnP/NAT-PMP）
  - TCP STUN 观测（固定本地端口 `P`，并遵循 `tcp://`/`udp://`/dual scheme 过滤）
- 控制面（coordinator）产出 TCP attempt 可执行信息：
  - `tcp_punching_enabled/tcp_punching_error`
  - `tcp_detect_behavior`（沿用 mode0..4 语义；mode2/4 允许喷射但预算更小且需可解释）
  - 端口约定：TCP STUN 使用 `P`，TCP listen/punching 使用 `P+100`（候选端口在控制面收敛后下发，attempt 端不二次偏移）
- `connectivity.Attempt` 扩展为 TCP direct + TCP punching（simultaneous-open）：
  - winner 收敛（settle window）与跨平台资源护栏（MaxConcurrency/TotalBudget/DialTimeout/SettleWindow）
- 新增 TCP 数据面：`TLS 1.3 + stream`（不把 QUIC/KCP 扛到 TCP）：
  - pinning（mTLS）：`HKDF(secret_key, sid, role)` 派生每会话 `ed25519` 证书密钥并校验对端指纹，不新增额外 wire 字段。
- lab 增强（不是仅改 lab）：
  - STUN server 支持 STUN over TCP
  - 新增至少一个覆盖 `mode2/4` 喷射的回归用例，并要求 `transport.payload_exchanged` 证据链。

## Capabilities

### New Capabilities
- `miopunch-tcp-p2p-v0`: 定义 Door 2 的 `TCP direct + punching + TLS stream dataplane` 能力边界、默认顺序、端口约定（`P`/`P+100`）、mode2/4 喷射护栏与可解释输出，以及 `tcp_only` fail-fast 口径。

### Modified Capabilities
- `xtcp-connectivity`: 从 “P2 UDP-only” 演进为 “受 `p2p_network` 控制的 UDP/TCP 并列尝试”，并更新固定 attempt 顺序描述。
- `miopunch-mqtt-signaling`: exchange 的 program-defined information 扩展为包含 `capabilities` 与 `p2p_network`（以及 TCP 相关字段），并允许 attempt 产出 TCP 或 UDP path。
- `miopunch-dataplane`: 从 “post-connectivity UDP path” 扩展为 “post-connectivity path（UDP 或 TCP）”，明确 TCP=TLS stream，且仍要求可机读的 payload evidence。

## Impact

- Affected code (high-level):
  - `connectivity/`（gather/attempt、TCP listener、TCP STUN、TCP portmap、network policy）
  - `internal/wire/`（capabilities、p2p_network、tcp_detect_behavior 下发）
  - `internal/coordinator/`（TCP analyzer + 行为下发 + `tcp_only` fail-fast）
  - `dataplane/`（新增 TLS stream 路径；保留现有 QUIC/KCP over UDP）
  - POC product path：`internal/task/*`、`internal/pocacceptor/*`、`cmd/miopunch/*`
  - Lab path：`cmd/miopunch-lab/*`、`stun/*`、`lab/guest/cases/*`、`lab/guest/bin/mlab-xtcp-run`

