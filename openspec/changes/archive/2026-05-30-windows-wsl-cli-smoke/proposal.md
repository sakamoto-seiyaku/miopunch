## Why

Windows 与 WSL 的真实 CLI 闭环现在比 GUI 更重要，但现有排查主要混在桌面交互里，导致 join 失败、日志等级和 daemon 取证都不够直接。需要一条只用 CLI、可重复、可取证的 Windows/WSL smoke 路线来确认 `up -> init-network -> invite -> approve -> join` 是否真的能跑通。

## What Changes

- 新增一个 Windows/WSL CLI smoke change，只验证 CLI 正向闭环，不再依赖 GUI 测试。
- 把验证范围收敛到两条双向矩阵：
  - Windows 建网，WSL join
  - WSL 建网，Windows join
- Smoke 必须强制收集取证材料：CLI stdout/stderr、`--report`、daemon log、state snapshot、run metadata。
- Smoke 必须明确记录失败阶段、`reason_code`、`facts` 和 `suggestions`，避免只看到“join 失败”。
- Smoke 必须使用 session bundle 产物和可重复的隔离目录，避免污染既有环境。

## Capabilities

### New Capabilities
- `windows-wsl-cli-smoke`: Windows/WSL CLI join smoke with bidirectional positive-path coverage and diagnostics capture.

### Modified Capabilities
- None.

## Impact

- OpenSpec: new change under `openspec/changes/windows-wsl-cli-smoke/`
- Lab / realtest docs: add a CLI-first Windows/WSL runbook
- Host/guest smoke wiring: reuse existing `poc-v1-cli-smoke` mechanics where possible
- No GUI behavior change is intended in this change
