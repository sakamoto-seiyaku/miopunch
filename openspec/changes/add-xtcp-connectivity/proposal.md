# Proposal: add-xtcp-connectivity

## Why

`P1 xtcp-kernel` 提供了稳定的 `IPv4 UDP punching` 基线，并且在 `P0` 实验台中可回归、可复盘。
但在真实网络中，`IPv6 / dual-stack` 与端口映射辅助（`UPnP / NAT-PMP`；`PCP` deferred）往往决定了“是否能直连”和“是否必须打洞”。

`P2` 的目标是补齐连通性：在不引入 relay 的前提下，优先走更直接、更可靠的路径，同时保留 `P1` punching 作为最后兜底。

## What Changes

本 change 新增 `xtcp-connectivity` 能力，定义并实现：

- `UDP only`：`P2` 不包含 `TCP punching`。
- `IPv6-first`：当存在可用 `IPv6` 候选时，应优先尝试 `IPv6` 直连。
- `IPv4 port mapping helpers`：以 best-effort 方式尝试 `UPnP / NAT-PMP` 获取 `IPv4` 端口映射，作为额外候选与诊断信息来源；不阻塞主流程（`PCP` deferred）。
- `Single snapshot exchange`：只交换一次候选快照（no trickle candidates）；helper 结果晚到不阻塞建链。
- `Fixed attempt order`：`IPv6` → `IPv4 portmap direct` → `P1 xtcp punching(mode0..4)`。
- `Observability`：对 `gather / exchange / attempt` 的关键事件输出可机读时间线；失败必须可定位到具体阶段并携带可排障上下文。
- `Regression gate`：允许为 `P2` 重组/扩展 `P1` glue code 与协议字段，但必须保持 `P1` 的 `IPv4 punching kernel` 行为基线，并在 `P0` 实验台回归证明不漂移。

## Inputs / References

- Charter: `docs/p2-connectivity-charter.md`
- Roadmap: `docs/roadmap.md`
- Project conventions: `openspec/project.md`
- Reference projects: `docs/reference-projects.md`（重点：`Tailscale`、`MiniUPnP`、`go-libp2p`、`goupnp`、`pion/ice`）

## Impact

- 新增 capability：`xtcp-connectivity`。
- 可能修改既有 `control plane` 的候选交换字段与端侧建链编排逻辑（保持 `P1` punching kernel 行为不变为硬约束）。
- `P0` 实验台将新增 `IPv6` 与 `portmap` 相关回归场景。
- 明确排除：`relay/fallback`、候选增量更新（trickle）、端口映射续租/重发现/多网关选择（完整生命周期）、`TCP punching`。
