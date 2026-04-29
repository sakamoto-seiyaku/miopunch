## Context

MNT-01 已落地主线二节点连接性矩阵（场景 1），并把 “真实 `miopunch` 主线节点 + MQTT-only + 证据链 + gate 分层” 固化为可回归入口。下一步进入场景 3（12 节点 NAT 综合网络）前，需要先补齐场景 2：用真实主线节点从空白启动完成控制面闭环，并把幂等性、权限边界与恢复语义固化为 required gate。

本 change 的被测对象仍然是主线 `miopunch` daemon/CLI（通过 LocalAPI 驱动任务）。测试环境提供自部署 MQTT broker（required），并可选提供 STUN、pcap、netem 等夹具能力。夹具不得预置 membership/decls/peers 或任务结果。

## Goals / Non-Goals

**Goals:**

- 提供 MNT-02 场景 2 的主线控制面 e2e runner 与分层 gate（至少 smoke/selftest）。
- 在“空白节点”上覆盖 `invite/approve/join`、多成员一致性、`ping`/`sh` smoke、restart、broker outage/recovery、revoke、幂等性与并发/竞态。
- 明确覆盖 “多成员同时访问同一被访问端” 的数据面并发能力：同一被访问端在已有会话存在时，仍应能接受并服务后续成员的 `ping/sh`（不得出现“第一个 session 独占 acceptor”）。
- 明确 revoke 的强语义边界：当某节点观察到有效 revoke tombstone 后，必须拒绝 revoked 节点的新操作，并主动切断其既有会话（不要求对端收到专门通知，允许对端被动观察到断开/失败）。
- 输出可复盘 artifacts：任务 report、daemon 日志、broker 日志、网络抓包与关键状态快照，并在 gate 内做最小证据校验。
- required gate 仅依赖测试环境自部署 MQTT broker；不引入 `coord`，不使用公网 broker。

**Non-Goals:**

- 不把 NAT1-4 矩阵或 Door-2 punching 稳定性问题重新引入场景 2 gating（场景 2 默认使用简单网络画像，保证控制面可稳定回归）。
- 不验证场景 3 的多节点 overlay、邻居维护与扰动恢复。
- 不在本 change 中顺手修复测试暴露的产品缺陷（按 findings/独立 fix change 处理）。

## Decisions

1. 采用 lab guest 内的 netns 小网络作为场景 2 的运行底座

Rationale: 与 MNT-01 复用同一套 “可控拓扑 + 自部署 broker + artifacts” 的工程路径，避免把场景 2 变成另一套 Docker/systemd 体系。

2. 场景 2 默认使用 “单 NAT + 多 peer 同 LAN” 的简单画像

Rationale: 场景 2 的重点是控制面闭环与恢复语义，而非 NAT traversal 成功率。简单画像能显著降低 flakiness，并缩短迭代周期；复杂 NAT 组合由 MNT-01/MNT-03 覆盖。

3. 新增 `miopunch-lab mnt02-seed` 用于生成多 peer 的初始 state

Rationale: `pocstate` 的默认 MQTT broker 是公网地址；required gate 必须显式配置自部署 broker。seed 命令负责为 N 个 peer 生成 identity 与最小 `state.json`（local config：`mqtt_broker/topic_prefix/p2p_port/data_proto/quic_cc/stun` 等），但不得生成 net/governance/membership 或预置 peers。

4. Runner 以 LocalAPI 驱动 `miopunch` 任务并收集 report/export

Rationale: 场景 2 的验收主体是产品 CLI/任务合约（stage/reason_code/facts/suggestions/report/redaction）。runner 应使用 `miopunch ... --format json --report ... --redact <cmd>` 执行并在 artifacts 中保存输出，用于 gate 内证据校验与事后排障。

5. Gate 分层与覆盖边界

- smoke: 最小闭环（`up -> invite/approve/join -> ping`）+ 1 个 `sh` smoke。
- selftest: 在 smoke 基础上增加多成员、一致性、幂等性、restart、broker outage/recovery、revoke 与并发覆盖。

## Risks / Trade-offs

- [Risk] 多 peer netns 拓扑与进程编排复杂，容易引入 harness 误报。→ Mitigation: 先固定 N=6 的最小可用拓扑；每个 case 统一输出 “case.env + attempts.tsv + summary.json + logs/pcap”。
- [Risk] 控制面任务存在真实时间窗口（invite expiry / backoff），导致用例不稳定。→ Mitigation: case 统一设置较短但足够的 expires/budget；避免依赖 wall-clock 偶然性；对并发用例使用 bounded retry 并记录事件。
- [Risk] 场景 2 scope 膨胀导致 gate 过慢。→ Mitigation: 明确 smoke/selftest 边界；重 case（broker outage/recovery、并发）放入 selftest。
