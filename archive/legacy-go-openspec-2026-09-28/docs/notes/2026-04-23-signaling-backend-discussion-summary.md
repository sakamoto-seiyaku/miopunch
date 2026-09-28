# 2026-04-23 signaling backend 讨论摘要（临时）

> 目的：把本轮关于 `coord`、`MQTT`、可插拔 signaling backend、以及慢速/状态型信道的讨论先落盘，避免上下文丢失。  
> 状态：临时讨论记录；不是最终设计稿；后续若继续推进，应收敛为 decision / charter / OpenSpec change。  
> 背景：当前产品主线已经完成最小 POC 闭环，但后续希望继续讨论“更去中心化的控制面入口/信道插件化”。

## TL;DR（截至 2026-04-24）

- 第一版只叫 **`V1`**：先把“外部信道/入口”拆成可插拔 `backend` 框架，并跑通双 backend（主备）落地。
- `V1` 推荐组合：`MQTT + NATS(core)`（主备，上限 2）。`demo.nats.io` 仅用于 smoke/开发验证，不作为生产依赖。
- `V2` 候选（更慢但更广泛/更自控）：`Git private repo` 与 `Email（SMTP + IMAP/POP3）` 等 `store-and-poll / store-and-forward` carrier。
  - 它们更适合 `bootstrap + fallback mailbox + scheduled/store-and-poll exchange`，不应假设 `realtime exchange`。
  - Email 需要把反垃圾/投递延迟/元数据暴露与（若自建）较高运维成本写进预期。

## 本轮先确认的上下文

- 产品二进制 `miopunch` 已不再暴露 `coord/peer/stun/mqtt-broker` 这类实验入口；这些入口保留在 `miopunch-lab`。
- `coord server` 仍存在于 lab/实验链路中，但产品 POC 主线不应再把“必须自建 dedicated coord server”作为默认心智模型。
- NAT/打洞分析逻辑已收束到 `internal/punchdecision`；MQTT 路径由 visitor 侧 leader 直接调用中立决策边界做一次性分析。
- 已在 `docs/roadmap.md` 的“后续方向”中更新基础清理状态：产品路径去掉 `coord` 服务语义，`internal/coordinator` 仅保留 lab coord 适配器语义。

## 对问题的重新表述

本轮讨论后的核心问题，不再是：

- “MQTT 能不能被某个别的协议替换？”

而更接近：

- “一个完整的 signaling backend 需要提供哪些能力？”
- “不同 backend 可以用什么模式来完成这些能力？”
- “backend 的差异，最终应该体现在 capability profile 和 protocol strategy 上，而不是写死成 `mqtt` / `dht` / `irc` 这些名字。”

## 当前讨论后的暂定结论

### 1) `coord` 作为服务，不应继续占据产品主路径

- `coord server` 可以继续保留为 `miopunch-lab` 的实验/回归入口。
- 但产品主线路径不应再依赖 dedicated coord server。
- 后续若继续演进，应继续把“分析/决策逻辑”和“coord server 形态”分开看：前者属于 `internal/punchdecision`，后者仅是 lab 入口。

### 2) signaling backend 不应只被理解为“替换 MQTT”

- 当前 `MQTT` 同时承担了多种语义：
  - 入口/接入 hint
  - mailbox / fallback delivery
  - NAT exchange / barrier / start coordination
- 后续不应把这些语义继续混成“一个固定 MQTT backend”。

### 3) 一个**完整**的 signaling backend，至少必须支持三类能力

注意：这里讨论后的口径不是“这三类能力可选其二”，而是：

- 一个完整 backend **必须全部支持**
  - `Discovery`
  - `Mailbox`
  - `Exchange`

区别只在于：`Exchange` 的实现模式可以不同。

## 2026-04-24 新增确认的高层口径

### 1) external backend 的正式定位

当前已确认：

- 外部 backend 在系统中的定位是：
  - `bootstrap`
  - `fallback`
  - `exchange`（`realtime` 或 `scheduled` 至少一种）

