## Why

现有 NAT lab 和 POC e2e 已能分别验证底层夹具、代表性打洞路径和产品控制面，但还没有一套以真实主线节点为被测对象的二节点连接性矩阵。现在需要先把场景 1 做成可复现、可诊断、可分层运行的 required gate，作为后续场景 2 和 12 节点场景 3 的前置基础。

## What Changes

- 新增 MNT-01 主线连接性矩阵：用真实 `miopunch` 主线节点替换旧 lab peer 作为连接性验收对象。
- 统一使用测试环境自部署 MQTT broker 作为唯一主线信令入口；`coord` 不进入本 change，也不作为 fallback。
- 定义二节点 fixture 契约：夹具只提供最小 identity、peer、hello/auth bootstrap、MQTT/STUN endpoint 和网络画像，不得预置 NAT 结论、候选路径、邻居状态或成功缓存。
- 覆盖 UDP 5 类画像的无向 15 类组合，以及 TCP 7 类画像的有向 49 类组合。
- 将 `auto` 路径选择、IPv6、portmap、loss/netem、blocked、STUN unavailable 和 transport variants 作为专项覆盖，不与主矩阵无限笛卡尔积。
- 建立结果分类与证据链：`success-required`、`success-preferred`、`diag-fail-allowed`、`fail-required`，并要求 MQTT signaling、candidate discovery、attempt path、payload evidence 或 failure reason。
- 对 TCP hard/irregular 组合允许 bounded repeat/retry 下的可解释失败，但不允许静默跳过、无限重试或无诊断失败。
- 测试过程中发现的非测试自身项目代码问题记录到 `docs/notes/mainline-network-test-findings.md`，不在本 change 中混修。

## Capabilities

### New Capabilities

- `miopunch-mainline-connectivity-matrix-v0`: 定义场景 1 主线二节点连接性矩阵，包括 MQTT-only 信令、自部署 broker、fixture 契约、UDP/TCP profile 矩阵、结果分类、证据要求和 gate 分层。

### Modified Capabilities

- None.

## Impact

- Affected lab/runtime:
  - 后续 apply 阶段将新增或扩展主线连接性测试入口、二节点 fixture、profile/case 生成、artifact 收集和 gate 命令。
- Affected validation:
  - 后续 apply 阶段应提供 smoke/selftest/fulltest 三层 gate，并保留 required/full 矩阵的清晰边界。
- Affected docs/specs:
  - 新增 OpenSpec capability `miopunch-mainline-connectivity-matrix-v0`。
  - 以 `docs/decisions/mainline-network-test-charter.md` 和 `docs/roadmap.md` 为事实源。
- Out of scope:
  - 不实现场景 2 控制面 e2e。
  - 不实现场景 3 的 12 节点 NAT 综合网络。
  - 不测试 invite/join/governance/multi-node overlay；fixture 中的 hello/auth bootstrap 只用于通过现有主线握手。
  - 不引入公网 MQTT required gate。
  - 不修复测试运行中暴露的产品代码问题。
