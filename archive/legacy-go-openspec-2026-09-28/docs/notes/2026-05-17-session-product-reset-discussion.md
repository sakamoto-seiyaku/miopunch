# 2026-05-17 Session Product Reset Discussion

> 状态：滚动维护的讨论记录；用于约束“面试版最小闭环分支”的 Hard-Min 设计，不是 OpenSpec change。
>
> 目的：为一个新的产品分支记录“为什么要重做最小闭环”和“第一刀先切哪里”。
>
> 边界：本文不是正式 OpenSpec change，不是定稿设计，也不是立即实现清单。本文只记录已经讲清楚、后续必须反复对照的核心约束。
>
> OpenSpec 组织方式（避免跑飞）：可以一次性创建多个 change 的“壳”（stub：只写 scope/owned paths/done，不细化实现），但只允许一次 apply 一个 change；每个 change 完成后立刻 verify+archive，再补写下一个 change 的细节。

## 收敛版摘要（Hard-Min + 面试版 POC 契约）

下面这一段是本文的“可执行结论”。其余章节更多是背景、现状拆解、以及为什么要这样收敛。

### POC 必须可演示的闭环

固定闭环（GUI 必须能带着跑）：

- `CreateNetwork` / `JoinNetwork`
- 设备发现（看到 peer 列表、状态）
- NAT/STUN 探测与候选交换
- `UDP` punching 建路（path establishment）
- 数据面 session recipe（默认：`UDP + KCP + TLS 1.3 + yamux`）
- `ping`
- `shell`

### Rule of One（最小闭环每条轴只保留一种实现）

- path establishment：只做 `UDP punching`
- session recipe：只做 `UDP + KCP + TLS 1.3 + yamux`
- 控制面投递：只做 `MQTT mailbox`（bootstrap + punching 协商期）
- 控制面 E2E：只做 `peer_e2e_v1`（sign-then-encrypt；recipient-only；`Ed25519` + `X25519`）

### v1 控制面约束（必须收敛）

- v1 message kinds 只允许：`join_request`、`enroll_response`、`dial_offer`、`dial_answer`。
- MQTT topic：`join_topic/reply_topic` 随机不可猜；批准后 `topic_prefix = mp/v1/net/<net_root>`（由 `mailbox_secret` 派生），`inbox/presence` 挂在其下。

### 主干不变（可接回点）

主干稳定为：

`PathResult -> SessionRecipe -> PeerSession -> LogicalStream`

后续接回 QUIC/TCP/relay/mesh/topology 的方式应是“新增 capability 或新增 recipe”，而不是改写主干。

### OpenSpec changes（先建壳；一次只 apply 一个）

建议用 OpenSpec 把这次重组拆成少量纵切 changes。可以先把全部 change 建“壳”（stub），但实现时只允许一次 apply 一个 change：做完就 verify+archive，再进入下一个。

建议顺序（kebab-case）：

- `poc-v1-01-controlplane-wire`
- `poc-v1-06-persistence`
- `poc-v1-02-enroll-bootstrap`
- `poc-v1-03-presence-discover`
- `poc-v1-04-dial-punch`
- `poc-v1-05-secure-session`
- `poc-v1-07-gui-wizard`

### Hard-Min 安全与身份边界（面试可讲清楚）

- CreateNetwork 默认生成：`network_id`（随机）、`authority_ed25519_keypair`（唯一签发 key）、`mailbox_secret`（随机 32B，仅用于派生 `net_root/inbox` topic 降低枚举/猜测；不用于正文机密性）。
- 设备首次启动默认生成：两对长期 key（策略 A）
  - `Ed25519`：身份与签名；`peer_id` 一律由 `ed25519_pub` 推导。
  - `X25519`：控制面 E2E 收件人公钥（不从 Ed25519 派生）。
- 入网凭证：`MemberCredential`（authority Ed25519 签发；只绑定公钥，不存 `peer_id`；短有效期）。
- 控制面消息：peer-targeted 统一 outer/inner envelope；outer `src` 不可信；安全语义以 inner 解密后验签为准。
- 数据面：每个 peer session 都必须有强身份绑定的 secure channel（KCP 上显式 pinned `TLS 1.3`；QUIC 作为后加 recipe 时使用其内置 `TLS 1.3` 握手与身份校验）。

## 背景

当前主线已经不再只是一个“极简、可解释的 `join -> ping -> sh` 产品闭环”。

现状里已经叠在一起的东西包括：

- 入网与成员管理
- MQTT mailbox / broker reachability / broker pinning
- punching / candidate / attempt / topology / recovery
- KCP / QUIC / TLS + yamux transport recipe
- daemon / LocalAPI / desktop runtime state / GUI bridge

这些能力本身不是都错了，但它们现在经常被捆成一个大的主流程。结果是：

- 一个简单功能需要穿过过多环节；
- 一处改动会牵动过多模块；
- 作者本人很难完整、稳定地向别人讲清楚“系统到底是怎么工作的”；
- GUI 只是把这种复杂性照了出来，并不是复杂性的根源。

因此，这次不是单纯重做 GUI，也不是在现有主线上继续修补，而是考虑从当前主线剥离一个新的产品分支，重新收缩为“作者自己完全能讲明白”的最小闭环版本。

## 这个新分支的硬目标

新分支首先服务这几个目标：

- 作者自己能看懂。
- 作者能把主流程、关键数据、关键安全边界完整讲给别人听。
- 产品闭环必须极简，但不能牺牲已经确定需要保留的安全与身份边界。
- 架构要允许后续增量加回能力，而不是每次加能力都回头重改主干。

当前预期的最小闭环仍然是：

- 启动
- 建网或入网
- 看到设备
- `ping`
- 打开 shell

比这个闭环更外层的能力，在新分支里都应视为后加项，而不是主流程的默认组成部分。

