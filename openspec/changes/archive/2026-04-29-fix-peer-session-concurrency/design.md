## Context

当前产品主线在 MNT-02（场景 2）中需要覆盖多成员并发与 revoke 边界，但实现上存在单槽位限制：

- MQTT signaling：以 `SID` 为粒度使用固定 topic（`info/* resp/* ready/* start`），同一 `SID` 下的并发 attempt 会互相踩踏。
- dataplane server：QUIC 侧只 accept 一次连接后关闭 listener；KCP 侧将 `UDPConn + raddr + conv=1` 直接绑定成单会话 `net.Conn`，天然不具备 “同一 UDP socket 并发多会话 accept” 的语义。
- acceptor：`serveOnce -> sess.AcceptStream()` 死循环，导致第一个 session 长期占用 acceptor，使后续 peer 难以建立自己的 session。

本 change 以最小化设计复杂度为原则，先把 “并发 attempt 分桶 + 多 session accept/serve + revoke 强语义（被动销毁）” 收敛成可实现的合约与任务，并将 MNT-02 用例从 workaround 拉回到真实验收。

## Goals / Non-Goals

**Goals:**

- MQTT signaling 支持同一 `SID` 下的并发 attempt：引入 `dial_id` 分桶，保证并发 visitor 不会踩踏消息。
- dataplane server 语义支持并发多连接/多会话：QUIC/KCP/TLS 都必须具备 accept loop 语义，不再是 single-shot。
- acceptor 允许同时服务多个 inbound peer transport session：每条 session 独立 goroutine serve streams，acceptor 主循环不被单 session 独占。
- revoke 强语义（语义 A）：节点本地观察到有效 revoke tombstone 后，必须拒绝新操作并主动切断该 peer 的既有会话；不要求主动通知对端。
- MNT-02 selftest 移除 “kill daemon 释放 acceptor” workaround，并用 required case 验收并发与 revoke。

**Non-Goals:**

- 不引入“每个操作都重打洞/重建底层连接”的策略；同一 peer-pair 默认推荐复用一条主 session，操作通过 logical streams 多路复用。
- 不在本轮实现类似 Tailscale 的“在同一 UDP socket 上同时承载 traversal + 主协议并做报文分流”的完整设计；本轮只要求 server 侧 accept/serve 模型与 signaling 分桶正确。
- 不新增额外外部依赖（仍以自建 MQTT broker 作为 required gate 的 signaling 入口）。
- 不承诺与旧版本 topic 结构互通（POC/未发布阶段允许 breaking）。

## Decisions

### 1) MQTT signaling：按 `dial_id` 分桶（同 SID 并发 attempt 不踩踏）

**Decision**

- `dial_id` 取值：直接复用 visitor 的 `NatHoleVisitor.TransactionID`（已存在且对每次 dial 唯一）。
- `SID` 仍按既有规则从 `proxy name + secret` 派生；不新增额外 CLI flag。
- MQTT topic 改为 `SID + dial_id` 分桶：
  - `base = <topicPrefix>/<sid>`
  - `base/attempt/<dial_id>/info/visitor`
  - `base/attempt/<dial_id>/info/client`
  - `base/attempt/<dial_id>/resp/visitor`
  - `base/attempt/<dial_id>/resp/client`
  - `base/attempt/<dial_id>/ready/visitor`
  - `base/attempt/<dial_id>/ready/client`
  - `base/attempt/<dial_id>/start`

**Rationale**

- 同 `SID` 下并发 visitor 的踩踏问题必须通过 topic 分桶解决，不能靠 “延迟/重试/单机锁” 修补。
- 复用 visitor `TransactionID` 避免引入新的 ID 生成规则与字段迁移。

**Alternatives Considered**

- A. 继续使用固定 topic + 在 payload 内携带 transaction_id 过滤：仍会造成 barrier/start 混乱，且对错配的诊断更差。
- B. 每个 attempt 派生一个新 SID：会破坏 SID 的用户心智（proxy/secret），也会放大 broker topic 空间并引入更多状态。

### 2) MQTT 交换顺序：visitor 先发 `info/visitor`，client 被动响应并并发处理

**Decision**

