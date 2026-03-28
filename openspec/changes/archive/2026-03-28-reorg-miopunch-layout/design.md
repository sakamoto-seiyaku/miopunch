## Context

当前仓库仍以顶层 `xtcp/` 目录承载核心实现（`control/coord/msg/peer/nathole/connectivity/obs/...`），而 `cmd/miopunch` 直接 import 这些包。
这带来两个直接问题：

- **命名割裂**：对外项目名是 `miopunch`，但核心代码命名空间是 `xtcp`，后续会持续扩散这种割裂。
- **职责混杂**：`xtcp` 下存在多个“杂物桶”（例如 `util`、`nathole` 内部多职责混放），使得 `P3` 需要的分层（`connectivity` vs `dataplane`）很难在现有结构上干净落地。

`docs/decisions/p3-miopunch-transport-charter.md` 已明确：`P3` 的结构与命名迁移必须先于数据面新能力接入执行。

## Goals / Non-Goals

**Goals:**
- 移除顶层 `xtcp/` 命名空间，收敛为 `miopunch` 语义导向的 Go 包布局。
- 明确顶层领域包与 `internal/` 的边界：
  - 顶层领域包：`connectivity/`、`event/`、`nat/`、`stun/`
  - 内部实现包：`internal/control/`、`internal/coordinator/`、`internal/wire/`、`internal/netutil/`、`internal/peer/`、`internal/punching/`、少量 `internal/*util/`
- 保持行为不漂移：`go test ./...` 必须通过，`P0` 实验台与现有 runner 不应因目录变化失效。
- 更新持续维护的文档（尤其是 `docs/roadmap.md`）以对齐新命名与正确的 change 路径引用。

**Non-Goals:**
- 不在本 change 引入 `dataplane` 抽象、`brutal`、或 QUIC fork 迁移（这些由后续 change 承担）。
- 不做对外兼容层：不保留 `xtcp/` 作为 deprecated wrapper（避免继续扩散旧命名）。
- 不机械重写历史报告/历史决策文档。
- 不改变线协议、NAT 策略、超时默认值或 CLI flag 语义（除非为移除 `xtcp` 对外术语所必需）。

## Decisions

- **迁移方式**
  - 使用 `git mv` 做目录移动以保留历史，并在同一变更中完成 import 更新与 `gofmt`。
  - 以“可编译可测试”为节奏分段实施，但避免引入同时存在的两套实现（不做 `xtcp` wrapper）。

- **目标布局（映射原则）**
  - `xtcp/connectivity` → `connectivity/`
  - `xtcp/obs` → `event/`
  - `xtcp/stun` → `stun/`（server 保持顶层；client/discovery 仍归属 connectivity）
  - `xtcp/control` → `internal/control/`
  - `xtcp/coord` → `internal/coordinator/`（含 coordinator server 侧逻辑）
  - `xtcp/msg` + `xtcp/transport/message.go` → `internal/wire/`
  - `xtcp/transport/tls.go` → `internal/tlsutil/`
  - `xtcp/netutil` → `internal/netutil/`
  - `xtcp/peer` → `internal/peer/`（P3 后续再拆分出 `dataplane/`）
  - `xtcp/nathole`：
    - NAT 分类/分析 → `nat/`
    - 打洞内核实现细节 → `internal/punching/`
    - 端侧流程对外语义（后续）→ `connectivity/`（本 change 以不漂移为优先，避免过度重切）
  - `xtcp/util/**`：不保留新的泛化 `util/`，按语义并入拥有者或收敛到少量 `internal/*util/`。

- **对外入口与术语**
  - `cmd/miopunch` 保持单入口，但 help/usage 文案不再对外暴露 `xtcp`。
  - 事件输出（machine-readable diagnostics）保持现有语义与稳定格式，避免因目录迁移造成回归校验漂移。

## Risks / Trade-offs

- [大规模移动导致 merge 冲突] → 将本 change 作为 `P3` 第一 change，后续 dataplane/brutal change 在其基础上 rebase；实施阶段尽量保持提交聚焦与可 review。
- [行为漂移风险] → 每个里程碑节点跑 `go test ./...`，并在 `P0` 实验台跑最小 smoke（例如 `core-01`）验证入口未断。
- [短期内 internal 边界不完美] → 本 change 优先“去掉 xtcp 命名空间 + 建立新骨架”；更细的拆分（尤其 dataplane 抽象与更深的 nathole 拆分）放到后续 change。

## Migration Plan

1. 建立新目录骨架（`connectivity/`、`event/`、`nat/`、`stun/`、`internal/**`），并按映射进行 `git mv`。
2. 更新 Go import 路径与包名；保持外部行为与 flag 语义不变。
3. 收敛 `cmd/miopunch` 的 help 文案，去除对外 `xtcp` 术语。
4. 运行 `go test ./...` 并修复受影响的单元测试。
5. 更新 `docs/roadmap.md` 等持续维护文档的命名与 change 路径引用。
6. 在 `P0` 实验台执行最小 smoke（至少 `core-01`）确认入口仍可用。

## Open Questions

- `xtcp/nathole` 的进一步拆分深度：本 change 只做最小不漂移迁移，还是顺带拆得更干净？（默认：先最小迁移，后续 change 再深化）
- 是否在同一 change 内同步重命名 OpenSpec capability（`xtcp-kernel/xtcp-connectivity`）？（默认：不做，避免与其他 change 的 delta spec 冲突）