## 第一刀：先把“路径建立”和“会话协议”分开

当前讨论确认的第一刀，不是先拆 GUI，也不是先拆 task，而是先拆以下两个概念：

### 1) 路径建立（path establishment）

路径建立回答的是：

- 这次是走 `UDP` 还是 `TCP`。
- 这条路径是 `direct`、`punching` 还是将来的其它手段。
- 这条路径背后的 socket / listener / conn 资源是谁持有、谁负责关闭。
- 将来若加入新的 path 手段，例如：
  - UDP 多端口
  - fake TCP（UDP 伪装成 TCP）
  - 其它新的打洞/建路方法
  它们都应该优先落在这一层。

这一层的输出不应直接等于“最终跑什么协议”。

### 2) 会话协议（session protocol / session recipe）

会话协议回答的是：

- 在已经可用的 path 上，如何升级成可复用的 peer session。
- 在该 session 上，如何打开 logical stream。

当前已知 recipe 至少包括：

- `UDP path -> KCP -> TLS 1.3 -> yamux`
- `UDP path -> QUIC`
- `TCP path -> TLS 1.3 -> yamux`

关键约束：

- `TCP` 只是 path/carrier，不应被编码成“天然就是 TLS+yamux”。
- `UDP` 只是 path/carrier，不应被编码成“天然就是 KCP 或 QUIC”。
- 后续新增 `TCP` 下的新协议时，不应回头改 path establishment。
- 后续新增 `UDP` 下的新协议时，也不应回头改 path establishment。

## 当前代码里已经暴露出来的真实耦合点

### 1) `connectivity.AttemptResult` 已经在暗示“路径结果”

当前 `connectivity.AttemptResult` 已经部分体现了这条边界：

- UDP 路径返回 `Conn + Remote`
- TCP 路径返回 `TCPConns`

这说明 repo 里已经隐约存在“先得到 path result，再决定上层如何升级”的结构基础。

### 2) `internal/task/poc_dial.go` 目前承担了过多职责

`dialPeerStream` 现在同时负责：

- gather
- MQTT exchange
- punching attempt
- UDP owner / traversal demux
- KCP / QUIC / TLS session 选择
- 资源所有权转移
- logical stream 打开

这说明真正的耦合不是 `PeerSession` 抽象本身，而是“task 编排层直接在拼 transport recipe”。

### 3) `dataplane.PeerSession` 是现有代码中值得保留的上边界

`dataplane.PeerSession` 已经把上层操作约束成：

- `OpenStream`
- `AcceptStream`
- session lifecycle

这是当前代码里一个相对健康的切口。后续 `ping`、`sh` 等上层语义应该继续只依赖这个边界，而不应继续知道：

- 是 KCP 还是 QUIC
- 有没有 KCPOwner / QUIC transport
- path 的底层资源怎么关

## 新分支里的最小闭环建议

为了避免一开始就掉进“为了通用而通用”的坑，新分支的第一版可以非常克制：

- path 只保留 `UDP`
- session recipe 只保留 `KCP -> TLS -> yamux`
- 上层只跑最小业务闭环：`join -> ping -> sh`

这个最小版本的价值不是“永久只做 KCP”，而是：

- 先把 path establishment 和 session recipe 的边界立住；
- 先证明上层闭环不需要知道底层 recipe 细节；
- 以后加 `UDP + QUIC` 或 `TCP + *` 时，只是新增 recipe，而不是回头改主流程。

## 避免过度抽象的约束

当前讨论明确不希望走向一套为了抽象而抽象的框架。新分支在这块应遵循：

- 先用具体 struct 和明确 support matrix，不先发明一堆 registry/factory/plugin 黑话。
- 先让“现在只有一条 recipe 的最小闭环”跑通，再讨论多实现并存。
- 接口只放在真正被消费的位置；不要为了未来想象中的实现数目提前造接口。
- 能用一个清晰的数据结果表达的，就不要先升成层层嵌套的抽象层。

目前更像是需要这样的切口：

`PathResult -> SessionRecipe -> PeerSession -> LogicalStream`

而不是一开始就构造一个过度泛化的“TCP/UDP × 各种 protocol adapter”体系。

## 当前讨论的落点

今天先把下面几点固定住：

- 新分支的首要目标不是 feature parity，而是“作者自己能讲明白的最小闭环”。
- 第一刀先拆 `path establishment` 和 `session protocol`。
- `TCP` / `UDP` 与 `TLS` / `KCP` / `QUIC` 不是同一条抽象轴。
- `PeerSession` 可以继续作为上边界保留。
- `internal/task/poc_dial.go` 是后续重组时必须重点拆解的汇合点。

后续讨论可以继续围绕两个具体问题往下走：

1. `PathResult` 在新分支里最小应该长成什么样。
2. `SessionRecipe` 的第一版如何只支持 `UDP + KCP`，同时不给未来的 `TCP + *` 或 `UDP + QUIC` 留下坏边界。

## 原有复杂能力不是废弃，而是后加 capability

当前讨论又进一步明确了一点：

- 主线里现有的复杂能力并不是“没有意义”；
- 问题不在于它们存在，而在于它们太早进入了默认主流程；
- 新分支的目标不是否定这些能力，而是先把最小闭环收缩出来，然后给这些能力保留清楚的接回点。

换句话说，新分支不应该做成一个“只能跑 demo、以后一加功能就得重写”的死路版本。它应该做成：

- 第一版只保留最小闭环；
- 复杂能力以独立 capability group 的方式后加；
- 后加时尽量不改 `path establishment -> session recipe -> peer session` 这条主干。

## MQTT 在现状中实际上承担了三类职责

结合当前代码，MQTT 现在至少承担三段不同职责，而且这三段不是同一件事：

### 1) invite / join / approve mailbox