- visitor：生成 `dial_id`，先发布 `attempt/<dial_id>/info/visitor`，然后等待 `attempt/<dial_id>/info/client` 与后续 `resp/* ready/* start`。
- client：持续订阅 `base/#`，当观察到任意 `attempt/<dial_id>/info/visitor` 时，为该 dial_id 启动一个 attempt handler（带超时与清理），在同一 bucket 内发布 `info/client`、`resp/*`、barrier 与 `start`。
- 并发：client 允许同时处理多个 dial_id；每个 dial_id 的 handler 互不共享 topic、互不共享 barrier 状态。

**Rationale**

- client 侧无法预先知道 dial_id；必须由 visitor 先发起。
- 使用 per-attempt handler 可以把状态与超时边界收敛到 attempt 维度，避免全局锁与全局队列相互影响。

### 3) dataplane：server 侧必须具备 “accept loop”，且 per-session serve streams

**Decision**

- QUIC：listener 不得 single-shot；需要长期运行的 listener，并在 goroutine 中循环 `Accept()`，每个连接独立创建 `PeerSession` 并进入 stream serve loop。
- KCP：不再把 `UDPConn` 直接绑定成单个 `kcp.NewConn3(conv=1, raddr, ...)` 会话。改为基于单个 `net.PacketConn` 的 listener 模型（`ServeConn` + `Accept/AcceptKCP`），使同一 UDP socket 可并发接多会话。
- TLS/TCP：保持 `net.Listener.Accept()` 的 accept loop；由上层 acceptor 负责把每条 accepted conn 转为 `PeerSession` 并独立 serve。

**Rationale**

- MNT-02 的并发访问需要 “一个被访问端同时接受多 peer 的 session”，accept loop 是最小必要条件。
- QUIC/KCP 本身具备在单 UDP socket 上并发多连接/多会话的基础能力；当前实现属于错误的 single-shot 适配方式。

### 4) acceptor：从单槽位 serveOnce 改为多 session 并发 serve

**Decision**

- acceptor 的职责拆分为两类循环：
  - “建立 inbound session 的入口循环”：持续处理新 dial_id/新连接。
  - “每条 session 的 serve loop”：在独立 goroutine 内对该 session 循环 `AcceptStream()` 并按 kind 分发到 handler（例如 shell）。
- 禁止在 acceptor 主循环中长时间阻塞在某一条 session 的 `AcceptStream()`。

**Rationale**

- 这是修复 “第一个 session 独占 acceptor” 的根本条件。

### 5) revoke：不主动通知，但本地观察到后必须主动切断既有会话

**Decision**

- 不新增 “revoke push 通知” 的消息类型或通道。
- 语义边界以 “本节点已观察到有效 revoke tombstone” 为准：
  - 从该时刻起拒绝 revoked peer 的新 stream-open/新操作。
  - 同时主动关闭该 peer 的所有既有 peer sessions（close reason 归为 `authorization_revocation` 或等价）。
- 对端只需被动观察到断开/失败；不要求获得专门的 “你已被 revoke” 提示帧。

**Rationale**

- 满足强语义 A 的最小实现：不需要全网原子广播，但本地观察到后必须立刻执行断连与拒绝。

## Risks / Trade-offs

- [Risk] 并发 dial_id 增加 MQTT 消息量与本地 goroutine 数量。  
  → Mitigation: 每个 dial_id handler 必须有硬超时与清理；限制 per-SID 同时活跃 attempt 数（仅作为防护，不作为小硬上限语义）。

- [Risk] topic 结构变更导致旧版本节点无法互通。  
  → Mitigation: 明确本阶段不做兼容；MNT-01/MNT-02 gates 统一升级后验证。

- [Risk] KCP listener 化需要验证与现有 “punching 得到 raddr” 的衔接方式。  
  → Mitigation: 先在设计上约束：KCP session 的 peer 绑定发生在 TLS hello/auth 之后；listener 侧接入后根据实际 remote 进行会话归属（以库提供的 accept 语义为准），并补齐最小单测/集成回归。

- [Risk] revoke “主动切断既有会话” 需要有可靠的 session registry。  
  → Mitigation: registry 只记录已通过 hello/auth 的 verified peer_id；未 verified 的 session 仍按 idle timeout/transport fatal 处理；close reason 可观测。

