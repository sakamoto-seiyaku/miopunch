# peer transport session 并发与 revoke 强语义讨论记录（2026-04-28）

> 状态：讨论结论落盘（design note），不是实现提交。  
> 目的：把 MNT-02 讨论中暴露的 “acceptor 只能服务第一个 peer transport session” 大问题写成可追踪的设计上下文，避免上下文丢失；后续将收敛为 OpenSpec change + 代码修复 + 测试用例补齐。  
> 约束：本文件以现有仓库实现为参照（便于定位根因），但不强行承诺具体函数签名；实现细节以未来 change 为准。

## 1. 问题表述（为什么这是 blocker）

当前实现存在一个“单槽位 acceptor”问题：**第一个连上来的 peer transport session 会长期占用 acceptor，使得后续其它 peer 很难再建立自己的 session/stream**。

这会直接破坏 MNT-02/主线网络的基本假设：

- 一个节点应能同时被多个远端节点访问（规模上不应有小硬上限；若存在资源上限，也必须是很大且可配置/可解释的值）。
- 横向访问不应因为“谁先连上”而被永久饿死。

## 2. 结合场景的复现示例（p3 连接不上 p1）

以 3 节点为例：

- `p1`：被访问端（acceptor / issuer / member 都可能扮演该角色）
- `p2`：先发起访问的成员
- `p3`：后发起访问的成员

步骤：

1. `p2 -> p1` 先执行一次 `ping` 或 `sh`，建立 dataplane session，并保持进程常驻。
2. 随后 `p3 -> p1` 再发起 `ping` 或 `sh`。

观察到的失败模式包括：

- `p3` 侧长时间超时 / 卡在建链阶段（即使 `p2-p1` 此刻并未“主动阻塞”）。
- 或者 `p3` 的信令阶段发生竞争/踩踏，表现为不可解释的握手失败。

结论：这不是“并发压力大导致性能差”的问题，而是模型上只允许一个 inbound session 常驻的结构性缺陷。

## 3. 根因（当前实现的三个硬限制点）

### 3.1 `pocacceptor` 的生命周期模型：serveOnce 永不返回

`internal/pocacceptor/acceptor.go`：

- `Run()` 周期性读取 state，然后调用 `serveOnce(...)`。
- `serveOnce(...)` 在成功建立 `dataplane.PeerSession` 后进入：
  - `for { sess.AcceptStream(ctx) ... }`

该循环不会自然退出，因此 `Run()` 无法回到外层循环再次处理其它 peer 的握手/建 session。

也就是说：**只要第一个 session 活着，acceptor 就不会再进入下一次“接新 peer”的逻辑**。

### 3.2 QUIC server 只 accept 一次连接

`dataplane/session_transport.go` 的 `serveQUICSession(...)`：

- `quic.Listen(...)` 后只执行一次 `ln.Accept(ctx)`。
- accept 成功后立刻 `ln.Close()`。

这在语义上等价于：**QUIC 监听端一次只接一个连接**。即使上层 acceptor 被改成可并发，底层也会把并发连接拒掉/饿死。

### 3.3 MQTT signaling 的 topic 以 SID 为粒度，只提供一个“握手槽位”

`internal/signaling/mqtt/session.go`：

- `baseTopic = <topicPrefix>/<SID>`
- 使用固定子 topic：
  - `info/client` / `info/visitor`
  - `resp/client` / `resp/visitor`
  - `ready/client` / `ready/visitor`
  - `start`

这意味着：同一个 `SID` 下，所有 visitor/client 的尝试都落在同一组 topic 上。

一旦存在并发 visitor（例如 `p2` 与 `p3` 同时或相近时间访问 `p1`）：

- 消息会互相覆盖/抢占（先来的 resp 可能被后来的尝试消费）。
- barrier/start 的语义也会被破坏（“谁的 start 是哪个 attempt 的 start”无法区分）。

结论：**信令层也默认“同 SID 同时只有一个 attempt”**。

## 4. 已确认的设计目标（讨论结论）

### 4.1 session 并发与上限

- 单端同时被访问的 peer transport session 数量：不设置小硬上限。
- 允许同时存在多条 session（包括同一 peer 的多条 session，不做“强制唯一”的硬约束）。
- 真实上限只来自资源（fd、内存、CPU、带宽）与显式的预算控制（例如配置项/限流/idle timeout），并且需要可诊断。