这一段主要体现在：

- `internal/task/invite.go`
- `internal/task/join.go`
- `internal/task/approve.go`
- `internal/task/mqtt_invite.go`

它负责的是：

- 用 invite code 建立第一次控制面接触；
- 交换 join request / approval response；
- 把 membership bundle、broker effective、seed peer 等初始控制面数据发给新成员。

这是一套“入网控制面 mailbox”。

### 2) punching 期间的实时 exchange / barrier

这一段主要体现在：

- `internal/task/poc_dial.go`
- `mqttsig.Open(...).RunVisitor(...)`

它负责的是：

- 在打洞前后交换 candidate / detect behavior / barrier；
- 为 path establishment 生成一次性的协商结果；
- 让后续的 UDP/TCP 尝试知道该怎么打。

这是一套“建链协商面”。

### 3) post-join 的 peer relay / mesh / bootstrap expansion

这一段主要体现在：

- `internal/controlplane/forwarder.go`
- `internal/task/bootstrap_more.go`
- `internal/task/maintain_neighbors.go`
- `internal/task/topology.go`

它负责的是：

- peer 不直连时，用 MQTT 或 neighbor 去转发控制消息；
- 用 `bootstrap_more` 扩展候选集合；
- 维护邻居、做 topology 选择和恢复；
- 让系统逐渐从“一个点对点闭环”长成“一个更像 overlay 的网络”。

这是一套“overlay 扩展面”。

## 最小闭环里应保留和移出的东西

当前更合理的切法，不是把 MQTT 整体删掉，而是把它拆成“最小闭环必须保留的”和“后加 capability”。

### 最小闭环建议保留

- `invite / join / approve` mailbox
- punching 期间的一次性 realtime exchange
- 单 peer 的 `ping -> sh`

因为这三者合起来，已经足够形成一个：

- 能建网/入网
- 能拿到 seed peer
- 能完成一次 direct path establishment
- 能升级成 session 并打开业务 stream

的完整产品闭环。

### 最小闭环建议先移出默认主流程

- `bootstrap_more`
- `maintain_neighbors`
- 基于 neighbor 的 mesh forward / bounded flooding
- “peer 直连失败后继续走 overlay/relay 扩展”的默认恢复路径
- 拓扑诊断驱动的长期 neighbor 维持逻辑

这些能力不是删除，而是先不把它们当作“一个简单 shell 应用天然必须带着跑”的主路径。

## 这些能力未来接回时，应该接在哪里

当前最重要的约束不是“未来都能加”，而是“未来加回时不要把最小主干重新搅乱”。

因此现在更合理的接回方式是：

- `path establishment` 这一刀只负责把底层 carrier 建出来；
- `session recipe` 只负责把 carrier 升级成 `PeerSession`；
- overlay / mesh / bootstrap / relay 不要再回头侵入这两刀；
- 它们应该作为更外层的 peer-reachability / control-plane capability 来接。

也就是说，后面如果要把原来的复杂系统接回来，更像是新增：

- 更复杂的 peer selection
- 更复杂的 control-plane routing
- 更复杂的 recovery strategy

而不是重新改写：

- UDP/TCP path establishment 的边界
- KCP/QUIC/TLS session recipe 的边界
- `PeerSession -> OpenStream` 这条上层调用路径

## 最小闭环的安全设计应收敛成一条可解释的密钥链

> 注：本节包含一些“更完整/更泛化”的早期讨论点，其中若与后文的 **Hard-Min 收敛版** 冲突，应以 Hard-Min 为准。
> 特别是：Hard-Min 已将“控制面消息机密性依赖 net_secret / group AEAD”的叙事收敛为“peer-targeted E2E（X25519）+ mailbox_secret 仅用于 topic 派生”。

当前代码里的安全设计并不是完全混乱，但它被分散在多条流程里，不容易讲清楚。结合现状，可以把它理解成三把不同职责的钥匙：

### 1) invite 钥匙：只负责第一次入网 bootstrap

当前体现为：

- `internal/controlplane/invite_crypto_v0.go`
- `SealInviteJoinRequestV0`
- `SealInviteMembershipBundleV0`

它负责两件事：

- join request 用 `invite_secret + invite_topic` 做 AEAD；
- membership bundle 用 issuer/member 的 `X25519` 共享密钥再结合 `invite_topic` 做 AEAD。

这把钥匙的职责应该非常窄：

- 只用于“我还没入网，但我要安全地完成第一次加入”；
- 一旦 membership bundle 成功落地，这把钥匙就退出主流程。

### 2) net 钥匙：只负责入网后的控制面

当前体现为：

- `internal/controlplane/group_wrapper.go`
- `internal/controlplane/inbox_topic.go`
- `internal/controlplane/sign.go`

它负责的是：

- 用 `net_secret` 派生 inbox topic；
- 用 `net_secret` 做 group AEAD；
- 用设备自己的 `Ed25519` 对控制消息签名。

这意味着“现状里”的控制面语义更像是：

- MQTT 只是 transport；
- broker 只看到 opaque ciphertext 和导出的 topic；
- 控制消息机密性依赖 network-scope 的 `net_secret`（group AEAD）；
- 控制消息发送者身份来自 `Ed25519` 签名。

而在 Hard-Min 新分支里，这段语义被收敛为：

- broker/relay 不定义安全语义，只负责投递；
- peer-targeted 控制消息一律走 `X25519` 收件人端到端加密（机密性），发送者身份由 `Ed25519` 签名给出；
- `mailbox_secret` 只用于 topic 派生，不参与正文机密性。

### 3) peer/session 钥匙：只负责点对点数据面

当前体现为：

- `internal/tlsutil/pinned.go`
- `internal/signaling/mqtt/session.go`
- `dataplane/session_transport.go`