也就是说：

- 外部 backend 不是“纯 bootstrap-only 入口”
- 它是正式的 signaling 组成部分

### 2) backend 是网络级对象

当前已确认：

- backend 属于 **network-level config**
- 一个网络允许配置多个 backend
- 但后续实现不应把“多个 backend 并存”做得过于复杂

补充确认（针对 `join code` / bootstrap）：

- `BackendRoster`（例如 `MQTT + NATS` / `MQTT + DHT`）属于网络属性，最终应下发/同步到每个节点
- `BackendRoster` 的变更（新增/删除/主备切换）应通过治理/快照更新来完成（而不是每个节点本地手配覆盖）
- `join code` 不需要限制“只能编码一个入口”；编码两个 seed 入口也完全允许
- `join code` 倾向额外携带一个可校验的 hash（`network definition snapshot hash`），用于让新节点在入网阶段快速做一致性校验（同时也能自然表达“这份 join code 可能已过期”）
  - 当前讨论倾向：hash pin 到“网络定义/网络属性”层的治理快照（包含 `BackendRoster` + `owners/admins` 等重大网络属性）
    - 好处：backend roster 变更、admin/owner 变更都会让旧 join code 失效；符合小团队“重大变更=重新发码”的运维习惯
    - 同时不把成员集/decls head 纳入 pin 范围：否则每次 approve/revoke 都会让尚未过期的 join code 失效，并与 `max_uses` 语义冲突
  - hash mismatch 直接视为“join code 已过期/不匹配”，并作为硬错误失败（个人/小团队场景更可接受）
- 即使 `join code` 只携带一个 seed，节点入网后也应通过网内同步拿到完整 roster
- `join code` 的作用是 bootstrap hint；网络级 roster 以治理/快照等可验真的网络对象为准

### 3) backend 之间地位平等

当前已确认：

- backend 不存在“一等公民 / 二等公民”之分
- 只要某 backend 能满足系统要求的 signaling 数据传输能力，它在系统中的地位就是平等的
- 差异只应通过 capability / profile 体现，而不应在架构层预设“协议等级”

### 4) backend 一律不可信

当前已确认：

- backend 一律按“不可信入口/不可信传输介质”建模
- 这与当前对 `MQTT broker` 的处理口径一致：
  - backend 负责承载入口、状态或消息
  - backend 不应拥有控制网络语义的权力
  - backend 不能解密控制面消息
  - backend 不能伪造受信控制面语义

进一步口径：

- backend 可能暴露运行时元数据（例如流量规模、活跃度、节点数量推断、admin 模式推断等）
- 但它不应因此获得操控网络的能力

### 5) 上层只看 capability / profile，不看 backend 内部实现细节

当前已确认：

- 调用 backend 时，上层不应知道其具体底层实现细节
- 例如：
  - 是 `MQTT publish`
  - 是 `git push`
  - 是 `DNS TXT` 更新
  - 是某聊天平台消息发送
- 这些都应由 backend 自己封装

上层只应看到：

- backend 提供了哪些能力
- 它是否满足调用要求
- 它的 profile / 元数据（例如快/慢、realtime/scheduled、payload 上限等）

### 6) 多 backend 采用“主备 / 优先级选择”，而不是镜像复制

当前已确认：

- 多 backend 共存时，先采用 **模型 B：主备 / 优先级选择**
- 不采用“同一控制状态/消息同时镜像写入多个 backend”的模型

这意味着：

- 一个网络可以配置多个 backend
- 讨论中先限定最多 2 个：primary + backup（`MQTT + NATS` / `MQTT + DHT` 只是示例；用户可配置任意两种组合）
- **写端**：外部 fallback 时按优先级择一写入；若主路在简单超时预算内不可达/失败，则切换到备用；不做并行镜像
- **读端**：为处理非对称可达，需同时监听/轮询所有已配置 backend，并在本地合并 + 去重

这样做的目的：

