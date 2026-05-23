## Context

POC v1 的 GUI 不是“把内部状态机都画出来”，而是让用户完成闭环：建网/入网 -> 发现 -> 打洞 -> 建会话 -> ping/shell。

依赖：前面所有 v1 changes（控制面/入网/发现/punch/session）。

## Goals / Non-Goals

**Goals:**
- 线性 Wizard：每个 stage 有明确开始/成功/失败。
- 每个 stage 默认只展示三行人话总结；证据/细节进入 Evidence 面板。
- reason_code 枚举固定 <=12。

**Non-Goals:**
- 不做多导航层级、不做“全量 debug 面板当产品 UI”。

## Decisions

- stage 固定为 6 个；不允许加“半 stage”。
- evidence 必须可导出（便于面试/复盘）。

## Owned Paths (planned)

- `internal/task/desktop_state.go`（或迁移到新的 desktop model 包）
- `cmd/miopunch-desktop/*` / `desktop/*`（按当前 repo 实际结构调整）
