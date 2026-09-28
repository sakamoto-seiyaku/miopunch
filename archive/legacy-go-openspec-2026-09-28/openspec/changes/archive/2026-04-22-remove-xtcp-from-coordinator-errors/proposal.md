## Why

当前 `coordinator` 返回给 peer 的部分错误文案仍包含历史遗留的 `xtcp` 命名（例如 “xtcp server ...”）。这会造成项目对外语义割裂、误导实验使用者，并与 “prefer miopunch naming; avoid introducing new xtcp names” 的仓库规则冲突。

本变更以最小范围修复：只调整错误文案，不改变任何打洞/交换/分析行为。

## What Changes

- 将 `internal/coordinator/nathole_controller.go` 中对外可见的错误文案从 `xtcp ...` 收敛为 `miopunch ...`（或更中性的 `proxy ...` / `peer ...` 语义）。
- 维持错误语义一致：仍然能表达 “proxy 不存在 / visitor 不被允许 / auth 失败” 等原因。
- 基础门禁验证：`gofmt`、`go test`、`go vet`、`check_no_xtcp_imports`。

## Capabilities

### New Capabilities
- `miopunch-coordinator-errors`: 约束 coordinator 对外错误信息的命名一致性（不得暴露 `xtcp` 残留），便于实验输出与文档一致。

### Modified Capabilities
- (none)

## Impact

- Affected code: `internal/coordinator/nathole_controller.go`
- No protocol/data model changes; error strings only.