- 避免引入“backend 与 backend 之间的额外实时同步问题”
- 同时避免引入“全网需要先协商出同一个 active backend 才能收敛”的额外复杂度
- 把复杂度控制在：
  - network config 如何表达 backend roster 与优先级
  - 多路收件箱的合并与去重
  - 写端失败时的主备切换

### 7) 多 backend 模式下的最小边界

当前已确认的实现倾向：

- 对 `Mailbox`：读端同时收两路；写端外部 fallback 择一写入（主备切换按写端失败决定）
- 对 `Exchange`：同一 `round` 的 exchange 状态/锚点只落在一个 backend；切换只发生在进入下一 `round`

也就是说：

- 不在同一 round 内把 exchange 的关键状态拆到多个 backend
- 若该 round 失败，下一 round 可以切到备用 backend

### 8) `scheduled exchange` 的协议边界表述（修正版）

当前已确认的更准确口径是：

- `scheduled exchange` 的**协议边界**明确排除 `hard NAT ↔ hard NAT`
- 除此之外，其余场景原则上纳入目标范围
- 若某个具体 backend 达不到所需 timing/profile，则视为：
  - **该 backend 不满足 scheduled 能力**
  - 而不是继续修改 `scheduled exchange` 的协议边界

这条口径把：

- 协议边界
- backend 实现能力

明确分开了。

### 9) 默认 backend 的产品口径

当前已确认：

- 在“不做任何配置”的情况下，可以允许 `MQTT` 作为默认 backend
- 但 backend 仍然属于 network config 的一部分
- 不采用“编译时指定 backend”或“运行时随意动态加载 backend”的产品模型

## 三个能力面（不是三条物理信道）

### A. Discovery

问题定义：

- 新节点刚加入时，如何找到网络入口、或找到至少一个可继续交互的 rendezvous 点？

可能的实现形态：

- `MQTT topic prefix`
- `DHT rendezvous key`
- `IRC/聊天软件` 的 channel / group
- `DNS TXT`
- `git private repo`
- 其它可轮询/可发现的状态源

关键点：

- `Discovery` 不要求秒级互动；
- 只要求“新加入者能拿到下一步所需入口”。

### B. Mailbox

问题定义：

- 已知目标 peer/会话后，如何把一条控制消息最终投递给对方？

可能的实现形态：

- `publish/subscribe`
- `chat message`
- `repo commit / file write`
- `DNS TXT + poll`
- `object store / KV / shared state`

关键点：

- 允许慢；
- 允许重复/乱序；
- 上层可依赖 `msg_id / expires_at / cached response / 去重 / 幂等` 修复投递语义。

补充：当网络配置多个外部 backend（例如 `MQTT + NATS` / `MQTT + DHT`）时，一个直觉类比是：

- 每个 backend 都提供一套“外部收件箱（inbox）”能力
- 同一个 `peer_id` 会在每个 backend 上各自派生出一个 inbox 地址（topic / key / channel 等）
- 节点的“逻辑收件箱”是多路合并视图：
  - 网内（直连/转发）收到的控制消息
  - 外部 inbox@MQTT 收到的控制消息
  - 外部 inbox@DHT 收到的控制消息

配套的最小规则（KISS）：

- **网内优先**：正常情况下先走网内直连/转发；外部 backend 主要承担 bootstrap 与 fallback
- **写端择一**：外部 fallback 时按优先级择一写入（主备切换按写端失败决定），失败判定以“简单超时”为主，不做并行镜像
- **读端全收**：为处理非对称可达，读端需要同时监听/轮询所有已配置 backend 的 inbox，并在本地合并 + 以 `msg_id` 去重
- **回包尽量沿原路**：若某请求/消息是从某个 external backend 收到，则其响应/回包优先走同一个 backend（这是已验证可达的路径）；失败才按主备顺序切换

关于“简单超时”的补充口径：

- 主备切换阈值建议作为 backend profile/config 的一部分（例如 per-backend 的 fallback timeout / retry budget）
- 若某 backend 未提供该信息，则使用系统默认值