它负责的是：

- 由 `proxy_name + secret_key` 派生 `sid`，用于 rendezvous lane；
- 由 `secret_key + sid + role` 派生 pinned TLS 身份；
- 在 path 建好后升级成 `KCP -> TLS -> yamux`、`QUIC` 或 `TCP -> TLS -> yamux`。

这把钥匙的职责也应该非常窄：

- 只负责“两个 peer 之间这次 session 怎么安全建起来”；
- 不负责 group control-plane；
- 不负责 invite bootstrap。

## 对最小闭环的实际建议

新分支里，安全设计不要再跟着“任务种类”到处长，而应该直接按这三把钥匙收敛。

### 最小闭环必须保留

（以 Hard-Min 为准）

- trust bootstrap：`InviteCapability -> JoinRequest(PoP) -> Approve -> MemberCredential`
- 控制面（peer-targeted）：`Ed25519` 签名 + `X25519` 收件人端到端加密（统一 envelope 程序）
- MQTT 投递：`mailbox_secret` 仅用于派生 `net_root/inbox` topic（topic 不好猜，不承担正文机密性）
- 数据面：每个 peer session 都必须有强身份绑定的 secure channel（KCP 上显式 `TLS 1.3` pinned；QUIC 则使用其内置的 `TLS 1.3` 握手与身份校验）

这四项是简历里真正值得讲，而且逻辑清楚的安全主干。

### 最小闭环最好马上改掉的一点

当前 `mqttsig` 打洞交换更像是：

- `SID` 决定 topic lane；
- lane 下直接发 JSON；
- 它是 rendezvous / barrier 机制，不是完整的端到端控制面加密框架。

这在实现上能跑，但在叙述上很别扭，因为它和 join/approve 那套“签名 + AEAD 控制消息”不是同一套安全故事。

因此新分支更理想的方向是：

- join 之后，所有 peer-to-peer 控制消息，包括 punch request / punch response / barrier，都统一回到同一套 control-plane envelope；
- envelope 继续保留“签名 + AEAD”的模型；
- MQTT 只作为承载，不再有一套单独的明文 `mqttsig` 协议故事。

这样最小闭环就可以讲成：

- 未入网阶段：`InviteCapability -> JoinRequest(PoP) -> Approve -> MemberCredential`，并用 enrollment response（peer E2E）下发 `mailbox_secret` 等最小 bootstrap；
- 已入网阶段：所有 peer-to-peer 控制消息统一走 `peer_e2e_v1`（`Ed25519` 签名 + `X25519` 收件人端到端加密）；最小闭环不实现 group-scoped 广播/治理类控制消息；
- 建链成功后：所有业务数据都走 per-session secure channel（KCP 上显式 pinned `TLS 1.3`；QUIC 则使用其内置 `TLS 1.3` 握手与身份校验）。

这条线比“某些控制消息是签名+AEAD，另一些控制消息只是知道 SID 就能进入 lane”更适合作为新分支的主设计。

## 关于 transport secret 的取舍

当前点对点 session 安全主要绑在 `proxy_name + secret_key` 上。这一套不是不能用，但它在“作者自己能完整解释”的目标下有一个问题：它比较像旧主线历史包袱，不够自然。

因此新分支里这块可以有两种策略：

### 策略 A：第一版先保留 `secret_key`

优点：

- 更贴近当前实现；
- 更容易先跑通最小闭环；
- 后续先把架构边界立住，再考虑换密钥模型。

缺点：

- 简历里需要额外解释 `proxy_name/secret_key/sid/pinned TLS` 这一串历史设计。

### 策略 B：新分支直接把 peer transport secret 改成“由身份和网络上下文派生”

例如改成：

- `pair_secret = HKDF(X25519(self, peer), salt=sha256(network_id_bytes)[:16], info="miopunch/v1/pair_secret")`
- `sid` 与 pinned TLS 身份都从 `pair_secret` 派生

优点：

- 更容易讲清楚；
- 不需要在 peer config 里长期携带额外 `secret_key`；
- 更像一个统一的密码学故事。

缺点：

- 对当前实现切口更大；
- 会让“快速抽一个最小闭环”这件事多一层重构。

当前更稳妥的顺序仍然是：

- 先按策略 A 立住最小闭环主干；
- 但在设计上明确，`peer/session` 钥匙是一个独立切口，将来可以单独替换成更自然的派生方案，而不反向污染 invite/control-plane 主干。

## 已入网后的控制消息应分成 peer-targeted 与 group-scoped 两类（但 POC 只实现前者）

这里是最近讨论里最关键的一次收口。

Hard-Min 的方向是：

- 最小闭环里，发给具体 peer 的控制消息一律做 peer-to-peer 端到端（recipient-only）；
- group-scoped 的控制消息留作 capability 后加（而不是在最小闭环里引入 network-scope 的共享机密与广播治理复杂度）。

这意味着：

### 1) peer-targeted control

例如：

- dial offer / dial answer
- punch request / punch response
- barrier / attach / shell 控制
- 后续任何“只打算让最终目标 peer 看懂”的控制消息

这些消息即使经过：

- MQTT mailbox
- neighbor relay
- bounded flooding

也都应该保持 peer-to-peer E2E。

中间 relay 最多只该看到：

- `src_peer_id`
- `dst_peer_id`
- `msg_id`
- `expires_at`
- `hop_limit`
- `next_hop`
- ciphertext 长度

而不该看到：

- 业务 body
- punching 细节
- attach 参数
- shell 控制语义

### 2) group-scoped control（capability 后加，最小闭环先不做）

典型例子是：需要全体成员共同理解的组级状态与治理信息（例如 authority 轮换、吊销/策略 epoch、组广播等）。

POC 分支的 Hard-Min 约束是：

