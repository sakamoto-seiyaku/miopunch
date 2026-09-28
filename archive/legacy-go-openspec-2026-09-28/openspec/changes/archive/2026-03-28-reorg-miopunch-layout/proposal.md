## Why

`P3` 的前置条件之一是“先统一命名与目录，再继续叠加新能力（dataplane/brutal 等）”。
当前仓库仍以 `xtcp/` 为主命名空间，目录职责混杂且与 `miopunch` 项目名割裂，导致后续分层（`connectivity` vs `dataplane`）与依赖收敛成本持续上升。

## What Changes

- **BREAKING**：将 `github.com/miopunch/miopunch/xtcp/...` 下的核心实现按职责迁移到新的 Go 包布局（不再保留顶层 `xtcp/` 命名空间），并更新所有引用与 import。
- 按 `docs/decisions/p3-miopunch-transport-charter.md` 的映射建议，形成更 Go-idiomatic 的目录结构：
  - 对外领域包：`connectivity/`、`event/`、`nat/`、`stun/`
  - 内部实现包：`internal/control/`、`internal/coordinator/`、`internal/wire/`、`internal/netutil/`、`internal/peer/`、`internal/punching/`、以及少量 `internal/*util/`（避免继续扩散通用 `util/` 桶）
- `cmd/miopunch` 继续作为单二进制入口，帮助文案与 import 路径收敛为 `miopunch` 新结构（不再对外出现“xtcp”命名）。
- 更新会持续维护的文档以对齐新命名与新目录（例如 `docs/roadmap.md`）；历史报告/历史决策文档不做机械重写。
- 增加“目录收敛”回归护栏：
  - `go test ./...` 通过
  - `openspec validate --strict` 通过
  - 防止引入新的 `xtcp/` 顶层路径与对外术语

## Capabilities

### New Capabilities
- `miopunch-layout`: `P3` 第一阶段的仓库结构与命名收敛（目录重组 + import 收敛 + 文档对齐 + 护栏）。

### Modified Capabilities
- (none)

## Impact

- Affected code:
  - `xtcp/**`（整体迁移与拆分）
  - `cmd/miopunch/**`（更新 import 与帮助文案）
  - `lab/**`（如有硬编码路径/命名的脚本或验证器需要同步更新）
- Affected docs:
  - `docs/roadmap.md`（收敛阶段命名与 change 路径引用）
  - `docs/decisions/p3-miopunch-transport-charter.md`（作为布局迁移依据，不新增实现细节）
- Compatibility:
  - 本项目当前不以外部 import 稳定性为目标；该变更会破坏旧 `xtcp/` import 路径（预期且可接受）。