### C. Exchange

问题定义：

- 双方如何对一次 NAT traversal 尝试达成共同起跑状态？
- 这不只是“发 candidate”，而是：
  - 建立同一 session 视图
  - 交换 candidate snapshot
  - 确认双方都已收到
  - 协调同一个 attempt window / `start_at`
  - 在失败时进入下一轮

这里的关键结论是：

- `Exchange` 是完整 backend 的**必选项**；
- 变化的是 `ExchangeMode`，不是“要不要 exchange”。

补充确认（多 backend + 网内优先 的落地口径）：

- `Exchange` 仍遵循 **网内优先**：若网内直连/转发可通，则优先在网内完成本轮 exchange 所需信息交换与 barrier/窗口协调
- 若网内不可通，则按外部 backend 的主备顺序做 fallback（例如 primary→backup），失败判定同样以“简单超时”为主
- 一旦某条路径（网内/某个外部 backend）收到对端响应并形成共识，本 round 的 exchange 关键状态/锚点即绑定到该路径；不在同一 round 内跨 backend 拆分关键状态
- 上述描述中 `MQTT + NATS` / `MQTT + DHT` 仅为示例；实际允许用户配置任意两种 backend 组合作为主备

## ExchangeMode：当前讨论中出现的三种模式

### 1) `realtime`

特征：

- 双方近似秒级互通；
- 实时交换 candidate；
- `ready -> start_at(now + small_delta) -> attempt`。

更贴近当前 `MQTT signaling` 的实现方式。

### 2) `scheduled`

特征：

- 信道较慢，但仍能传输明确的会话状态；
- 双方先约定未来一个 `start window`（不是单点 `start_at`）；
- 在窗口开始前完成 candidate 交换；
- 双方在同一个 window 内持续 attempt。

讨论中的共识：

- 慢聊天信道并非不能做 exchange；
- 只是它们需要的是“scheduled exchange protocol”，而不是简单复用当前 MQTT 的快 barrier 协议。
- 当前先**明确排除**：`hard NAT ↔ hard NAT` 不属于 `scheduled exchange` 的支持范围。
- 当前倾向先使用更简单的 v1 口径：
  - `start window` 长度可放宽到 `20–30s`
  - 不把它继续拆成更复杂的多层窗口/多阶段 freshness 子协议
  - 双方在整个 window 内持续 attempt，而不是追求某一刻完全同时开始

### 3) `store-and-poll`

特征：

- backend 更像共享状态存储，而不是 message bus；
- 双方通过“写入状态 + 轮询对方状态”来完成会话收敛；
- 可以用在：
  - `git private repo`
  - `DNS TXT`
  - `KV / object storage`
  - 其它 write/read/poll 形态的信道

讨论中的重要修正：

- `Exchange` 不一定非要是“消息交换”；
- 它也可以是“状态收敛”。

## 当前已收窄的 `scheduled exchange` 口径（v1 倾向）

### 1) `scheduled exchange` 使用 `start window`

- 不再把慢速 exchange 设计成单点 `start_at`
- 改为：
  - `window_start`
  - `window_len`
- 双方只要在同一个 window 内有足够重叠，即可持续 attempt

### 2) 明确不支持 `hard NAT ↔ hard NAT`

- 当前讨论已经收窄为：`scheduled exchange` 明确**不支持** `hard NAT ↔ hard NAT`
- 这样可以避免为了少数最难场景引入过多复杂度

### 3) 时间同步要求不追求“严格同步”

- 由于使用的是 `start window`，而不是单点开始：
  - 不需要把时钟同步建模得过于苛刻
  - 当前倾向只要求双方时钟“大致一致”
- 当前讨论中的经验口径：
  - 目标：双方时钟误差在 `3–5s` 内
  - 若误差超过约 `10s`，稳定性会明显下降

### 4) 当前先不把协议复杂化