- 最小闭环先不实现 group-scoped 广播/治理类控制消息；
- 未来若要加回，应该作为独立 capability 叠加，不回头侵入 `peer_e2e_v1` 的主程序与闭环主流程。

## peer E2E control envelope 应先于 relay 实现定住

最小闭环不一定一开始就实现 overlay relay，但它现在就必须把“消息程序”定住。后面新增任何 relay/carrier，都只是适配这个程序。

当前更合理的形态是一个两层包：

- outer relay header：给 MQTT / relay / next-hop 看
- inner peer message：只给最终目标 peer 看

### outer relay header

外层只放路由和投递所需的最小信息，例如：

- `env_version`
- `route_mode`
- `src_peer_id`
- `dst_peer_id`
- `msg_id`
- `expires_at_unix_ms`
- `hop_limit`
- `next_hop_peer_id`
- `e2e_scheme`
- `ciphertext`

约束：

- relay 可以基于它做路由、去重、丢弃；
- relay 不应该需要理解 inner message；
- 后续无论是 MQTT 还是 mesh relay，都只该消费 outer header。

> 注：Hard-Min 最小闭环把 outer header 进一步收敛为更小的字段集（`v/src/dst/msg_id/expires_at/scheme/ct`），
> 并把 `src` 明确标注为“不可信，仅路由/调试”；`hop_limit/next_hop` 等多跳路由字段作为 relay capability 后加项。

### inner peer message

内层才是真正的控制语义，至少应包含：

- `proto_version`
- `src_peer_id`
- `dst_peer_id`
- `msg_id`
- `created_at_unix_ms`
- `expires_at_unix_ms`
- `kind`
- `in_reply_to`
- `body`
- `sig`

当前更倾向的处理顺序是：

- 先构造 inner peer message
- 对关键字段与 body 做 canonical transcript
- 用发送者 `Ed25519` 签名
- 再把整个 inner message 做 peer-to-peer 端到端加密
- 最后再包到 outer relay header

> 注：Hard-Min 版本把 inner 的身份来源收敛为 `src_pub_ed25519`（解密后验签并推导 `peer_id`），
> 不再把 `src_peer_id/dst_peer_id` 作为需要签名绑定的字段前置进 inner（避免双份真相与路由耦合）。

这比让 relay 直接处理一套半明文 `mqttsig` JSON 更适合作为长期主程序。

## invite / join / approve 不应再围着“大 invite code”设计

当前讨论已经基本确定：

- 现在这条招引/批准路线本身就是最小闭环必须实现的能力；
- 因此这里不适合“第一版先将就，第二版再找补”；
- `invite code` 塞太多状态，是当前设计里一个根本问题。

新分支更好的方向是把它拆成四个对象：

### 1) InviteCapability

它只表示：

- 允许某个申请方发起一次 enrollment

它最多应该带：

- `invite_id`
- `issuer public keys`
- 最小 enrollment route / rendezvous 信息（例如 MQTT broker + `join_topic`）
- `not_after`
- `issuer signature`

Hard-Min（v1）进一步烧死字段集：

- `network_id_bytes`（16B）
- `authority_ed25519_pub`（32B）
- `authority_x25519_pub`（32B，用于加密 `join_request`）
- `broker`（host:port + tls 配置，v1 先允许固定一套）
- `join_topic`（随机不可猜）
- `invite_id`（16B）
- `not_after_unix_ms`
- `sig`（authority Ed25519，对 InviteCapability transcript 签名）

InviteCode（v1）编码：

- 展示格式：`MPINV1-<base64url(no-pad)>`
- payload：InviteCapability 的 TLV bytes（不是 JSON）。

它不应该再带：

- `net_secret`
- `seed_peers`
- `brokers_effective`
- topology / bootstrap recommendations
- 长期 transport secret

### 2) JoinRequest

它的本质是：

- 申请方先本地生成长期身份
- 再提交“请把这组身份加入网络”的请求

当前更合理的请求内容是：

- `invite_id`
- `requester_ed25519_pub`
- `requester_x25519_pub`
- `reply_topic`（申请方自生成随机不可猜，并先订阅）
- `device_name`（可选）
- `platform`（可选）
- `created_at_unix_ms`
- `expires_at_unix_ms`
- proof-of-possession signature（用 requester Ed25519 对 join_request transcript 签名）

Hard-Min（v1）约束：

- 不单独引入 `request_id/nonce`（统一用 outer/inner 的 `msg_id` 做去重与追踪）。
- 不在 join_request 里塞 seed peers/topology/broker effective 等运行时状态。

### 3) MembershipCredential

批准动作的核心产物不应该再是“大 bundle”，而应该是：

- 一个由 network authority / delegated issuer 签发的成员资格凭证

它至少应绑定：

- `network_id`
- `subject_ed25519_pub`
- `subject_x25519_pub`
- `role`
- `issued_at`
- `not_before`
- `not_after`
- `issuer_id`
- `serial`
- `policy_epoch`
- `revocation_epoch`
- `issuer signature`

这更像一个轻量 credential，而不是一包杂糅配置。

> 注：Hard-Min 版本进一步收敛该字段集：不在 credential 内存 `peer_id`（统一由 `subject_ed25519_pub` 推导），
> 并且暂不引入 `serial/policy_epoch/revocation_epoch/root/issuer` 等完整治理字段。见后文 Hard-Min 字段集。

### 4) EnrollmentPackage

批准后交付给新成员的东西应尽量克制，只放最小 bootstrap：

- `membership_credential`
- authority bundle / trust anchors
- bootstrap routes
- initial reachable peers
- 必要策略版本

而不是再把大量运行时状态塞进去。

## authority 模型应从一开始就分 root / issuer / member

即使第一版实例很简单，逻辑模型也应先分清楚：

- `network root key`
- `admin issuer key`
- `member identity keys`

