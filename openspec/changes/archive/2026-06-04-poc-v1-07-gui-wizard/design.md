## Context

07 是 `POCv1` 的默认 GUI/desktop 入口，但不再是 runtime authority 的落地点。它只消费 pre-07 headless runtime 提供的 snapshot / events / actions，并把这些 runtime-owned 状态组织成更接近归档版 GUI 的 operator control console。

## Extraction Strategy

- `internal/pocv1/runtime` 与 shared daemon `localapi` RPC / stream contract 已由 `miopunch-poc-v1-headless-runtime` 拥有。
- 07 只消费 pre-07 runtime 的 snapshot / events / actions，不再重新定义 stage model、final reason-code mapping、DiscoverView projection 或 shell gate authority。
- `cmd/miopunch-desktop`、`internal/localapi`、`internal/desktopbridge` 可继续作为桌面壳与 IPC/plumbing。
- legacy `internal/task/desktop_state.go` 与 `/api/v0/desktop/state` 不再是 extracted-v1 GUI 的最终事实源；它们只可作为旧行为对照或兼容壳。

## Scope

**07 owns:**

- 四个 operator tabs：
  - `Network`
  - `Shell`
  - `Admin`
  - `Settings`
- GUI control-console rendering and desktop flow composition
- desktop consumption of runtime snapshot / events / actions as presentation inputs
- console-local `ui_state` shape（如果 07 选择持久化该状态）
- desktop shell workspace and attach UX
- 从 pre-07 runtime 消费 typed contracts 的投影结果，并转换成 GUI view state
- direct GUI-to-daemon connection over `localapi` IPC

**07 does not own:**

- wire/security 语义（`01`）
- bootstrap 对象与 authority 流程（`02`）
- presence source-of-truth（`03`）
- punch attempt runtime（`04`）
- secure session recipe（`05`）
- persist layout authority（`06`）
- headless runtime authority、stage transitions、`SecureSession -> Shell` gate、本体 `UserSummary` / `Evidence` / final user-facing `reason_code` mapping（`06x`）
- 前序 capability 的底层 failure 语义本体；它们只提供 typed failure/evidence 输入。

## Owned Paths (planned)

- `cmd/miopunch-desktop/*`
- `cmd/miopunch-desktop/frontend/*`
- `internal/desktopbridge/*`

## Task Breakdown

1. 将 desktop runtime 改为通过 shared daemon `localapi` 的 RPC / event / shell stream 通道消费 headless runtime，而不是直接拼 legacy task internals 或经 CLI 桥接。
2. 在桌面端实现四个 operator tabs，并在其中呈现 runtime summary/evidence、peer/shell view state、invite/approve/join 操作与 desktop attach UX。
3. 保留 runtime-owned `stage` 作为状态展示和 shell gate 输入，而不是把它继续当作 GUI 导航模型。
4. 复用现有 desktop/localapi/desktopbridge 作为 transport/plumbing 壳。
5. 增加 Linux 桌面 smoke，验证 GUI 通过 headless runtime 完成 peer 选择、`SecureSession` ping gate 之后的 shell attach 与 Evidence export；Windows 桌面 smoke 只验证 GUI 启动、daemon 连接与 runtime contract consumption，不把 Windows/Linux 真机互连作为 07 的补偿职责。

## Acceptance

- 用户可通过 `Network / Shell / Admin / Settings` 四个 tab 完成 GUI 闭环，且 shell attach 只在 headless runtime gate 成功后发生。
- GUI 必须让最新 invite code 在创建后保持可见并可复制，不要求用户停留在单一步骤页。
- GUI 默认只展示简短 summary；详细信息通过 runtime 提供的结构化 Evidence 呈现。
- GUI 不再把全量 debug/task internals 当作默认产品状态展示，也不再自己拥有 runtime authority。
