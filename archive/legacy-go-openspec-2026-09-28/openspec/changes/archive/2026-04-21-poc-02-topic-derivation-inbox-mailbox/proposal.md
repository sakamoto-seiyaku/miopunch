## Why

Alpha/POC 控制面以 MQTT broker 作为“网外可达入口”，但 broker 被视为**不可信 mailbox**：我们不能依赖 broker 的隐私/鉴权语义，也不能让 topic 可被扫描枚举。为此需要把 inbox topic 设计为**从 secret 派生的高熵随机值**，并且做到“每个 peer inbox 唯一、且可由网络内任意节点自行推导”，避免中心分配与状态同步负担。

同时，入网流程的稳定性高度依赖“joiner 与 approver 命中**同一 broker 实例**”。仅靠 `broker_profile`/默认 hostname 很容易因 DNS/geo 分流落到不同实例而互相收不到消息。join code 必须显式携带本次入网要使用的 broker 端点信息，降低“看不见彼此”的偶发失败与排障成本。

## What Changes

- 控制面 inbox/mailbox topic 的确定性派生与约束落地：
  - inbox topic 派生必须包含 `peer_id`（避免不同 peer 订阅同一入口）。
  - topic 规范化：`base32(raw,no-pad)` 编码；写入 MQTT topic 时一律小写。
- join code 增加 broker 实例信息（`invite_brokers`）：
  - code 内必须包含 `invite_brokers`（最多 1–2 个 `host:port`）。
  - `invite` 写入 code 前尽力将 broker 端点规范化/固定为确定性的可连接地址（优先 `ip:port`）；无法固定时保留 hostname 并输出强警告。
  - `approve/join` 阶段严格使用 code 内的 `invite_brokers` 进行订阅/投递与回包，避免双方落到不同实例。
- 测试与可回归性：
  - 单元：topic 派生确定性测试 + 不同 `peer_id` 不同 inbox。
  - 集成：本地/CI 可复现的 control-plane smoke（本地 broker 进程或已有 lab broker 工具，验证“可订阅/可投递（密文）”）。

## Capabilities

### New Capabilities
- `miopunch-poc-control-plane-mailbox`: POC 控制面 mailbox/inbox 的 topic 派生与 join code broker pinning 约束（以 MQTT 兜底投递为主要驱动）。

### Modified Capabilities
- (none)

## Impact

- CLI / UX：
  - `cmd/miopunch`: `invite/approve/join` 的 join code 生成与解析需要新增/校验 `invite_brokers`，并在无法固定 hostname 时输出强警告。
- 控制面（实现侧）：
  - 派生 topic 的 helper（HKDF + base32 no-pad + lower-case）将成为 control-plane MQTT subscribe/publish 的基础能力。
  - join/approve 阶段的 broker 选择与并发收发策略需要以 `invite_brokers` 为输入。
- 状态与配置：
  - 需要把“最终生效 brokers（brokers_effective）”与“入网 brokers（invite_brokers）”的口径收敛，避免后续实现重复与歧义。