> 注：该段属于“完整模型”的方向性想法；Hard-Min 最小闭环明确先不做多级凭证链，
> 只保留单一 authority signing key（owner/admin 持有）。root/issuer 的拆分作为 capability 后加项。

更合理的语义是：

- root 决定谁有签发权；
- admin issuer 负责在线批准并签发 member credential；
- member 拿到 credential 后，用它参加后续控制面与数据面。

也就是说，批准动作更像是：

- “签发成员资格”

而不是：

- “把一个人塞进状态文件，然后顺便发一包共享秘密”

## 这条线与后续数据面/relay 扩展的关系

如果按这条方式重组：

- invite / join / approve 会成为 trust bootstrap；
- peer-targeted envelope 会成为后续所有 E2E 控制消息的共同程序；
- relay/MQTT 只负责投递，不再定义安全语义；
- path establishment 与 session recipe 只继续关心 carrier 和 secure session。

这样后面无论加：

- MQTT relay
- neighbor relay
- bounded flooding
- QUIC / TCP / 新的 session recipe

都不需要回头重写 trust bootstrap 和消息安全模型。

## 2026-05-21 补充：Hard-Min 收敛版（防止过度设计）

这一段是对前面讨论的“再收敛一次”。目标是把最小闭环变成一个可实现、可解释、且不会因为预留太多未来能力而变形的版本。

### 刹车点（明确不做）

- 不做 `root -> issuer -> member` 多级凭证链：最小闭环先只保留一个 authority signing key（owner/admin 持有）。
- 不强制引入 `CBOR/COSE`：最小闭环先用固定 transcript（字段顺序固定 + length-prefix）定义签名输入；JSON 仅用于日志/GUI 展示。
- 不做完整吊销系统：最小闭环靠 credential 短有效期 + 简单 epoch（可选）足够。
- 不在最小闭环实现 overlay relay/mesh：但消息 envelope 从一开始就按“relay 能转发密文但看不懂正文”设计。

### CreateNetwork 的默认产物

- `network_id`：随机生成（标识用途，不从任何 secret 推导）。
- `authority_ed25519_keypair`：唯一签发 key（最小闭环只有这一把）。
- `mailbox_secret`：随机 32B，仅用于派生 `net_root/inbox` topic（让 topic 不好猜）。

### 设备首次启动的默认产物（策略 A）

设备本地生成两对长期 key：

- `Ed25519`：用于签名身份与控制消息；并用于推导 `peer_id`。
- `X25519`：用于 peer-targeted 控制消息的端到端加密收件人公钥。

（明确不做 Ed25519->X25519 派生。）

## MemberCredential：不存 peer_id，只从公钥推导

最小闭环明确不在 credential 里放 `peer_id`（避免双份真相），统一用：

- `peer_id = base32(raw,no-pad, sha256(ed25519_pub)[:16])`

作为系统 peer 标识（与现有实现一致）。

### MemberCredential: Hard-Min 字段集

- `network_id`
- `subject_ed25519_pub`
- `subject_x25519_pub`
- `role`
- `not_after`（强制）
- `not_before`（可选但建议保留）
- `issuer_key_id`（建议保留做轮换钩子；最小闭环可固定为单一值）
- `sig`（authority Ed25519 对上述 v1 transcript 签名）

校验要点：

- 验签通过
- `network_id` 匹配
- 时间窗口有效
- 从 `subject_ed25519_pub` 推导的 `peer_id` 与系统使用的 peer_id 一致

> 注：这意味着前文里“MembershipCredential 绑定 `subject_peer_id`”的做法，在 Hard-Min 版本中被收敛为“只绑定公钥，peer_id 一律推导”。

## peer-targeted 控制消息：outer/inner envelope（带 src 但不信它）

最小闭环确认：

- outer 的 `src` 仅用于路由/调试/优化，不是安全语义来源；
- 安全语义上的发送者身份由 inner 解密后验签推导得出。

### Outer relay header（明文，Hard-Min）

- `v`
- `src`（不可信，仅路由/调试）
- `dst`
- `msg_id`
- `expires_at`
- `scheme = peer_e2e_v1`
- `ct`（ciphertext）

### Inner peer message（密文内，Hard-Min）

- `src_pub_ed25519`
- `msg_id`（必须与 outer 一致）
- `created_at`
- `expires_at`
- `kind`
- `in_reply_to`（可选）
- `body`
- `sig`（Ed25519，对 v1 transcript(inner_without_sig)）

处理顺序（固定为 sign-then-encrypt）：

- 构造 inner（不含 sig）
- 构造 v1 transcript（字段顺序固定 + length-prefix；带 domain-sep 字符串）
- 发送者 Ed25519 对 transcript 签名写入 sig
- recipient-only 端到端加密得到 ct（sealed-box 语义；v1 固定一套加密构造，不做参数矩阵）
- 包成 outer header 投递

### peer_e2e_v1：v1 固定加密构造（可实现口径）

- 长期密钥：每个设备固定两对 key：`Ed25519`（身份/签名）+ `X25519`（收件人公钥）。
- 每条控制消息：发送者生成一次性 `eph_x25519` keypair，并把 `eph_pub` 放进 `ct` 头部。
- 共享密钥：`shared = X25519(eph_priv, dst_x25519_pub)`。
- 对称密钥：`key32 = HKDF-SHA256(ikm=shared, salt=sha256(msg_id||dst_x25519_pub), info="miopunch/peer_e2e_v1/aead", L=32)`。
- AEAD：`XChaCha20-Poly1305`，`nonce24 = random(24B)`。
- AAD：outer header 的 `v|dst|msg_id|expires_at|scheme`（不含 `ct`）。
- `ct` 编码（单一格式，v1 不做可协商矩阵）：`ct = "MP1" || eph_pub(32) || nonce24(24) || aead_seal(inner_bytes)`。

