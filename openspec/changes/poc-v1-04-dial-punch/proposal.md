## Why

POC v1 的核心体验在于：点对点连上并打开 shell。dial/punch 是闭环里最容易“卡住但说不清”的一段，因此必须强收敛：只做 UDP punching + 5B 有上限并发矩阵，并给 GUI 提供可解释的 Evidence。

## What Changes

- 定义并实现 `dial_offer/dial_answer` 的 v1 最小字段集（dial_id/punch_token/candidates/member_credential）。
- 定义 `PathResult` 边界：本 change 只产出选中的 UDP path、资源所有权、以及 punch evidence；session upgrade 归 `poc-v1-05-secure-session`。
- punch attempt 策略固定：最多并发 4 对 candidate pair，总预算 10s，先成功先收敛。
- Evidence 输出：candidate 表、尝试矩阵、超时原因聚合。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-dial-punch`: v1 dial/punch 的收敛口径（UDP only）。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：`internal/task/poc_dial.go` 的拆分与收敛、`internal/punching/*` 的 attempt 策略、以及到 `PathResult` 的交接面。
