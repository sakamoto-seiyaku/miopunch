# Door 3 Signaling Backend 纲领（多 backend / 去中心化入口）

## 文档状态

- 本文档固化 Door 3（Signaling backend 插件化）的目标、边界、约束与关键原则。
- 本文档不展开实现细节，不替代后续 OpenSpec change。
- 本文档不包含具体字段名、函数签名、wire 细节；这些应在后续 change 的 `proposal / design` 中收敛。

## 背景（我们要解决的真实问题）

- 当前产品主线的控制面实践可以概括为：**网内优先（mesh 转发） + 外部兜底（MQTT mailbox）**。
- 这在 POC 阶段成立，但它隐含了一个“固定 MQTT broker 入口”的心智模型；后续希望把“外部入口/信道”抽象出来，变成可插拔 backend。
- backend 可能是 pubsub（如 `MQTT/IRC/Matrix`），也可能是状态型（如 `git private repo/DNS TXT/KV`）。
- 关键目标不是“把 MQTT 换成 DHT”，而是让系统能表达：**同一套上层语义**在不同 backend 上以不同模式实现，并在非对称可达下仍可落地（KISS）。

## 核心决策（已确认）

### 1) external backend 的正式定位

- 外部 backend 在系统中的定位是：
  - `bootstrap`
  - `fallback mailbox`
  - `exchange`（至少支持 `realtime` 或 `scheduled` 之一）

### 2) backend 是网络级对象（Network Config）

- backend 属于 **network-level config**（而不是每个节点本地手配覆盖）。
- `BackendRoster`（例如 `MQTT + DHT`）属于网络属性，最终应同步到每个节点。
- roster 变更（新增/删除/主备切换）应通过治理/快照等可验真对象传播。

### 3) backend 地位平等 + 一律不可信

- backend 不存在“一等/二等”之分；差异仅通过 capability/profile 体现。
- backend 一律按“不可信入口/不可信传输介质”建模：
  - backend 不能解密控制面消息
  - backend 不能伪造受信控制面语义
  - backend 可能暴露运行时元数据（规模/活跃度推断等），但不因此获得 network authority

### 4) 一个**完整**的 signaling backend 必须支持三类能力

> 口径：不是“可选其二”，而是“完整 backend 必须全部支持”，区别只在实现模式。

- `Discovery`：新节点/新会话如何获得入口与必要 hint。
- `Mailbox`：当网内不可达时，如何投递/收取控制消息（兜底收件箱）。
- `Exchange`：至少支持一种交换模式（见下节）。

### 5) `BackendProfile` 最小集合（默认推荐）

为避免把 backend 差异写死成 `mqtt/dht/irc/...`，上层只依赖 backend 暴露的最小 profile：

- `exchange_mode`：`realtime | scheduled | store-and-poll`
- `latency_class`：用于粗粒度表达“快/慢”的调度差异（不要求精确 RTT）
- `min_lead_time`：scheduled 下从“发布 start window”到“开打窗口起点”的最小提前量
- `max_payload_bytes`：单次写入/读出的 payload 上限（影响 inline vs ref/chunk）
- `push_or_poll`：收件箱语义是 push/subscribe 还是 poll/list

## 多 backend 的最小落地（KISS，上限 2）

### 目标上限

- 一个网络最多配置 **2 个** external backend：`primary + backup`。
- 若未来要支持 `>2`，需要新的 decision（避免现在过度设计）。

### 核心规则

- **读端全收**：同时监听/轮询所有已配置 backend 的 inbox，并在本地合并为一个逻辑 inbox（`msg_id` 去重 + 端到端验真/解密）。
- **写端择一**：对外写入按优先级只选一个 backend；失败（超时/不可达）才切换到 backup。
- **不做镜像/同步**：不把同一条消息并行写入两个 backend；也不引入 backend↔backend 同步层。
- **回复优先同路**：收到请求后，优先沿“请求到达的 backend”回复；失败再按优先级 fallback。
- **网内优先**：正常情况下控制消息应走网内直连/转发；external backend 主要用于 bootstrap 与兜底。

## Exchange 的模式与边界

### ExchangeMode

- `realtime exchange`：低延迟交互式交换（典型 pubsub backend 更擅长）。
- `scheduled exchange`：基于 `start window` 的约定式交换（慢速/状态型 backend 也可做）。
- `store-and-poll exchange`：状态写入 + 轮询观察的交换（属于 exchange 的一种实现风格；不要求伪装成 pubsub）。

### scheduled exchange 的协议边界（已收口）

- `scheduled exchange` 的协议边界明确排除：`hard NAT ↔ hard NAT`。
- 除此之外，其余场景原则上纳入目标范围。
- 若某个具体 backend 达不到所需 timing/profile，则视为该 backend **不满足 scheduled 能力**，而不是再去细分协议边界。

### scheduled exchange 的默认调度规则（KISS）

- **leader 固定**：由发起方（initiator/visitor）作为 round leader，负责发布该 round 的 `start window`。
- **clock skew 严格**：一旦检测到 clock skew 超阈值，则该 backend 在本 round 不走 scheduled（若另一个 backend 支持 realtime，则降级；否则失败并提示校时）。
- **window 可大**：允许把窗口从 `2–5s` 扩大到 `20–30s`（由 backend profile 约束）；代价是候选新鲜度与失败重试成本上升。
  - 对上层口径：这不是“协议边界更细”，而是“该 backend 的 scheduled profile 是否足够”。

### 多 backend 下 exchange 的锚点规则（最小语义）

- `Exchange` 的 round 在一次尝试中只锚定 **一个** backend（避免一轮状态跨 backend 分裂）。
- backend 切换只发生在 `round` 边界：本 round 失败→下一 round 才切换 backend。
- path 选择顺序（高层）：网内可达 → external primary → external backup（都失败则失败）。

## Join code 与 hash pin（已确认口径）

### 1) seed backend

- `join code` 允许携带 **1 或 2 个** seed backend（作为 bootstrap hint）。
- 即使只携带一个 seed，入网后也应通过网内同步拿到完整 roster。

### 2) network definition snapshot hash（严格校验）

- `join code` 倾向额外携带一个可校验的 hash：`network definition snapshot hash`。
- pin 的对象是“网络定义/网络属性”层快照（低频变化）：
  - **包含**：`BackendRoster`、`owners/admins` 等重大网络属性
  - **不包含**：成员集/`decls head`（避免 approve/revoke 导致未过期 join code 频繁失效并与 `max_uses` 冲突）
- `expected_hash != actual_hash`：视为 join code 不匹配/已过期，**硬失败**。

## 非目标（明确排除）

- 不在 Door 3 引入 dedicated `coord server` 作为产品默认依赖（lab 入口另算）。
- 不在 Door 3 引入复杂的多 backend 负载均衡、动态路由或健康评分体系（先用超时 + 主备）。
- 不在 Door 3 引入 backend↔backend 的复制/同步机制。
- 不在 Door 3 直接承诺“任意聊天软件/任意第三方平台都能用”；backend 只要满足 capability/profile 即可。

## 开放问题（需要继续讨论，但先不下钻 wire）

- scheduled/store-and-poll 下的 candidate 新鲜度预算、keepalive 责任归属与失败口径。
- 第一批值得验证的 backend 组合（当前倾向：`MQTT + DHT`；另备选：`MQTT` 基线 + 一个状态型 backend）。
