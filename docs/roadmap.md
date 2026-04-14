# Roadmap

## 文档状态

- 本文档用于收束前期讨论。
- 当前版本只定义主线，不锁死细节。
- 后续按实验结果、实现成本、真实网络表现持续调整。

## 当前进度（截至 2026-04-14）

- `P0`：NAT 实验台已落地，case 覆盖集可一键自测并导出 artifacts；见 `docs/decisions/p0-nat-lab-charter.md`、`openspec/changes/archive/2026-03-24-add-nat-lab-testbed/`、`docs/reports/2026-03-17-selftest.md`。
- `P1`：打洞内核（from `frp xtcp`）抽离已落地，`core-01..core-10 × {kcp,quic}` 在 P0 VM 内完成实测并汇总；见 `docs/decisions/p1-xtcp-kernel-charter.md`、`openspec/changes/archive/2026-03-24-add-xtcp-kernel/`、`docs/reports/2026-03-18-xtcp-fulltest.md`。
- `P2`：连通性增强层已落地（`IPv6-first`、`UPnP/NAT-PMP`、固定 attempt 顺序、可观测性、no-trickle），并在 P0 VM 内完成实测并汇总；见 `docs/decisions/p2-connectivity-charter.md`、`openspec/changes/archive/2026-03-24-add-xtcp-connectivity/`、`docs/reports/2026-03-24-xtcp-connectivity-fulltest.md`。
- `P3`：目录重组与命名收敛已完成并归档，传输层抽象正在推进；见 `docs/decisions/p3-miopunch-transport-charter.md`、`openspec/changes/archive/2026-03-28-reorg-miopunch-layout/`、`openspec/changes/add-miopunch-dataplane/`。
- `P3.5`：公网实验可达性补强已进入纲领阶段，聚焦 `IPv4-only / IPv6-only`、内置 `DNS` 与内置默认 `STUN/MQTT` 名单、以及中国大陆 / 非中国大陆 `STUN` 观测分域；见 `docs/decisions/p3.5-public-network-charter.md`。

## 定位

- `miopunch` 是一个以 `frp xtcp` 为起点、持续演进的 `Go NAT traversal` 项目。
- 项目关注 `可测试`、`可复现`、`可演进`。

## 核心目标

- 抽离一个最小可运行、可独立演进的 NAT traversal 内核。
- 在经典 UDP 打洞基础上补齐辅助连通性能力。
- 将“打洞成功”与“数据传输”解耦。
- 让建链失败可观测、可定位、可解释。
- 建立一套可复现的 NAT 测试与回归体系。

## 非目标

- 不在早期追求完整产品形态。
- 不在早期承诺完整虚拟局域网能力。
- 不在早期承诺 `TCP` 打洞、`VPP`、全平台完整支持。
- 不以 GUI、包装、易用性为当前重点。

## 开发原则

- `测试先行`。
- 追求尽可能完整的测试覆盖。
- 接受“测试代码多于功能代码”。
- 每个阶段都必须可验证、可演示、可回归。
- 可观测性优先；失败时应明确暴露所处阶段、网络条件、重试与回退信息。
- 优先解决真实 NAT 场景下的成功率、稳定性、可解释性。

## 测试体系

- 单元测试：协议、状态机、NAT 分类、重试、回退策略。
- 集成测试：基于单 VM 实验母机内部的 NAT 拓扑回归。
- 真实环境验证：独立于虚拟实验台，使用安卓蜂窝、本地宽带、云服务器组合验证。
- 失败路径同样需要回归，验证诊断信息、阶段信息、回退信息是否准确。
- 核心指标：建链成功率、建链时延、吞吐、回退率、不同 NAT 组合表现。

## 外部参考项目

- 详细清单与借鉴边界见 `docs/reference-projects.md`；这些项目用于吸收局部工程经验，不代表整体照搬。
- `frp`：`P1` 行为基线；后续阶段继续作为回归对照与来源追溯参考。
- `Tailscale`：`P2` 连通性、双栈、candidate / endpoint 聚合、可观测性；后续 `overlay / mesh` 也可参考。
- `MiniUPnP`：`P0 / P2` 的 `UPnP / NAT-PMP / PCP` 实验台与协议行为基线。
- `go-libp2p`：`P2` 的端口映射生命周期、续租、重发现、多网关选择参考。
- `goupnp`：`P2` 的 Go 侧 `UPnP` 客户端实现基线，但不能直接等价为完整双栈方案。
- `pion/ice`：`P2 / P3` 的 candidate、双栈 gather、优先级与路径切换参考。
- `gonc`：真实网络 heuristics、诊断表达与实验性打洞策略参考。

## 主线 Roadmap

### P0 测试台先行

- 先建设 NAT 仿真与回归环境，再推进功能开发。
- `P0` 采用 `single-VM lab host`：在 `QEMU VM` 内承载实验台。
- VM 内部优先使用 `netns + veth + nftables/iptables + tc` 表达 NAT 拓扑与链路扰动。
- `Docker` 不作为 `P0` 的拓扑主控；真实环境验证也不与虚拟实验台混为一体。
- 产出应包括可脚本化拓扑、回归用例、基础指标采集。

