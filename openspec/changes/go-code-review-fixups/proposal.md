## Why

随着 `P3/P3.5` 的能力逐步补齐（连通性编排、MQTT 交换信道、data plane 传输等），当前仓库已经具备“能跑通实验 + 有可观测事件”的探索价值。为了继续推进后续工作（以及避免在公网实验时被实现细节拖慢定位），需要先对现有 Go 代码做一次系统性 code review，并把已发现的风险点用可追踪的方式修复掉。

本 change 的目标不是重写架构或过度产品化，而是把影响正确性/可用性/可调试性的关键问题（资源泄漏、stdout 污染、命名割裂、死代码等）收敛到可接受状态。

## What Changes

- 修复控制面 QUIC 连接 `Close()` 的资源回收语义，避免连接/协程泄漏。
- 移除/收敛库代码中的 `fmt.Printf` 等 stdout 输出（特别是拥塞控制实现），统一走现有日志设施。
- 清理明显的死代码/无效 helper（例如未使用且语义错误的 `withDeadline`，重复/未引用的 frame helper 等）。
- 收敛非必要的 `xtcp` 命名残留（仅针对当前主线语义/运行时标识，不改动历史文档与归档报告）。
- 将本轮 review 的结论按模块拆分为可执行任务，并为关键修复补充最小验证（单测 / `go test` / `go vet` / `check_no_xtcp_imports`）。

## Capabilities

### New Capabilities
- `miopunch-code-health`: 定义并约束关键 Go 代码健康度要求（资源释放语义、日志输出约束、命名收敛与死代码清理），用于支撑后续实验与迭代的可维护性。

### Modified Capabilities
- (none)

## Impact

- 受影响代码模块（预期会修改）：
  - `internal/control/`：QUIC control stream 的关闭语义；ALPN 命名收敛。
  - `internal/dataplane/congestion/bbr/`：移除 stdout debug 输出，改为可控日志。
  - `dataplane/`：清理无效 helper；补齐忽略错误的边界处理（如 deadline 设置）。
  - `internal/peer/`：清理未使用 helper；优化错误包装与可读性（不改变协议语义）。
  - `connectivity/`：包注释中 `xtcp` 残留的收敛（不影响行为）。
- 验证与门禁：
  - `export PATH=/usr/local/go/bin:$PATH`
  - `gofmt -l .`、`go test ./...`、`go vet ./...`、`bash scripts/check_no_xtcp_imports.sh`

