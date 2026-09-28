## Why

Alpha/POC 的控制面目标是“mesh 优先 + MQTT 兜底”，但在进入可跑的三节点/多节点场景前，需要把**网内转发**做成可控、可诊断、不会放大的最小实现：

- 若签名 transcript 不覆盖 `dst_peer_id`，网络内任一成员可在不破坏签名的前提下重定向消息收件人，导致审计/解释失真。
- 若转发无上界（或缺少去重/限流），容易在异常/攻击流量下形成放大与资源耗尽。
- 若缺少“丢弃/限流事实（facts）”，现场排障将难以解释“为什么没到达/为什么没转发”。

此外，roadmap 对 POC-03 明确要求一个“同一 LAN 的 3 个进程 smoke”，用来验证“网内优先 + MQTT 兜底”不会互相打架（例如双路径重复投递导致重复副作用或错误的 request/response 配对）。

## What Changes

- 控制面 wire format（POC v0）落地：
  - 明文统一 UTF-8 JSON，字段集合/命名冻结。
  - 签名 transcript **覆盖 `dst_peer_id`**；`hop_limit` **不签名**（允许转发递减）。
  - 密文 framing 冻结：`v(1B) || nonce(12B) || ct`，`v=0` 表示 AES-256-GCM。
- bounded flooding（POC 固定 `H=3`）最小实现落地：
  - `hop_limit ∈ [0,3]`；`hop_limit>H` 直接丢弃。
  - 转发仅允许 `hop_limit--` 且必须按原 `dst_peer_id` 转发；不得回传来源邻居。
  - 去重窗口（LRU + TTL）与每 peer 的转发队列上限/丢弃策略（KISS）。
  - 把“限流/丢弃 facts”纳入可解释性输出（用于 smoke/排障）。
- 测试与验证：
  - 单元：签名/验签覆盖 `dst_peer_id`；`hop_limit` 修改不影响签名；密文 framing roundtrip；去重窗口行为。
  - 集成：3 节点模拟（A→B→C）验证 H=3、去重与 drop facts。
  - 真实环境：提供一套**同一 LAN 的 3 个进程 smoke harness**，验证 mesh-first + MQTT fallback 的双路径不会产生重复副作用，并可输出 drop facts。

## Capabilities

### New Capabilities
- `miopunch-poc-control-plane-wire-format`: 控制面明文 JSON 结构、签名 transcript 覆盖规则（覆盖 `dst_peer_id`、不覆盖 `hop_limit`）以及密文 framing。
- `miopunch-poc-control-plane-bounded-flooding`: bounded flooding（H=3）、转发约束、去重窗口、队列上限/限流与 drop facts。
- `miopunch-poc-control-plane-mesh-first-fallback`: mesh 优先 + MQTT 兜底的最小投递策略与“重复投递不产生重复副作用”的约束（用于 LAN 三进程 smoke 的验收口径）。

### Modified Capabilities
- (none)

## Impact

- Go 实现：
  - 新增/扩展 `internal/controlplane/` 下的 wire/crypto/dedup/forwarding 组件（POC v0）。
  - 增加一个可运行的 LAN smoke harness（放到 `tools/miopunch-cp-smoke/`，不污染产品 `miopunch`，也不要求修改 `miopunch-lab`）。
- 测试：
  - `go test ./...` 将新增单元与集成测试（不要求 CI 具备真实 LAN；LAN smoke 为手动可复现脚本/命令）。
- 风险：
  - crypto/wire 一旦冻结会影响后续所有控制面消息族；因此本 change 必须把字段/签名覆盖范围写清楚并用测试锁定。
