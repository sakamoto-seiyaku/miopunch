# P2 连通性增强纲领

## 文档状态

- 本文档定义 `P2` 的目标、边界、约束与关键原则。
- 本文档不展开实现细节，不替代后续 OpenSpec change。
- 后续实现前，应基于本文档与 `docs/roadmap.md` 创建并收敛对应 change。

## 背景

- `P1` 已提供 `xtcp-kernel`：以 `UDP` 打洞为主的“中心协调 + 两端数据面”建链流程，并在 `P0` 实验台中完成回归基线。
- 真实网络环境中，`IPv6 / dual-stack` 与端口映射辅助（`UPnP / NAT-PMP`；`PCP` deferred）经常决定“能否直连”和“是否需要打洞”。
- `P2` 的目标是补齐连通性：让系统优先走更可靠、更直接的路径，同时保持 `P1` 的打洞内核作为最后兜底路径（在 `P2` 仍不引入 relay）。

## 核心决策

- `P2` 只做 `UDP` 连通性增强：不包含 `TCP punching`。
- `IPv6` 是一等公民：只要存在可用的 `IPv6` 直连路径，应优先于所有 `IPv4` 方案。
- 端口映射（`UPnP / NAT-PMP`）是 `IPv4` 的连通性增强手段：它提供额外候选与诊断，不阻塞主流程。
- `P2(v1)` 的 `portmap helper` 只做 `UPnP + NAT-PMP`；`PCP` 另开 change（证据驱动）。
- `STUN` 不是 `P2` 直连路径的硬依赖：当 `IPv6` 或 `IPv4 portmap` 可用时，即使 `STUN` 未配置/不可用，也应允许建链成功；仅当回落到 `IPv4 punching` 时才依赖 `STUN` 产出 mapped addrs。
- 采用最小的 `tailscale-like` 分层流程：
  - `Prepare/Gather`（exchange 之前）：先 bind 本地 `UDP4` 端口；并发做 `STUN`（按需）+ `port mapping`（后台、best-effort、非阻塞）。
  - `Exchange`（单次快照）：把当前已收集到的候选一次性交换；不做候选增量更新（no trickle）。
  - `Attempt`（exchange 之后）：固定优先级尝试 `IPv6` → `IPv4 portmap 直连` → `P1 xtcp punching(mode0..4)`。
- “外圈 attempt policy”在端侧执行；协调端只负责会话编排与信息交换，不承担复杂策略决策。
- 允许修改 `P1` 阶段代码，但必须遵守：
  - 优先“重组/粘合/加扩展点”，而不是改动 `P1` 的 `IPv4 punching` 算法行为。
  - 任何对 `P1` 行为的影响必须用 `P0` 实验台回归证明：`P1` 既有矩阵结果不应因 `P2` 引入而劣化或漂移（除非 change 明确声明并给出证据）。

## 目标

- 提供 `IPv6 / dual-stack` 连通性能力，并可机读输出其选择与失败原因。
- 提供 `UPnP / NAT-PMP` 端口映射辅助能力（`IPv4`），并将其作为候选来源融合到整体尝试流程（`PCP` deferred）。
- 形成统一的“候选快照 + 固定尝试顺序 + 预算/超时 + 可观测事件流”的最小闭环。
- 保持 `P1` 的 `IPv4 xtcp punching` 作为兜底路径，并以回归保证其稳定性。

## 非目标（明确推后）

- 不在 `P2` 引入 `relay / fallback`。
- 不在 `P2` 做候选增量更新（trickle candidates）或中途重新交换。
- 不在 `P2` 做长期后台 gather/cache、端口映射续租/重发现/多网关选择等完整生命周期体系。
- 不在 `P2` 支持 `IPv6` 侧的端口映射（例如 `IGDv2 IPv6 pinhole`）。
- 不在 `P2` 做 `happy-eyeballs` 式复杂并发竞速与自适应权重学习（先固定策略，后续用证据再演进）。

## 可观测性约束

- 必须暴露并可机读记录：
  - `gather`：收集到哪些候选、来自何种来源（`v6` / `stun` / `portmap`）、耗时与错误。
  - `exchange`：本次会话交换的候选快照内容与版本号/时间戳。
  - `attempt`：尝试顺序、每个候选的开始/结束、超时/取消原因、最终选择的路径。
- 失败必须可解释：明确失败发生在哪一层（`v6` 失败、`portmap` 失败、`punching` 失败），并携带可用于排障的关键上下文。

## 测试与验收

- 单元测试：候选过滤与排序、attempt policy 的状态机、超时/取消语义、观测事件完整性。
- 集成回归：基于 `P0` 实验台新增并固化 `P2` 场景（至少覆盖 `IPv6` 可用、`portmap` 可用、回落到 `punching` 三类路径）。
- 回归约束：`P1` 的既有集成矩阵必须继续通过，并在 `P0` 实验台中复测确认结果基线不漂移。

> 开放问题与后续方向统一收敛在 `docs/roadmap.md` 的末尾章节。
