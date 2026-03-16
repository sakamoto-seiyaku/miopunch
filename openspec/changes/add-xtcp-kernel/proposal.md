# Proposal: add-xtcp-kernel

## Why

`P0` 已提供可复现的 NAT 实验台与回归用例，但项目主线仍缺少一个可独立演进的 NAT traversal 内核实现。
后续连通性增强（`P2`）与传输层抽象（`P3`）都需要一个“最小可运行的建链内核”作为基础。

`P1` 以 `frp xtcp` 为参考起点，但目标不是复制一个完整 `frp`，而是提炼出一个可测试、可解释、可演进的核心库与最小 CLI。

## What Changes

本 change 新增 `xtcp-kernel` 能力，定义并实现：
- 最小 `control plane`（中心协调）与两端信息交换流程。
- `control plane` 支持 `TCP / KCP / QUIC` 传输协议选择，对齐 `frp` 现有架构；`P1` 不实现额外“加密/压缩”包装逻辑。
- 基础 `discovery`（STUN）与必要的 NAT 行为信息记录/判定入口。
- UDP 打洞的核心状态机（`make hole` + `confirm`）。
- 受控 `fallback`（可配置、可观测、可测试），避免隐式降级掩盖失败原因。
- 最小 CLI：可在 `P0` 实验台中一键复现实验与回归。
- 测试体系：单元测试高覆盖 + 基于 `P0` 实验台的集成回归入口。

## Inputs / References

- Charter: `docs/p1-xtcp-kernel-charter.md`
- Roadmap: `docs/roadmap.md`
- Project conventions: `openspec/project.md`
- Reference implementation baseline: `frp/`（git submodule，固定版本；重点参考 `xtcp` 与 `pkg/nathole` 相关逻辑）

## Impact

- 新增 capability：`xtcp-kernel`
- 将引入新的 Go 模块与包结构（本阶段以 `Linux-first` 为准）
- 明确排除：`UPnP / NAT-PMP / PCP / IPv6`（`P2`）、面向性能的 `HY2` 风格调度优化（`P3`）、`overlay/mesh`、`VPP`、`TCP punching`
