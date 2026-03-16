# Design: xtcp-kernel

## Summary

`xtcp-kernel` 是一个以“中心协调 + 两端数据面”为主流程的 NAT traversal 内核。
本阶段只解决“能稳定建链且失败可解释”的最小问题，将连通性增强与高性能传输延后到 `P2/P3`。

## Key Decisions

### Reference baseline via submodule

- `frp/` 以 `git submodule` 固定版本作为参考与对照基线。
- 内核实现不依赖 `frp` 包作为运行时依赖；任何“对齐/偏离”需要可解释并体现在文档与测试中。

### Separation of concerns

- 明确区分 `control plane`（协调与信息交换）与 `connectivity kernel`（建链状态机）。
- 将“建链成功”与“面向性能的传输层优化”解耦：`P1` 只要求最小数据收发验证，但允许 `control plane` 选择 `TCP / KCP / QUIC` 作为传输协议；性能调度优化留给后续阶段。

### Testability-first

- 所有超时与重试策略必须可注入（时间源/参数可控），以支持确定性单元测试。
- 与 `P0` 实验台的集成回归是主线验收方式之一：不仅验证成功/失败，还验证诊断信息完整性。

### Observability as a hard constraint

- 建链过程必须按阶段输出可机读事件流，失败必须携带明确阶段与关键条件。
- `fallback` 必须显式可见、可配置、可测试，禁止“静默回退掩盖失败”。
