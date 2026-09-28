# Design: xtcp-kernel

## Summary

`xtcp-kernel` 是一个以“中心协调 + 两端数据面”为主流程的 NAT traversal 内核。
本阶段只解决“能稳定建链且失败可解释”的最小问题，将连通性增强与高性能传输延后到 `P2/P3`。

## Upstream baseline / source record

- `frp/` submodule revision:
  - commit: `94a631fe9c22491672b016413bb4d68067adeafb`
  - describe: `v0.62.1-93-g94a631fe`
- Copy-first extracted packages (Apache-2.0 headers preserved):
  - `frp/pkg/nathole/*` → `xtcp/nathole/*`
  - `frp/pkg/transport/{message.go,tls.go}` → `xtcp/transport/{message.go,tls.go}`
  - `frp/pkg/util/util/util.go` (subset) → `xtcp/util/util/auth.go`
  - `frp/pkg/util/log/log.go` → `xtcp/util/log/log.go`
  - `frp/pkg/util/xlog/*` → `xtcp/util/xlog/*`
  - `frp/pkg/util/net/{kcp.go,udp.go}` → `xtcp/netutil/*`

Deviations from upstream are intentionally kept minimal and driven by testability:
- Lab glue supports binding a fixed local UDP port for NAT traversal (without modifying upstream-extracted `frp/pkg/nathole` code).
- Data plane servers keep sockets alive briefly after payload exchange to avoid early-close failures in regression.

## Upstream Diff Summary (code review view)

Focus: keep `hole punching kernel` copy-first and minimize diffs against `frp/` baseline.

- `xtcp/nathole/*` vs `frp/pkg/nathole/*`
  - Behavior: unchanged (mode selection, classify/discovery/analyzer logic are identical).
  - Changes in upstream-copied files are limited to:
    - import path rewrites (`github.com/fatedier/frp/...` → `github.com/miopunch/miopunch/xtcp/...`)
    - drop `k8s.io/apimachinery/pkg/util/sets` dependency (use `map[int]struct{}`) in random-port sender helper
  - Additions: unit tests (`xtcp/nathole/*_test.go`)
- `xtcp/transport/message.go` vs `frp/pkg/transport/message.go`
  - import path rewrite only; logic unchanged
  - Additions: unit tests (`xtcp/transport/message_test.go`)
- Glue code (`xtcp/peer/*`, `xtcp/control/*`, `xtcp/coord/*`, `xtcp/msg/messages.go`) is intentionally *not* copy-first; it provides a minimal runnable harness around the upstream kernel.

Repro commands:
- `git submodule status`
- `git diff --no-index --stat frp/pkg/nathole xtcp/nathole`
- `git diff --no-index --stat frp/pkg/transport xtcp/transport`

## Key Decisions

### Copy-first extraction

- `P1` 采用“直接从 `frp/`（submodule）复制/抽离”方式落地，优先保持与上游结构相近，避免大规模重写与重构。
- 对复制进来的上游文件，保留原始版权与许可证头部，并记录来源 commit/tag；对上游行为的偏离必须可解释并用测试覆盖。
- `control plane` 协议优先采用“最小可用子集”，优先复用并裁剪 `frp` 现有消息与流程；不足之处后续再演进。

### Reference baseline via submodule

- `frp/` 以 `git submodule` 固定版本作为参考与对照基线。
- 内核实现不依赖 `frp` 包作为运行时依赖；任何“对齐/偏离”需要可解释并体现在文档与测试中。

### Separation of concerns

- 明确区分 `control plane`（协调与信息交换）与 `connectivity kernel`（建链状态机）。
- 将“建链成功”与“面向性能的传输层优化”解耦：`P1` 只要求最小数据收发验证，允许 `control plane` 使用 `KCP / QUIC` 作为传输选项，并在 P2P 数据面支持 `KCP / QUIC` 作为基线传输；性能调度优化留给后续阶段。

### Testability-first

- 所有超时与重试策略必须可注入（时间源/参数可控），以支持确定性单元测试。
- 与 `P0` 实验台的集成回归是主线验收方式之一：不仅验证成功/失败，还验证诊断信息完整性。

### Observability as a hard constraint

- 建链过程必须按阶段输出可机读事件流，失败必须携带明确阶段与关键条件。
- `P1` 不实现 `fallback/relay`；直连失败就失败，但失败路径必须可观测、可定位、可复盘。