同时，我们确认 **不引入“每个操作都重建底层连接/重打洞”** 的复杂设计：

- 对同一 `peer-pair`，默认推荐只有一条“主 session/载波”（坏了再重建）。
- `ping/sh/...` 这类操作一律在 session 内通过 logical stream 多路复用完成（QUIC native streams 或 TLS+yamux）。
- “一条 session 足够”是推荐路径与默认实现策略，而不是对外协议强制的硬限制。

### 4.2 revoke 语义选择

我们选择 revoke 的 **语义 A（强）**，并且口径是：

- “从此刻起不应再能继续使用这个网”。

这至少要求：

1. **新建** dataplane stream / 新发起操作：应被立即拒绝（在本机视图已包含 revoke tombstone 的前提下）。
2. **已存在** 的 peer session / 已建立的 stream：需要被主动终止（不能靠 idle timeout 自然拖延）。

> 需要进一步明确的边界（后续 change 中要写清楚）：  
> “此刻”在分布式系统中的可验收定义，通常应落在“某节点已观察到有效 revoke tombstone 的时刻”，而不是幻想全网原子同时生效。

## 5. 设计改造方向（准备进入 change 的内容）

### 5.1 signaling：引入 attempt 维度（dial_id / transaction_id）进行多路复用

目标：同一个 `SID` 下允许并发 attempt，不互相踩踏。

方向：

- 引入 `dial_id`（可直接复用 visitor 已有的 `TransactionID`，或在其基础上再派生）。**同一 `SID` 下必须按 `dial_id` 分桶**，否则并发 visitor 会踩踏 topic。
- MQTT topic 从 “SID 固定槽位” 改为 “SID + dial_id 分槽位”，示例：
  - `<prefix>/<sid>/attempt/<dial_id>/info/{visitor,client}`
  - `<prefix>/<sid>/attempt/<dial_id>/resp/{visitor,client}`
  - `<prefix>/<sid>/attempt/<dial_id>/ready/{visitor,client}`
  - `<prefix>/<sid>/attempt/<dial_id>/start`
- acceptor 侧订阅 wildcard（例如 `attempt/+/info/visitor`），每个 `dial_id` 生成一个 attempt handler（带超时与清理）。

### 5.2 transport：server 侧必须“可多连接 accept”

以 QUIC 为首要修复点：

- listener 生命周期应是常驻的（直到 daemon 退出或显式关闭）。
- `Accept()` 必须循环执行，并为每个 conn 生成独立 `PeerSession`。
- 每个 `PeerSession` 自己处理 stream accept loop（goroutine per session）。

TCP/TLS/KCP 侧也要确保 “serve one session” 的抽象不会在 acceptor 层面造成单槽位（要么返回 server 抽象，要么 acceptor 在上层管理多个 session）。

### 5.3 acceptor：从 “serveOnce” 改为 “长期运行的多 session acceptor”

最小目标是把现有结构从：

- “握手一次 -> 建一个 session -> 永远阻塞在 AcceptStream()”

改成：

- “握手/建 session 的入口能持续处理新 peer”
- “每条 session 在自己的 goroutine 内 serve streams”

### 5.4 revoke：需要 session registry + 事件驱动的主动断开

方向：

- 在 peer identity 被 hello/auth 可信化之后，把 session 登记到 registry（`peer_id -> []session`）。
- 当本机视图应用到新的 `revoke_member` tombstone 时：
  - 立刻关闭该 peer 的所有 session/streams（close reason 标记为 `auth_revoked` 或等价语义）。

## 6. 对测试与验收的影响（MNT-02 需要新增/收紧）

在 MNT-02（场景 2）里，至少需要一条 required 证据覆盖：

- `p1` 同时/先后服务 `p2` 与 `p3` 的 `ping/sh`，两者均能成功完成（并且不需要“先断开 p2 才能让 p3 连上”）。

以及 revoke 强语义的 required 证据覆盖：

- revoke 生效后，revoked 节点的后续 `ping/sh` 必须失败（已有要求）。
- 并且被撤销节点的**已有会话**不应继续可用（需要明确可验收动作，例如已有 `sh_attach` 被强制断开、或已有 session 无法再 open 新 stream）。
