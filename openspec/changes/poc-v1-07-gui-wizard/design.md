## Context

07 是整套 `poc-v1` 的最终闭环入口，但不是“兜底把所有前序语义重新做一遍”的大杂烩。它只消费前序 typed contracts，把它们按固定六阶段组织成用户能理解的流程。

## Extraction Strategy

- 新的 stage/runtime authority 进入 `internal/pocv1/runtime`。
- `internal/pocv1/runtime` 是从前序 typed failures 到 12 个 `UserReasonCode` bucket 的唯一最终映射者。
- `internal/localapi` 为当前提取态暴露平行 `/api/v1/poc/runtime` 与 `/api/v1/poc/runtime/events`。
- `cmd/miopunch-desktop`、`internal/localapi`、`internal/desktopbridge` 可继续作为桌面壳与 IPC/plumbing。
- legacy `internal/task/desktop_state.go` 不再是 v1 GUI 的最终事实源；它只可作为旧行为对照。

## Scope

**07 owns:**

- 六阶段 Wizard：
  - `Network`
  - `Enroll`
  - `Discover`
  - `Punch`
  - `SecureSession`
  - `Shell`
- stage transitions
- `SecureSession` 只有在一次成功的 identity-bound `ping/hello` 之后才允许转入 `Shell`
- `UserSummary`（每阶段最多三行）
- `Evidence`（可折叠/可导出，且至少包含 `facts[]` / `suggestions[]`，可附带额外 diagnostics）
- `UserReasonCode`（固定 12 个用户面 bucket，以及从前序 typed failures 到这些 bucket 的最终映射）
- 平行 v1 runtime DTO / API
- wizard-local `ui_state` shape（如果 07 选择持久化该状态）
- 从前序 change 消费 typed contracts 组装 runtime state

**07 does not own:**

- wire/security 语义（`01`）
- bootstrap 对象与 authority 流程（`02`）
- presence source-of-truth（`03`）
- punch attempt runtime（`04`）
- secure session recipe（`05`）
- persist layout authority（`06`）
- 前序 capability 的底层 failure 语义本体；它们只提供 typed failure/evidence 输入，不直接拥有最终用户面 bucket 决策。

## Owned Paths (planned)

- `internal/pocv1/runtime/*`
- `cmd/miopunch-desktop/*`
- `cmd/miopunch-desktop/frontend/*`

## Task Breakdown

1. 在 `internal/pocv1/runtime` 中建立 stage model、summary/evidence model（显式 `facts[]` / `suggestions[]`）与 reason-code budget/final mapping。
2. 将 desktop runtime 改为通过平行 `/api/v1/poc/runtime` 读取 `02/03/04/05/06` typed contracts，而不是直接拼 legacy task internals。
3. 复用现有 desktop/localapi/desktopbridge 作为 transport/plumbing 壳。
4. 增加 Linux/Windows 桌面 smoke、stage progression、`SecureSession` ping gate、Evidence export 测试。

## Acceptance

- 用户可从 `Network -> Enroll -> Discover -> Punch -> SecureSession -> Shell` 线性完成闭环，且 `SecureSession` 必须先完成一次成功 ping/hello。
- 每阶段默认只看到 <=3 行 summary；详细信息只在结构化 Evidence（至少有 `facts[]` / `suggestions[]`）。
- GUI 不再把全量 debug/task internals 当作默认产品状态展示。
