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
- 主线（`P0–P3.5`）不以 GUI、包装、易用性为当前重点（以实验可复现/可回归为优先）。
  - 例外：Alpha/POC 产品线以“可用能力 + 极度友好 + 可解释性”为重点（见下文“Alpha/POC 产品线”）。

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

### Alpha/POC 产品线（远程 Shell）

定位：

- 与 `P0–P3.5` 实验主线并行推进；目标是把能力包装成“个人/小团队可用”的 POC，而不是替代实验台。
- 首个可交付能力：`join → ping → sh(tmux)`（远程 Shell + tmux 现场恢复 + 高可解释性）。
- 无中心化数据面 relay（现在与未来都不做）；控制面允许 mesh 转发 + MQTT 兜底；broker 不可信但控制面端到端加密+签名可验真。

事实源：

- 语义/流程/口径：`docs/notes/2026-04-15-alpha-product-discussion.md`
- 术语词典：`docs/notes/2026-04-16-alpha-glossary.md`
- POC 实现清单（vertical slice）：`docs/notes/2026-04-20-poc-implementation-checklist.md`

待办任务（按 change 顺序；小步快走、每步可验证）：

- 约束：每个 change 都应（尽量）包含：单元测试、集成测试、真实环境 smoke test；并把“验收口径/失败口径/用户动作建议”写清楚。
- 建议：每个 change 用 OpenSpec workflow 跟踪（`openspec/changes/poc-XX-*`），避免把实现细节散落在聊天里。

#### Change POC-01：POC 口径收口 + 拆二进制（lab vs product）

- 目标：把“实验主线工具链”和“POC 产品 CLI”彻底解耦，防止互相污染；同时把 POC 可用性边界写清楚。
- 交付：
  - 明确 POC“可用性边界/成功标准”（无数据面 relay 前提下）：支持/不支持网络、`join/ping/sh` 最小验收、失败时用户动作（校时/换 broker/重试/换 seed）。
  - 拆二进制：产品 `miopunch`（POC CLI）与实验 `miopunch-lab`（现有 `coord/peer/stun/mqtt-broker`）。
  - 更新实验脚本/文档统一改用 `miopunch-lab`（不维持两套入口）。
- 测试：
  - 单元：`go test ./...`（至少覆盖新入口与 help/flags）。
  - 集成：lab 自测最小集（确保拆分不破坏实验台）。
  - 真实环境：`miopunch-lab` 基本命令在一台机器可跑通（用于回归“实验线”不被破坏）。

#### Change POC-02：控制面 topic 派生 + inbox/mailbox 基础约束

- 目标：把 broker 视为不可信 mailbox，但做到“入口不可枚举 + 每 peer inbox 唯一”。
- 交付：
  - topic 派生落地与单测（inbox topic 的 HKDF info 必须包含 `peer_id`；topic 小写；`base32(raw,no-pad)`）。
  - `join code` 里携带 broker 实例信息（POC 以“命中同一实例”为优先；后续再增强 hostname/多端点）。
- 测试：
  - 单元：topic 派生确定性测试 + 不同 peer_id 不同 inbox。
  - 集成：本地/CI 可复现的 control-plane smoke（用本地 broker 进程或已有 lab broker 工具跑两端订阅/投递）。
  - 真实环境：公共 broker 路径下完成一次订阅/投递（仅验证“可达 + 不泄露明文”）。

#### Change POC-03：控制面 wire format（签名覆盖 dst）+ bounded flooding(H=3) + 去重/限流

- 目标：把“网内转发控制消息”做成可控、可诊断、不会放大的最小实现。
- 交付：
  - 签名 transcript 覆盖 `dst_peer_id`（`hop_limit` 不签名）；转发仅允许 `hop_limit--` 且必须按原 `dst_peer_id` 转发。
  - bounded flooding：`H=3`，超出即丢弃；去重窗口；每 peer 的限流/队列上限/丢弃策略。
  - 把“限流/丢弃 facts”纳入可解释性输出（方便用户理解为什么没转发/没到达）。
