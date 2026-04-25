## Why

近期已经连续引入并完成多批 change，但对这些 change 之前与期间累积的 Go 代码缺少一次专门、系统的 review。现在需要创建一个独立的 review-only change，用于在后续 apply 阶段发现并记录问题，而不是在创建 change 时直接开始审核或修复。

## What Changes

- 新增一个全量 Go code review 的 OpenSpec change，覆盖当前仓库已有 Go 代码、测试、`cmd/`、`tools/` 和关键执行脚本的审计计划。
- 明确 apply 阶段才开始实际 review：运行自动检查、逐包阅读代码、复核问题、生成 `findings.md`。
- 明确本 change 的产出是问题清单，不包含修复；任何修复都应在后续独立 fix change 中处理。
- 明确创建阶段只生成 OpenSpec artifact，不运行代码审核命令，不生成 findings。

## Capabilities

### New Capabilities

- `miopunch-code-review-v0`: 定义一次 review-only Go 代码审计的执行边界、问题报告格式和验收要求。

### Modified Capabilities

- None.

## Impact

- Affected code:
  - None during change creation.
  - Apply 阶段只读取/检查 Go 代码并生成 review 报告，不修改 Go 源码、测试或运行脚本。
- Artifacts:
  - `openspec/changes/review-current-go-code/proposal.md`
  - `openspec/changes/review-current-go-code/design.md`
  - `openspec/changes/review-current-go-code/specs/miopunch-code-review-v0/spec.md`
  - `openspec/changes/review-current-go-code/tasks.md`
  - `openspec/changes/review-current-go-code/findings.md`（apply 阶段生成）
- Validation:
  - 创建阶段只验证 OpenSpec artifact 完整性。
  - apply 阶段再执行自动检查与 findings 复核。
