## Context

POC v1 的 GUI 不是“把内部状态机都画出来”，而是让用户完成闭环：建网/入网 -> 发现 -> 打洞 -> 建会话 -> ping/shell。

依赖：前面所有 v1 changes（控制面/入网/发现/punch/session）。

## Scope

- GUI 独占 Wizard stage model、stage transitions、`UserSummary`、`Evidence`、`reason_code`。
- stage 固定为 6 个：`Network / Enroll / Discover / Punch / SecureSession / Shell`。
- 每个 stage 默认只展示三行人话总结；证据/细节进入 Evidence 面板，并支持导出。
- 本 change 消费 03/04/05/06 提供的数据契约，但不回头定义 presence、persist、punch、session 的底层语义。
- 不做多导航层级，不把全量 debug 面板当产品 UI。

## Owned Paths (planned)

- `internal/task/desktop_state.go`（或迁移到新的 desktop model 包）
- `cmd/miopunch-desktop/*`
- `desktop/*`

## Done

- 6-stage wizard、summary/evidence contract、reason_code budget 冻结完成。
- GUI 成为 peer list、punch evidence、session state 的唯一渲染归口。
- 底层控制面、持久化、打洞、会话边界继续由前序 changes 拥有。