> 注：v1 的原则是“只有一种正确做法”。后续如果真要引入 HPKE 标准化套件，也只能作为 v2 capability/新 scheme，不允许改写 v1 语义。

### v1 wire encoding（避免 JSON canonicalization 地雷）

- v1 所有 control-plane payload（outer header + inner + body）统一使用二进制 `TLV` 编码（MQTT payload 直接发 bytes）。
- `TLV = tag(uvarint) || len(uvarint) || value(bytes)`；整数编码固定为 Go `encoding/binary` uvarint。
- 只有 GUI/日志输出允许用 JSON；JSON 永远不进入签名/验签输入。

### v1 transcript（签名输入，字段顺序写死）

`transcript = domain_sep || TLV(fields...)`

- `domain_sep`：ASCII `"miopunch/v1/transcript/<context>"`
- `msg_id`：16B 随机（bytes），用于去重/重放防护（缓存到 `expires_at`）。
- `created_at/expires_at`：统一 unix ms。

Inner transcript（不含 `sig`）字段顺序固定为：

- `msg_id`
- `created_at`
- `expires_at`
- `kind`
- `src_pub_ed25519`
- `body_bytes`

InviteCapability transcript（不含 `sig`）字段顺序固定为：

- `network_id_bytes`
- `authority_ed25519_pub`
- `authority_x25519_pub`
- `join_topic`
- `invite_id`
- `not_after`

### v1 错误语义（丢弃 + 聚合；不回错误包）

- 解密失败/验签失败/过期/重放/格式错误等一律本地丢弃。
- GUI 只显示聚合计数与“最近一次原因”（避免刷屏）；详细证据进 Evidence。

推荐的 reason 分类（用于映射到 <=12 的 reason_code）：

- `decrypt_fail`
- `bad_sig`
- `expired`
- `replay`
- `unsupported_version`
- `malformed`

## mailbox_secret：只用于 topic 不好猜，不参与正文机密性

当前决定：当控制面消息全部搬到 MQTT 时，仍希望 inbox topic 不容易被外部枚举/猜中。

### mailbox_secret 的职责

- 仅用于“派生 inbox topic 名称”，降低外部猜测 topic 的概率。
- 不用于解密正文；peer-targeted 消息机密性由 `X25519` E2E 加密保证。

### MQTT topics（v1 固定）

最小闭环只保留三类 topic（避免把“控制面能力”膨胀成“系统结构”）：

- `join_topic`：入网申请投递点（随机不可猜；在 invite 里给出）。
- `reply_topic`：申请方临时回邮箱（随机不可猜；申请方先订阅后再发 join_request）。
- `net_root` 下的 `inbox/presence`：仅在批准后才可派生（依赖 `mailbox_secret`）。

约定：

- `join_topic = "mp/v1/join/<join_token>"`
- `reply_topic = "mp/v1/reply/<reply_token>"`
- `join_token/reply_token`：16B 随机，`base32(raw,no-pad)` 编码（建议 lower-case）。

### v1 投递规则（4 kinds 对应 3 类 topic）

- `join_request`：publish 到 `join_topic`；消息用 `peer_e2e_v1` 加密，收件人是 `authority_x25519_pub`。
- `enroll_response`：publish 到 `reply_topic`；消息用 `peer_e2e_v1` 加密，收件人是 `joiner_x25519_pub`；正文包含 `MemberCredential + mailbox_secret`。
- `dial_offer/dial_answer`：双方入网后，publish 到对方 `inbox_topic`；消息用 `peer_e2e_v1` 加密，收件人是对方 `x25519_pub`。

`mailbox_secret` 只用于派生 `net_root` 以及 `inbox` 名称；不参与正文机密性。

### net_root 派生（批准后才可得）

输入：

- `network_id_bytes`
- `mailbox_secret`（随机 32B）

算法：

- `salt = sha256(network_id_bytes)[:16]`
- `root16 = HKDF-SHA256(ikm=mailbox_secret, salt=salt, info="miopunch/v1/net_root", L=16)`
- `net_root = base32(raw,no-pad,root16)`（建议 lower-case）
- `topic_prefix = "mp/v1/net/" + net_root`

### inbox topic 派生（批准后才可得；建议与现有风格一致）

输入：

- `network_id_bytes`（创建网络时生成的随机 bytes）
- `mailbox_secret`（随机 32B）
- `peer_id`（由 Ed25519 pub 推导的 26 字符）

算法：

- `salt = sha256(network_id_bytes)[:16]`
- `name16 = HKDF-SHA256(ikm=mailbox_secret, salt=salt, info="miopunch/v1/topic.inbox/"+peer_id, L=16)`
- `inbox = base32(raw,no-pad,name16)`（26 chars，建议 lower-case）
- MQTT topic = `topic_prefix + "/inbox/" + inbox`

### presence topic（仅用于解释/观测；v1 的 Discover 只靠这个）

- MQTT topic = `topic_prefix + "/presence/" + peer_id`
- retain + LWT：peer 上线后立刻 publish retained `online`；同时设置 LWT 为同 topic 的 retained `offline`。GUI 的 `Discover` 通过订阅 `.../presence/+` 拿到“成员快照 + 在线状态”。
- payload：UTF-8 JSON（只为可读性；不进入任何签名/安全语义），字段固定：
  - `v`（固定 1）
  - `state`（`online|offline`）
  - `peer_id`
  - `ed25519_pub_b64url`
  - `x25519_pub_b64url`
  - `device_name` / `platform` / `app_ver`
  - `ts_unix_ms`

分发：

- 仅通过对新成员的 enrollment response（peer E2E）下发 `mailbox_secret`，并在本地持久化。

## 2026-05-21 补充：面试版 POC 分支契约（约束最小闭环不失控）

