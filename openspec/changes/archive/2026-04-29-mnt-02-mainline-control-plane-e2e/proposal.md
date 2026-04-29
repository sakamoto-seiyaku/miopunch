## Why

MNT-01（场景 1：主线连接性矩阵）已具备稳定的二节点连接回归能力，但它刻意不覆盖 `invite/approve/join`、governance/decl 同步、权限边界与恢复语义。进入更大规模的场景 3 之前，需要先把场景 2（主线控制面 e2e）固化为可复现、可诊断、可分层运行的 required gate，用真实主线 `miopunch` daemon/CLI 从“空白启动”走完建网、成员变更、恢复与报告合约。

## What Changes

- 新增 MNT-02：场景 2 主线控制面 e2e 测试入口，使用真实 `miopunch` 主线节点作为被测对象。
- 统一使用测试环境自部署 MQTT broker 作为 required signaling 入口；不引入 `coord`，也不作为 fallback。
- 定义场景 2 fixture 契约：夹具只负责网络拓扑与基础设施（broker、可选 STUN、日志/pcap 等）以及最小运行参数，不得预置 membership、decls、peers 或任务结果。
- 固化 required 覆盖集（以 `miopunch` CLI/LocalAPI 为入口）：
  - blank `up`、`invite/approve/join` 闭环
  - 多成员一致性与权限边界（wrong actor / revoke）
  - `ping` 与 `sh` smoke（证明控制面闭环后的最小数据面可用）
  - restart、broker outage/recovery、幂等性与并发/竞态
  - report/export 与 redaction 合约
- 提供分层 gate（smoke/selftest/...）与 artifacts 汇总，确保日常回归成本可控。

## Capabilities

### New Capabilities

- `miopunch-mainline-control-plane-e2e-v0`: 定义场景 2 主线控制面 e2e 的 fixture 边界、required cases、证据要求与 gate 分层。

### Modified Capabilities

- None.

## Impact

- Affected lab/runtime:
  - 后续 apply 阶段将新增场景 2 的 runner、case 编排、artifact 收集与汇总 gate 命令（与 MNT-01 并列）。
- Affected validation:
  - 后续 apply 阶段应提供至少 smoke/selftest 两层 gate；并明确“必需通过”的 required 子集。
- Affected docs/specs:
  - 新增 OpenSpec capability `miopunch-mainline-control-plane-e2e-v0`。
  - 以 `docs/decisions/mainline-network-test-charter.md`（场景 2）与 `docs/roadmap.md` 为事实源。
- Out of scope:
  - 不引入公网 MQTT required gate。
  - 不把场景 3 的 12 节点 NAT 综合网络提前混入。
  - 测试过程中发现的产品缺陷按问题清单单独修复（不在测试 change 中顺手混修）。