- v1 先不引入更复杂的：
  - 子窗口
  - 多层 coordination / execution window
  - freshness/keepalive 的细粒度模型
- 先采用最简单的理解：
  - 双方交换 candidate
  - 协调出 `window_start + window_len`
  - 双方在该 window 内持续 attempt
  - 失败则进入下一轮

### 5) `window_start` 的决定规则：由 session leader 决定

当前讨论后的明确倾向：

- **backend 不决定 `window_start`**
- `window_start` 由协议中的 **session leader** 决定
- backend 只负责搬运/存储状态，并提供 profile（例如最小 lead time / 默认 window 长度）

当前 v1 倾向：

- `initiator / requester / 发起方` 是 leader
- 在当前 `visitor/client` 语义里，倾向直接把 **visitor 视为 leader**

leader 的最小职责：

- 确认本 round 双方 candidate 都已齐备
- 计算并发布：
  - `round_id`
  - `window_start`
  - `window_len`
- 非 leader 只接受，不覆盖该 round 的 window

当前讨论中的默认值（仅作为 v1 倾向，不是最终冻结）：

- `min_lead_time = 15s`
- `window_len = 30s`

也就是：

- leader 观察到双方 candidate 齐备后
- 发布“从 `now + 15s` 开始、持续 `30s` 的 attempt window”

## backend 的两大类型（本轮新增视角）

### 1) message-oriented backend

例子：

- `MQTT`
- `IRC`
- `Matrix`
- 聊天软件 bot
- `DHT pubsub`

倾向：

- 更适合 `realtime exchange`
- 也可支持 `scheduled`

### 2) state-oriented backend

例子：

- `git private repo`
- `DNS TXT`
- `KV / object storage`
- 共享文件/共享状态源

倾向：

- 更适合 `scheduled` 或 `store-and-poll exchange`
- 不应被强行抽象成假 pubsub

## 对“慢速信道”的新理解

本轮有一个重要澄清：

- “慢”不是简单的负面属性；
- 它意味着 backend 可能需要走不同的 exchange protocol。

例如：

- 聊天软件 / IRC 可能既能做 `Discovery`，也能做 `Mailbox`，还可以通过 `scheduled exchange` 做 `Exchange`。
- `DNS TXT` / `git private repo` 这种状态型 backend，也可能支持完整 signaling，只是它们更偏向 `store-and-poll exchange`。

因此后续不应简单问：

- “这个 backend 能不能替代 MQTT？”

而应问：

- “这个 backend 以哪种模式实现 exchange？”
- “它支持哪些 path / NAT 场景？”

## 本轮对几个外部例子的理解

### `edgevpn`

当前理解：

- 它更像一整套 `libp2p + DHT + gossip` overlay / discovery substrate；
- 不是一个“把 MQTT 换成 DHT”的小替换；
- 它证明的是“去中心化 bootstrap / discovery / overlay 是可行的”。

### `wa-tunnel`

当前理解：

- 它本质上是“通过 WhatsApp 文本/文件消息搬运 TCP payload”的 carrier/tunnel；
- 它证明的是“奇特/慢速第三方信道也能承载控制/数据交互”；
- 但它不是现有 `miopunch` signaling 设计的直接蓝本。

### `git private repo`

本轮脑暴中新增的重要方向：

- 私有 repo 可以被视为一种状态型 backend；
- 双方通过写文件/提交状态、轮询对方状态来完成 discovery/mailbox/exchange；
- 它不适合假装成 pubsub，但可能天然适合 `store-and-poll exchange`；
- 需要注意 repo 写权限、元数据暴露、回滚/旧值、版本一致性等问题。

## 当前还未定、但已经明确需要讨论的问题

### 1) 统一的 exchange 状态机

建议后续明确：

- `INIT`
- `ROUND_OPEN`
- `LOCAL_CANDIDATES_PUBLISHED`
- `REMOTE_CANDIDATES_OBSERVED`
- `START_AT_AGREED`
- `ATTEMPTING`
- `SUCCESS / FAILED`
- `NEXT_ROUND`

