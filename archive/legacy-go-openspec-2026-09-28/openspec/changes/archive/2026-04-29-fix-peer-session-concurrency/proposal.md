## Why

当前主线产品路径在 “同一被访问端同时被多个 member 访问” 时会出现结构性失败：第一个连上来的 peer transport session 会长期占用 acceptor，导致后续 peer 很难再建立自己的 session/stream。MNT-02 selftest 目前不得不用“kill 已被 revoke 的 member daemon 以释放 acceptor”的 workaround 才能继续验证其它 member，这使得场景 2 的并发与 revoke 语义无法被正确验收。

根因同时来自三层：

- MQTT signaling 以 `SID` 为粒度使用固定 topic，缺少 attempt 维度分桶；并发 visitor 会踩踏消息。
- dataplane 的 server 侧（尤其 QUIC/KCP）存在 “只 accept 第一条连接/会话” 的 single-shot 行为。
- acceptor 的生命周期模型是 “serveOnce -> 单 session AcceptStream 死循环”，天然单槽位。

本 change 需要把这些并发与 revoke 强语义边界收敛为明确的 spec 合约与实现任务，作为进入更复杂主线网络测试（MNT-03）的前置修复。

## What Changes

- MQTT signaling：
  - 引入 `dial_id`（复用 visitor `TransactionID`）并将同一 `SID` 下的 exchange/barrier/start 按 `dial_id` 分桶，允许并发 attempt。
  - visitor 先发布 `info/visitor`，client 侧按 bucket 响应，避免 “client 先发导致无法绑定 dial_id” 的歧义。
- dataplane：
  - server 侧必须支持并发接受多个 inbound peer transport session：
    - QUIC：listener 接受连接应为 accept loop（不再只 accept 一次后关闭）。
    - KCP：使用 listener/accept 模型而不是把 UDPConn 直接绑成单会话 KCP conn。
    - TLS/TCP：保持 accept loop 语义，并确保不会被单 session serve 模型独占。
  - 明确默认策略：同一 peer-pair 推荐复用一条主 session，`ping/sh/...` 通过 logical streams 多路复用；不以 “每个操作重打洞/重建底层连接” 为目标。
- revoke：
  - 不引入主动通知通道。
  - 一旦节点本地观察到有效 revoke tombstone，必须拒绝 revoked peer 的新操作，并主动切断其既有会话（对端被动观察到断开/失败即可）。
- MNT-02：
  - 删除当前 selftest 中的 “kill member daemon 以释放 acceptor” workaround。
  - 新增/收紧 required 用例：在已有 member 会话存在时，另一个 member 仍能成功 `ping/sh` 同一被访问端；revoke 后 revoked member 被拒绝且会话被切断，非 revoked member 不受影响。

## Capabilities

### New Capabilities
- (none)

### Modified Capabilities
- `miopunch-mqtt-signaling`: 追加 “同一 `SID` 下按 `dial_id` 分桶，允许并发 attempt” 的 requirement（topic 语义与并发保证）。
- `miopunch-dataplane`: 追加 “server 侧必须支持并发多 peer session accept/serve” 与 “revoke 观察到后主动切断既有会话” 的 requirement（覆盖 QUIC/KCP/TLS）。

## Impact

- Affected code (expected):
  - `internal/signaling/mqtt`（topic 派生、wait/publish、并发 demux）
  - `dataplane`（QUIC/KCP/TLS server accept 语义、session 生命周期）
  - `internal/pocacceptor`（从单槽位 serveOnce 模型转为可并发 serve 多 session）
  - `lab/guest/bin/mlab-mnt02-run`（case 调整，去除 workaround，新增并发/强 revoke 验收）
- Backwards compatibility:
  - MQTT signaling topic 结构变化，默认不要求与旧版本节点互通（POC/主线未发布阶段允许 breaking）。
- Dependencies:
  - 不引入新的外部服务依赖；继续以自建 MQTT broker 作为 required gate 的唯一 signaling 依赖。

