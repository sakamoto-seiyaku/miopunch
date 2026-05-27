## Why

如果 pre-07 的 headless runtime 已经成立，但最终桌面仍然继续直接拼 legacy `internal/task` 或 `/api/v0/desktop/state`，那这轮 `POCv1` 仍然没有真正统一入口。07 的职责不再是补齐 runtime authority，而是把已经成立的 extracted-v1 runtime 收束成默认 GUI/desktop 入口。

## What Changes

- 将 07 收缩为消费 `miopunch-poc-v1-headless-runtime` 的 GUI wizard / desktop bridge / frontend change。
- 定义并后续实现六阶段 Wizard 的 GUI 呈现、desktop shell 入口、runtime snapshot/event/action 消费，以及 GUI 特有的 view-state 行为。
- 明确可以复用现有 `cmd/miopunch-desktop`、`internal/localapi`、`internal/desktopbridge` 作为 desktop/plumbing 壳，但 extracted-v1 stage/runtime source of truth 已由 pre-07 runtime 拥有。
- GUI 直接连接 shared daemon 的 `localapi` IPC endpoint，不通过 CLI 进程桥接。
- 为 07 增加 Linux/Windows 真实桌面 smoke 验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-gui-wizard`: 当前 POC v1 的 desktop wizard 与默认 GUI 入口。

### Modified Capabilities

- (none)

## Impact

- 计划新增或重接线代码：`cmd/miopunch-desktop/*`、`internal/desktopbridge/*`
- 计划消费的上游 contract：`internal/pocv1/runtime/*`、shared daemon `localapi` RPC / stream contracts
- 不重新定义底层 wire/persist/punch/session/runtime 语义；07 只消费前序 typed contracts。