### P1 抽离打洞内核

- 从 `frp xtcp` 提炼最小 NAT traversal 核心。
- `frp/` 以 `git submodule` 引入并固定版本，仅作为参考与行为对齐基线（不 vendor，不作为运行时依赖）。
- 范围包括 `STUN`、NAT 分类、信息交换、`make hole`、P2P 数据面（`KCP / QUIC`）。
- `control plane` 与 P2P 数据面均支持 `KCP / QUIC`（`TCP` 仅作为默认基线，不计入该选择；不含 `fallback relay`）；不引入额外“加密/压缩”包装逻辑。
- 目标是形成可独立测试、可独立运行、可独立演进的核心库与最小 CLI。
- 这一阶段不引入复杂组网语义。
- `P1` 不引入 `fallback/relay`；直连失败就失败，必须可观测、可定位、可复盘。

### P2 补齐连通性

- 在基础打洞流程上补齐 `UPnP`、`NAT-PMP`、`IPv6`、双栈策略（`PCP` deferred，证据驱动另开 change）。
- 将端口映射辅助视为连通性增强，而不是经典打洞的替代。
- 将 `IPv6` 视为一等公民能力，而不是附属特性。
- `P2(v1)` 只交换一次候选快照（no trickle candidates）；helper 结果晚到不阻塞主流程。
- `P2(v1)` 固定尝试顺序：`IPv6` → `IPv4 portmap` → `IPv4 punching(mode0..4)`。
- 优先收敛统一的 `candidate model` 与 `attempt policy`，将 `STUN / IPv6 / port mapping helpers` 统一为候选来源。
- `helper` 只负责产出 candidate、lease 与诊断信息，不直接侵入 `transport` 语义。
- 允许为 `P2` 重组/扩展 `P1` glue code 与协议字段，但必须保持 `P1` 的 `IPv4 punching kernel` 行为基线，并在 `P0` 实验台回归证明不漂移。
- 目标是最大化真实网络中的建链成功率。

### P3 抽象传输层

- 将打洞成功后的 session 抽象成独立传输层接口。
- 目录重组与命名收敛：见 `openspec/changes/archive/2026-03-28-reorg-miopunch-layout/`。
- `dataplane` 抽象与最小验收：见 `openspec/changes/add-miopunch-dataplane/`。
- 以 `KCP / QUIC` 为基线，并定义传输选项的协商/切换与测试基准。
- 在传输层稳定后，引入 `HY2` 风格的 `QUIC` 调度/拥塞控制作为与 `KCP / QUIC` 同级的传输选项。
- 目标是把“能连上”与“传得好”拆成两个独立问题。

### P3.5 公网实验可达性补强

- `P3.5` 是 `P3` 与 `P4` 之间的过渡阶段，目标是让真实网络实验更少依赖手工环境修补。
- 支持显式约束仅使用 `IPv4` 或仅使用 `IPv6`，用于公网诊断、定向实验与问题隔离。
- 在显式要求或系统 `DNS` 不可用时，允许回退到内置 `DNS`；同时提供内置默认 `STUN / MQTT` 名单，减少手工录入 `IP` 的负担。
- 面向中国大陆 / 非中国大陆分流场景，将不同 `STUN` 观测面视为不同公网视角，并为后续打洞路径选择保留仲裁空间。
- `P3.5` 仍以“完成公网实验准备”为目标，不提前展开完整产品化配置、加密与发布语义。

## 后续方向

- 个人 `overlay / mesh` 网络。
- 多节点互通、节点间转发、弱中心化 relay。
- `VPP` 数据平面。
- `TCP` 打洞。
- `udp2raw` 式伪装与用户态协议栈实验。
- `Linux / Android / Windows` 深化支持。

## 里程碑原则

- 每个阶段都必须有单独产出物。
- 每个阶段都必须定义回归基线。
- 每个阶段都必须用真实网络或仿真网络复验。
- 新方向进入主线前，先通过实验分支验证价值。

## 开放问题（待讨论）

> 这些问题不直接进入当前主线实现；需要更多真实网络证据与工程验证后再开 change 推进。

- `port mapping`：`UPnP/NAT-PMP` 是否足够；是否需要 `PCP`；是否需要续租/重发现/多网关选择；是否需要直接移植 `Tailscale net/portmapper`。
- `IPv6` 候选选择：地址过滤、接口选择与优先级（先收敛最小可用规则，再在真实网络中迭代）。
- `IPv6 NAT66 / 受限 IPv6`（例如教育网）：是否需要 `UDP6` 侧 STUN；是否需要把 `P1 IPv4 punching kernel(mode0..4)` 泛化到 `UDP6`。
- `Prepare/Gather` 时间预算：在 no-trickle 前提下，gather 窗口如何平衡“成功率/时延”。
- 全局代理/TUN 干扰：clash 等导致 STUN 得到的公网信息不一致、tun 网卡干扰、fakeip 等问题。
