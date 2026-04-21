## Context

Alpha/POC 控制面采用“mesh 优先 + MQTT 兜底”的思路：当没有邻居或无法直达时，通过 MQTT broker 做点对点投递的 mailbox。这里 broker 被视为不可信入口，因此：

- topic 不能可枚举（否则被扫描就等价于被投递垃圾/被观察元数据）。
- 每个 peer 必须有唯一 inbox topic（避免不同 peer 共享入口导致混淆与放大攻击面）。
- topic 必须可由网络内任意节点在“无中心分配”的前提下推导（便于恢复/换 broker/重装）。

此外，入网阶段（`invite/approve/join`）必须尽力保证 joiner 与 approver 命中同一 broker 实例。仅依赖 hostname + DNS/geo 分流会造成“双方落到不同实例互相看不见”，因此 join code 需要携带确定性的 `invite_brokers`（最多 1–2 个端点）作为本次入网的唯一入口集合。

## Goals / Non-Goals

**Goals:**
- 落地 inbox topic 的确定性派生与格式规范：
  - HKDF 的 `info` 必须包含 `peer_id`；
  - 输出使用 `base32(raw,no-pad)`；
  - 写入 MQTT topic 时统一小写。
- 收敛 join code 中的 broker pinning 口径：
  - code 内必须包含 `invite_brokers`（最多 1–2 个 `host:port`）；
  - `invite` 写入 code 前尽力将 hostname 固定为确定性的 `ip:port`（无法固定则强警告）；
  - `approve/join` 阶段严格使用 code 内的 `invite_brokers`。
- 提供最小可回归验证：
  - topic 派生的单元测试；
  - 一个本地/CI 可跑的 control-plane mailbox smoke（本地 broker 进程 + 订阅/投递）。

**Non-Goals:**
- 不在本变更定义或实现完整的控制面 wire format、签名覆盖、bounded flooding、去重/限流（属于后续 POC-03）。
- 不在本变更推进完整的端到端加密/鉴权/TLS 交付形态（仍坚持 broker 不可信，但安全细节在后续 change 逐步收敛）。
- 不引入多 broker 的复杂容错与健康探测策略（仅约束 join code 的“命中同一实例”最小语义）。
- 不一次性覆盖 presence/state 等其它派生 topic（本变更仅锁定 inbox/mailbox 基础约束与派生口径）。

## Decisions

### 1) Inbox topic 派生算法（POC v0，冻结）

- 输入：`net_secret`（raw bytes）与规范化后的 `peer_id`（字符串）。
- salt：`net_id_raw16 = sha256(net_secret)[:16]`（raw 16B）。
- HKDF：
  - `name16 = HKDF(net_secret, salt=net_id_raw16, info="miopunch/v0/topic.inbox/"+peer_id, L=16)`
  - 约束：`info` 必须包含 `peer_id`，避免不同 peer 共享同一入口。
- 输出：
  - `inbox_topic = base32(raw,no-pad,name16)`
  - 作为 MQTT topic name 时一律 `strings.ToLower(inbox_topic)`，并在 publish/subscribe 两侧都执行同一规范化。

备注：选择 `L=16` 的目标是确保 topic “有效熵 ≥128bit”，满足不可枚举的基本要求。

### 2) Join code 的 broker pinning：`invite_brokers`

- join code 内必须携带 `invite_brokers`（最多 2 个）：
  - 形态：`host:port`（可为域名或 IP；POC 默认语义为 `tcp://`，但 code 内不包含 scheme）。
  - 禁止：不在 code 内携带 broker 的用户名/密码/证书等材料。
- 取值来源（优先级）：
  1) 若本机 `up` 正在运行：优先取其 active brokers（最多 2 个）。
  2) 否则：取“生成 invite 时最终生效的 broker 列表”的前 1–2 个（显式 `control_plane.brokers` 优先；否则按 `broker_profile` 的内置默认顺序）。
- 端点规范化（为“命中同一实例”服务）：
  - 若端点为 hostname：按 `[resolver]` 解析 A 记录并固定为**一个** `ip:port`（取 resolver 返回的第 1 个）。
  - 若无法解析：保留 hostname 原样写入 code，但 `invite/approve/join` 必须输出强警告（提示该 code 成功依赖双方 DNS 结果一致，建议改用 `ip:port` 或显式配置 `control_plane.brokers`）。
- 使用策略：
  - `approve/join` 阶段仅使用 code 内的 `invite_brokers` 做订阅/投递与回包，避免双方落到不同 broker 实例。

### 3) 测试策略（对应 roadmap 的最小可验证）

- 单元测试：覆盖 topic 派生的确定性与隔离性（不同 `peer_id` 产生不同 inbox topic）。
- 集成 smoke：启动本地 MQTT broker 进程，验证两端：
  - 能订阅自身 inbox topic；
  - 能对对端 inbox topic publish 一条“不可解释明文”的 payload（具体加密/签名细节留给后续 change，但 smoke 至少保证 payload 非可读明文）。

## Risks / Trade-offs

- [peer_id 规范化不一致] → 在进入 topic 派生前对 `peer_id` 做统一规范化（去分隔符、统一大小写、校验长度/字符集），并用单测锁定。
- [hostname 固定到单一 IP 导致偶发不可达] → 优先使用 `up` 的 active brokers（已验证可连）；无法固定时保留 hostname 但强警告，让用户明确风险与替代方案。
- [topic 大小写不一致导致互相看不见] → publish/subscribe 两侧都执行 `lower-case` 规范化，并在测试中覆盖。