- 测试：
  - 单元：签名/验签覆盖 `dst_peer_id`；hop_limit 修改不影响签名但不会改变 dst；去重窗口正确性。
  - 集成：3 节点模拟（A→B→C）验证 H=3、去重与丢弃 facts。
  - 真实环境：同一 LAN 的 3 个进程 smoke（验证“网内优先 + MQTT 兜底”不互相打架）。

#### Change POC-04：RPC 时间语义 + invite/approve 幂等/uses 持久化（可重启不重复计数）

- 目标：让 join/approve 可重试、可解释、可恢复；issuer 重启后不重复扣 uses、不重复交付 bundle。
- 交付：
  - RPC request 必须包含 `expires_at_unix_ms`，严格过期丢弃；保留 `abs(now-created_at)>10m` sanity drop 并提示校时。
  - issuer(admin) 持久化：`uses_left` + `handled_request_id → cached_response`（覆盖 invite 有效期）。
- 测试：
  - 单元：过期丢弃 + 校时提示；幂等缓存命中；uses 不重复扣减。
  - 集成：issuer 重启回归（同一 request_id 重放不产生新 uses 消耗）。
  - 真实环境：Windows↔Linux（公共 broker）下 `invite→join→approve` 可重试闭环。

#### Change POC-05：daemon `up` + LocalAPI（CLI↔daemon）最小闭环 + 输出契约冻结

- 目标：把“常驻进程 + CLI”跑通；为 UI/面板与未来扩展预留稳定接口。
- 交付：
  - `up` 常驻 + task 框架：`invite/join/approve/ping/sh_ls/sh_attach/revoke_member`（先闭环，后扩展）。
  - LocalAPI：HTTP/JSON + SSE + WS（shell 字节流）；冻结 `stage/reason_code/exit_code` 输出契约（顶层 envelope 稳定）。
  - 本地 state/密钥落盘最低口径：threat model + state 目录权限/ACL + system service/用户态两种最小权限运行方式（不自研加密落盘）。
- 测试：
  - 单元：handler/task 状态机；输出 envelope 稳定性测试。
  - 集成：起 daemon → CLI 调用 → 校验 stage/reason_code/exit_code。
  - 真实环境：Windows 安装/启动 daemon（含管理员权限需求：TUN/驱动未来可用），CLI 可用且错误提示友好。

#### Change POC-06：`sh(tmux)` vertical slice（WSL/SSH targets）+ 单写者锁 + 现场语义

- 目标：交付 POC 核心价值：远程 Shell + tmux 现场恢复 + 可解释性。
- 交付：
  - `sh`：WSL/SSH targets；`tmux new -A -s <session>` 语义；单写者锁（WS 心跳/TTL）；resize；Ctrl-C 透传。
  - 数据面固定默认栈（POC 不做自动协商/降级）；配置不一致给出 `DP_STACK_MISMATCH` 等强解释与修复建议。
- 测试：
  - 单元：锁超时/抢占规则；frame 编解码；错误 reason_code。
  - 集成：本机 tmux + WS 循环（attach/detach/reconnect）回归。
  - 真实环境：Windows 平板/PC + 家中主机（WSL/VM）+ Android 入网辅助，演示 `join→sh` 全流程。

#### Change POC-07：HTTP 面板（POC 最小）+ report/export（可解释性对外输出）

- 目标：把“用户极度友好/一览无余”的可解释性落到 UI 与可分享报告。
- 交付：
  - HTTP 面板：只监听 `127.0.0.1`；卡片+SSE；写操作白名单 `invite/join/sh_attach`。
  - 报告导出：`--report` 生成 md；`--redact` 脱敏开关；事件留存策略（log rotation）写清楚。
- 测试：
  - 单元：SSE/WS 基础协议；report 结构稳定（顶层字段不漂移）。
  - 集成：起面板 → SSE 刷新 → 触发任务 → 卡片状态推进；导出 report 可读。
  - 真实环境：浏览器打开面板完成一次 join/sh，生成可分享的脱敏报告。

## 后续方向

- 个人 `overlay / mesh` 网络。
- 多节点互通、节点间转发/中继（仅 peer↔peer；不引入中心化数据面 relay）。
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