### 2) round / session 模型

需要明确：

- `session_id`
- `round_id`
- `candidate_version`
- `gathered_at`
- `expires_at`
- `start_at`
- `attempt_deadline`
- `result`

### 3) `start_at` 协调规则

当前讨论已收窄为：

- 不再优先讨论单点 `start_at`
- 改为讨论 `window_start + window_len`
- 当前倾向：由 **session leader** 决定并发布该 round 的 window
- backend 只声明 profile，不直接掌握会话时序语义

仍待继续明确：

- leader 规则是否固定为 `visitor/initiator`
- 不同 backend profile 的 `min_lead_time` 默认值
- clock skew 超过何值时应该明确拒绝 scheduled 模式

### 4) candidate 新鲜度与 keepalive

这是 scheduled/store-and-poll 方案里的核心风险：

- candidate 生成后等待太久，NAT mapping 可能已变化或失效；
- 需要讨论：
  - candidate freshness budget
  - mapping keepalive
  - 哪些 path class 允许慢速 exchange

### 5) backend capability profile

需要明确每个 backend 至少声明：

- discovery 方式
- mailbox 方式
- `exchange_mode`
- `latency / visibility` 级别
- payload 上限
- push / poll 语义
- 是否支持 ordering / ack
- 适用的 path / NAT 类别

### 6) 统一 envelope / payload 规则

后续需要统一：

- `msg_id`
- `dst_peer_id`
- `created_at / expires_at`
- `delivery_mode`
- `payload_inline / payload_ref`
- 分片 / chunking / 引用策略

### 7) 安全模型

特别是状态型 backend，需要明确：

- backend 是否可信
- provider 能看到哪些元数据
- 回滚 / stale read / replay 怎么处理
- backend 是否拥有 network authority
- 写权限与密钥分发如何定义

## 当前偏向的方向（仅讨论口径，不是最终设计）

- 后续设计不应再围绕“固定 MQTT broker”展开。
- 更合适的方向是：
  - `SignalBackend = capability profile + protocol strategy`
  - backend 差异通过 `ExchangeMode` 与支持的 path class 体现
  - 同一 backend 可以用不同方式满足 `Discovery / Mailbox / Exchange`

## 建议的下一轮讨论顺序

当前阶段更合适的做法是：

- 先收敛 **高层架构边界**
- 暂不下钻到 `scheduled exchange` 的最小消息集/字段集
- 暂不提前定义各 mode 的具体 wire 细节

原因是：

- 目前更大的不确定性，不在字段级协议
- 而在于：一个网络如何表达多个 backend、以及在非对称可达下如何用主备落地（不引入 backend↔backend 同步）

因此，下一轮更建议按这个顺序讨论：

1. network config 如何表达 backend 列表、优先级、默认项与 capability/profile  
2. 在非对称可达下的最小落地规则：读端全收（多路监听/轮询）+ 合并去重；写端择一（主备切换按写端失败）  
3. `Exchange` 在多 backend 下的最小边界：round 锚点只落一个 backend；失败后下一 round 才切换  
4. backend 向上层至少需要暴露哪些高层 profile（例如 `exchange_mode`、延迟级别、是否需要 `min_lead_time`）  
5. 第一批值得真正验证的 backend 组合（例如 `MQTT` 基线 + 一个状态型 backend）

当前已确认并作为上述讨论前提的边界是：

- 多 backend 采用 **模型 B：主备 / 优先级选择**
- 读端全收：同时监听/轮询所有已配置 backend 的 inbox，并在本地合并 + 去重
- 写端择一：外部 fallback 时按优先级择一写入（失败才切换），不做并行镜像
- backend 切换只发生在 `round` 边界
- `scheduled exchange` 的协议边界明确排除 `hard NAT ↔ hard NAT`
- 若某 backend 达不到所需 timing/profile，则视为该 backend 不满足 `scheduled` 能力，而不是修改协议边界
