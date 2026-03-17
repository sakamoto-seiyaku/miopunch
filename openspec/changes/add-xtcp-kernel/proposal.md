# Proposal: add-xtcp-kernel

## Why

`P0` 已提供可复现的 NAT 实验台与回归用例，但项目主线仍缺少一个可独立演进的 NAT traversal 内核实现。
后续连通性增强（`P2`）与传输层抽象（`P3`）都需要一个“最小可运行的建链内核”作为基础。

`P1` 以 `frp xtcp` 为参考起点，并采用“直接从 `frp` 复制/抽离”方式落地最小内核。
目标不是复制一个完整 `frp`，而是从中抽离出可独立构建与测试的核心切片，并在最小改动下建立稳定回归与可观测性基线。

## What Changes

本 change 新增 `xtcp-kernel` 能力，定义并实现：
- 最小 `control plane`（中心协调）与两端信息交换流程。
- `control plane` 与 P2P 数据面均支持 `KCP / QUIC`（`TCP` 仅作为默认基线，不计入该选择；不含 `fallback relay`）；`P1` 不实现额外“加密/压缩”包装逻辑。
- 从 `frp/`（submodule）复制/抽离 `xtcp/nathole` 相关代码，以最小改动形成独立模块；保留上游许可证与头部，并记录来源 commit/tag。
- 基础 `discovery`（STUN）与必要的 NAT 行为信息记录/判定入口。
- UDP 打洞的核心状态机（`make hole` + `confirm`）。
- `P1` 不实现 `fallback/relay`：直连失败就失败，必须可观测、可定位、可复盘。
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
- 许可证与归因：仓库以 `GPLv3` 为默认选择；复制自 `frp` 的代码继续遵循其上游许可证要求（例如保留 `Apache-2.0` 头部与归因文件）。
- 明确排除：`UPnP / NAT-PMP / PCP / IPv6`（`P2`）、面向性能的 `HY2` 风格调度优化（`P3`）、`overlay/mesh`、`VPP`、`TCP punching`