这一段把“为什么要抽离一个新分支做最小闭环”进一步落实成**硬约束**，用于防止实现过程中再次把未来能力、调试可观测性、GUI 展示欲望等东西重新塞回默认主流程。

### 目标与边界

- 该分支必须是一个**可演示的闭环 POC**，用于面试准备与简历叙事。
- 作者必须能对主流程的每一步给出可重复、可解释的回答（网络工程深度即可，不要求密码学专家深度）。
- 该分支不是“永久 demo 版”，必须为后续把主线能力以 capability 的方式接回去留出清晰接点，但**不允许**为了未来接回点而破坏最小闭环的收敛。

### 必须可演示的产品闭环（GUI + Win/Linux 可运行）

闭环固定为：

- `CreateNetwork` / `JoinNetwork`
- 设备发现（看到 peer 列表、状态）
- NAT/STUN 探测与候选交换（可解释、可观测）
- `UDP` punching 建路（path establishment）
- 数据面 session recipe 建立（默认：`UDP + KCP + TLS 1.3 + yamux`）
- `ping`
- `shell`

约束：

- GUI 的职责是给用户“下一步做什么”和“现在卡在哪里/为什么卡住”的答案；不展示全量内部状态机细节，不把 debug 信息当产品信息输出。
- GUI 输出分两层：`UserSummary`（每个 stage 最多 3 行“人话”）+ `Evidence`（可折叠/可导出：STUN/NAT/candidates/punch matrix/TLS 过程等）。
- GUI stage 固定为：`Network` / `Enroll` / `Discover` / `Punch` / `SecureSession` / `Shell`。
- `reason_code` 总数上限 12（新增必须合并/替换旧的；禁止无边界增长）。
- Windows/Linux 上应能直接运行（非“只在开发机/只靠脚本”才能跑起来的打包方式）。

### 可解释性要求（面试问答的最低口径）

对每一个关键子系统（例如 punching、MQTT mailbox、数据面 recipe、控制面 E2E、netns 测试）至少能回答三类问题：

- 这一层解决什么问题，边界在哪里（它不解决什么）。
- 常见失败形态是什么，如何观测与定位（日志/指标/可视化的最小集合）。
- 有哪些替代方案，为什么当前 POC 分支先不选它（或作为 capability 后加）。

特别说明（密码学深度的边界）：

- 作者可以说明：威胁模型、签名/加密在系统里承担的职责、以及为什么“现有方案在 POC 目标下足够”。
- 不承诺能回答形式化证明、密码算法细节安全性推导等“密码学专家级”问题。

### Rule of One：最小闭环每条轴先只允许一种实现

为避免“过度设计”，最小闭环阶段对关键轴做硬限制：

- path establishment：只做 `UDP punching`（保留 TCP/其它 path 作为 capability 后加点）。
- 数据面 session recipe：只做 `UDP + KCP + TLS 1.3 + yamux`。
- 控制面投递：只做 `MQTT mailbox`（入网 bootstrap + punching 协商期交换）。
- 控制面端到端安全：只做 `peer_e2e_v1`（sign-then-encrypt；recipient-only；X25519 ECDH + HKDF + 单一 AEAD），broker/relay 只负责投递不承担安全语义。

任何一条轴要加入第二种实现（例如 QUIC、TCP path、多种 E2E scheme）时，才允许引入更一般化的接口/抽象；否则一律用具体 struct/函数实现，避免为未来想象造框架。

### v1 Punch（5B：有限并发矩阵，不做 ICE/barrier）

- `dial_id`：16B 随机。
- `punch_token`：16B 随机。
- 尝试策略：最多并发 4 个 candidate pair；总预算 10s；先成功先收敛到唯一 endpoint。
- 成功口径：以“收敛到一个经过验证的可用 UDP path，并产出 `PathResult`”为最终成功；KCP 建链和 TLS 验证属于后续 `SecureSession`。

`dial_offer/dial_answer` 的 body（v1 固定最小集）：

- `dial_id`
- `punch_token`
- `candidates`（host/srflx）
- `member_credential`（用于后续数据面 pin 身份）

### v1 SecureSession（6A：pin 到 MemberCredential 身份）

- TLS 证书：每个 peer 用自己的 Ed25519 key 自签，仅用于携带公钥（不走 WebPKI）。
- 验证口径：TLS 握手完成后，取对端证书的 Ed25519 pub，必须与对端在 `dial_offer/dial_answer` 中携带的 `MemberCredential.subject_ed25519_pub` 一致，并且该 credential 必须能被 `authority_ed25519_pub` 验签通过。
- 工程约束：TLS 里用自定义 verify（例如 `VerifyConnection`）实现上述校验；v1 不引入第二套 pin 机制。

### v1 Persistence（7B：多网络最小化结构）

- `device/`：设备长期 `ed25519/x25519`。
- `networks/<network_id>/`：`member_credential`、`mailbox_secret`、`broker`、`last_seen_peers`、`ui_state`。
- v1 不做加密落盘/解锁流程；只要求文件权限收紧（0600/0700）。

### 主干不变：PathResult -> SessionRecipe -> PeerSession -> LogicalStream

最小闭环分支的“可接回主线能力”的前提不是预先引入复杂框架，而是保持主干边界稳定：

- `PathResult`：只描述“这次建出来的可用路径资源”，不夹带上层协议选择。
- `SessionRecipe`：在已建立路径上升级成可复用 `PeerSession` 的具体做法（POC 默认只有一条）。
- `PeerSession`：上层业务（ping/sh）唯一依赖的边界。
- `LogicalStream`：业务流。

主线里已有的复杂能力（mesh/neighbor/relay/topology 等）以 capability 形式接回时，尽量只新增 recipe 或新增控制面 capability，不回头重写主干。
