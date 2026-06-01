## Why

`dial/punch` 是这轮最容易再次被旧实现拖乱的部分：当前仓库已经有 punching、connectivity、session、task 编排的混合物。如果 04 不把“协商层”和“尝试层”抽成一条干净的 UDP-only 主线，后面只会继续在旧栈里缝合。

## What Changes

- 将 04 重写为 `internal/pocv1/punch` 的抽离蓝图。
- 定义并后续实现 `dial_offer/dial_answer` body、基于 trusted roster + `TopicScope` 的 inbox topic 投递、固定 5B attempt matrix、punch evidence 与 `PathResult`。
- 明确允许复用 legacy `internal/punching` / `internal/punchwire` / `connectivity` 的叶子机制，但不允许把 legacy task/runtime/orchestration 整包搬入 v1。
- 为 04 增加独立的两节点 MQTT+UDP smoke 与 evidence 验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-dial-punch`: 当前 POC v1 的 UDP-only dial/punch 合同。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/punch/*`
- 计划参考或窄复用的 legacy 叶子实现：`internal/punching/*`、`internal/punchwire/*`、`connectivity/*`
- 不包含 KCP/TLS/yamux 建会话、GUI stage runtime 或 topology/neighbor maintenance。
