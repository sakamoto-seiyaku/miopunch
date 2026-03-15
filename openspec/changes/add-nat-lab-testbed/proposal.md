# Proposal: add-nat-lab-testbed

## Why

`miopunch` 的后续主线依赖一个可复现、可观测、可回滚的 NAT 测试实验台。
当前开发环境是 `Windows 11 + WSL2`，直接在宿主默认网络上构造复杂 NAT 场景会增加污染风险，也会降低排障可解释性。

在开始 `XTCP` 内核抽离、连通性增强和传输层实验之前，需要先落一个独立的 `P0` 测试底座。这个底座必须：
- 与 `Windows` 和 `WSL2` 默认网络隔离。
- 支持 `RFC 4787` 主分类、`NAT1-4` 兼容标签、`frp` 工程标签三层标注。
- 支持代表性 case 切换，而不是机械全组合枚举。
- 支持失败路径诊断与环境回滚。

## What Changes

本 change 新增 `nat-lab-testbed` 能力，定义：
- 基于单个 `QEMU VM` 的隔离实验母机。
- VM 内部基于 `netns / veth / nftables or iptables / tc` 的 NAT 拓扑构造方式。
- `base-ready` 与 `lab-ready` 两级环境快照模型。
- 同一 VM 内多个 case 共存定义、任意时刻仅一个 active case 的运行模型。
- 以 `RFC 4787` 为主分类、以 `NAT1-4` 与 `frp` 标签为辅的 case 描述方式。
- 第一批主覆盖集与最小观测产物要求。

## Inputs / References

- Charter: `docs/p0-nat-lab-charter.md`
- Roadmap: `docs/roadmap.md`
- Project conventions: `openspec/project.md`

## Impact

- 新增 capability：`nat-lab-testbed`
- 当前阶段只定义能力和边界，不直接引入用户态 NAT emulator。
- 当前阶段不要求完整自动化流水线，但要求为后续脚本化和自动化保留清晰入口。
