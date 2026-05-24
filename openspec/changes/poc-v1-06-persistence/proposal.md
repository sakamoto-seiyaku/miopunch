## Why

如果 `poc-v1` 继续把状态散落在 `internal/pocstate`、task 运行时和 GUI 私有模型里，后面的 enroll/discover/dial/session 都会继续互相绕。06 必须先落地，因为它是这轮并行抽离的地基。

## What Changes

- 将 06 重写为 `internal/pocv1/persist` 的抽离蓝图，并把它设为 `01..07` 的首个实现顺序。
- 定义并后续实现设备与网络状态布局、文件职责、单 broker 配置、`roster_snapshot`、`TopicScope`、原子写、权限语义和 typed persist APIs。
- 明确 legacy `internal/pocstate` 只作为参考，不再承接新的 v1 持久化 authority。
- 为 06 增加独立的 cold-start、rewrite、restart 和权限验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-persistence`: 当前 POC v1 的本地持久化 authority。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/persist/*`
- 计划参考的 legacy 行为：`internal/pocstate/*`、`internal/controlplane/file_atomic.go`
- 后续 `02/03/04/07` 都必须通过 06 APIs 读写状态或派生 topic scope。
