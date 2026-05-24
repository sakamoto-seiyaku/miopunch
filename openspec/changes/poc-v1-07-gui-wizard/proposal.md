## Why

如果前面的 `01..06` 都抽出来了，但最终桌面仍然由 legacy `internal/task` 状态拼图驱动，那这轮“完全能解释、完全能运行”的 POC v1 还是不成立。07 的职责是把新主线收束成唯一用户入口，并把 stage/runtime authority 从旧任务管理器中剥出来。

## What Changes

- 将 07 重写为 `internal/pocv1/runtime` 加平行 `/api/v1/poc/runtime` 桌面接口的抽离蓝图。
- 定义并后续实现六阶段 Wizard、`UserSummary`、`Evidence`、用户面 `UserReasonCode` 上限，以及从 `02/03/04/05/06` 消费 typed contracts 的 runtime。
- 明确可以复用现有 `cmd/miopunch-desktop`、`internal/localapi`、`internal/desktopbridge` 作为 desktop/plumbing 壳，但 v1 stage/runtime source of truth 必须迁移到新栈。
- 为 07 增加 Linux/Windows 真实桌面 smoke 验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-gui-wizard`: 当前 POC v1 的 desktop wizard 与最终闭环入口。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/runtime/*`
- 计划复用的壳层：`cmd/miopunch-desktop/*`、`internal/localapi/*`、`internal/desktopbridge/*`
- 不重新定义底层 wire/persist/punch/session 语义；07 只消费前序 typed contracts。
