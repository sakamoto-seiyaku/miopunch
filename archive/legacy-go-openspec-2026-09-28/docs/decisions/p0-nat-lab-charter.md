# P0 NAT 测试实验台纲领

## 文档状态

- 本文档定义 `P0` 的目标、边界、约束与关键原则。
- 本文档不展开实现细节，不替代后续 OpenSpec change。
- 后续实现前，应基于本文档与 `docs/roadmap.md` 创建并收敛对应 change。

## 背景

- 项目需要一个可复现、可回归、可观测的 NAT 测试环境，作为后续 `P1-P3` 的基础设施。
- 当前开发环境是 `Windows 11 + WSL2`。
- 测试环境不得影响 Windows 正常网络，也不应直接污染 WSL2 默认网络。
- 测试环境的第一目标是建立稳定、可控的实验回路，而不是在一开始就逼近所有真实公网行为。

## 核心决策

- `P0` 采用 `single-VM lab host` 方案。
- 在 `WSL2` 内运行单个 `QEMU VM`，该 VM 仅作为实验母机。
- VM 默认采用 Debian 官方 `qcow2 / cloud image` 作为基础镜像，优先开箱即用而不是自行最小安装。
- VM 默认通过 `SSH` 管理，管理面与实验面分离，并允许正常联网。
- NAT 拓扑、链路扰动、抓包、诊断全部在 VM 内完成。
- `Windows` 和 `WSL2` 默认网络不作为实验对象，不做持久修改。
- VM 中安装 `Docker`，但 `P0` 阶段不预先承诺其具体用途；是否介入实验拓扑由后续实现决定。
- `P0` 不采用多 VM 拓扑，不采用 Docker 作为拓扑主控。

## 目标

- 提供一个与宿主环境隔离的 NAT 实验台。
- 在 VM 内构建可脚本化的 `netns / veth / nftables or iptables / tc` 网络拓扑。
- 支持最小 NAT 打洞测试矩阵与失败路径回归。
- 支持抓包、日志、规则状态、连接状态、链路状态等观测手段。
- 提供两级环境快照：`base-ready` 与 `lab-ready`，用于快速回滚到干净状态。
- 为后续 `XTCP` 内核抽离与回归测试提供统一执行环境。

## 快照策略

- `base-ready`：VM 已可启动、`SSH` 正常、基础工具与 `Docker` 已安装，但尚未放入 lab case 定义与待测二进制。
- `lab-ready`：VM 内已放入 `case` 定义、切换脚本与校验逻辑，但没有 active case 正在运行，也没有待测二进制产物。
- 快照用于回滚环境基线，不用于记录某个 case 的测试结果。

## 非目标

- 不在 `P0` 解决真实公网中的全部 NAT 变体。
- 不在 `P0` 构建完整产品级虚拟局域网能力。
- 不在 `P0` 引入多 VM 复杂拓扑。
- 不在 `P0` 以 Docker bridge 或 Compose 作为实验网络主控。
- 不在 `P0` 用真实环境验证替代虚拟实验台。

## 测试层次划分

### 虚拟实验台

- `P0` 的测试对象是 VM 内部的虚拟 NAT 实验环境。
- 它负责提供可控、可复现、可回归的开发与调试回路。
- 它优先解决实验速度、隔离性、可观测性、可清理性问题。

### 真实环境验证

- 真实环境验证是另一条独立测试线，不等同于 `P0` 实验台。
- 真实环境验证依赖额外机器、真实网络、真实链路条件和真实 NAT 设备。
- 真实环境验证用于确认虚拟实验台之外的真实表现，而不是替代实验台。
- 后续阶段应同时维护“虚拟实验台回归”与“真实环境验证”两类测试。

## NAT 场景命名与分类

- `P0` 以 `RFC 4787` 行为模型作为 NAT 场景的主分类。
- 每个 NAT 节点至少记录两条主轴：`Mapping Behavior` 与 `Filtering Behavior`。
- `NAT1 / NAT2 / NAT3 / NAT4` 作为兼容标签保留，用于矩阵编号、讨论与覆盖检查。
- `frp` 的 `EasyNAT / HardNAT` 与 `BehaviorNoChange / BehaviorPortChanged / BehaviorIPChanged / BehaviorBothChanged` 作为工程标签保留，用于对齐实现与排障。
- 每个测试场景必须同时标注 `RFC 4787` 标签、`NAT1-4` 标签、`frp` 工程标签。

### 兼容映射

- `NAT1`：`EIM + EIF`，即 `full cone NAT`。
- `NAT2`：`EIM + ADF`，即 `restricted cone NAT`。
- `NAT3`：`EIM + APDF`，即 `port restricted cone NAT`。
- `NAT4`：`APDM + APDF`，即 `symmetric NAT`。
- `RFC 4787` 的表达范围大于 `NAT1-4`；对于无法自然落入 `NAT1-4` 的组合，仍以 `RFC 4787` 标签为准。

### 工程标签原则

- `frp EasyNAT` 近似表示 `EIM-like`，`frp HardNAT` 近似表示 `non-EIM-like`。
- `frp` 工程标签是实现视角，不替代 `RFC 4787` 主分类。
- `frp` 工程标签除 `EasyNAT / HardNAT` 外，还应补充 `Behavior` 与端口变化规律性。
- 排查与算法分析优先看 `frp` 工程标签；测试矩阵与覆盖率优先看 `RFC 4787` 与 `NAT1-4` 标签。

## 实验台架构

- `QEMU VM` 负责隔离宿主，不承担 NAT 语义表达。
- VM 内部使用 Linux 网络命名空间表达 `peer`、`nat`、`stun`、`coord` 等角色。
- VM 内部使用 `veth` 连接各角色，使用 `nftables or iptables` 表达 NAT 与转发规则。
- VM 内部使用 `tc` 表达时延、丢包、限速等链路扰动。
- VM 内部以 `SSH` 作为默认管理通道，管理面与实验面分离。
- 初期各角色可直接以二进制进程运行，后续再评估是否容器化。

## 可观测性要求

- 建链失败时，必须明确失败发生在哪个阶段。
- 诊断信息至少覆盖 `discovery`、`signaling`、`hole punching`、`fallback`、`transport`。
- 系统应暴露关键网络条件、重试行为、回退行为与最终失败原因。
- 失败路径同样必须可回归、可验证、可复盘。

## Case 体系

### Smoke Cases

- `smoke` 只用于快速验证实验台是否可用，不代表完整覆盖。
- 第一批 `smoke` 至少包括：`NAT1 x NAT1`、`NAT3 x NAT1`、`NAT4 x NAT3`、`NAT4 x NAT4`、显式失败路径各一例。
- 每个 `smoke` case 仍需同时记录 `RFC 4787`、`NAT1-4`、`frp` 三层标签。

### 主覆盖集

- `P0` 不追求 `NAT1 / NAT2 / NAT3 / NAT4` 的机械全组合覆盖，而采用代表性覆盖。
- 覆盖设计优先基于可打通性支配关系、实现风险和算法分支，而不是基于笛卡尔积计数。
- 对于 `NAT1 / NAT2 / NAT3` 这类 `easy-like` 场景，如果某一结论已被更严格的代表场景覆盖，则不再强制补齐更宽松组合。
- 在 `easy-like` 范围内，`NAT3` 可作为比 `NAT1`、`NAT2` 更保守的代表场景；`NAT1`、`NAT2` 主要保留基线和兼容性验证用例。
- 对于 `NAT4`，应继续区分端口变化规律性、角色分配和对端类型，因为这部分更直接决定打洞难度与算法分支。
- 默认不拆分 `A/B` 方向 case；仅当实际测试中观察到两端在先发/后发、角色分配或结果上存在明确不一致行为时，再将该 case 细分为更小的测试 case。
- 主覆盖集中的每个 case 都必须落回 `RFC 4787 Mapping + Filtering` 主分类，并附带 `frp` 工程标签。
- `P0` 的第一批主覆盖集可优先包含：`NAT1 x NAT1`、`NAT1 x NAT2`、`NAT1 x NAT3`、`NAT3 x NAT3`、`NAT2 x NAT4`、`NAT4(regular) x NAT1 or NAT3`、`NAT4(irregular) x NAT1 or NAT3`、`NAT4(regular) x NAT4(regular)`、`NAT4(irregular) x NAT4(regular)`、`NAT4(irregular) x NAT4(irregular)`。

### 第一批主覆盖集说明

| Case ID | NAT 组合 | RFC 4787 主视角 | `frp` 工程视角 | 保留原因 |
| --- | --- | --- | --- | --- |
| `core-01` | `NAT1 x NAT1` | `EIM+EIF ↔ EIM+EIF` | `Easy ↔ Easy` | 最宽松基线，验证实验台、协调链路与基础建链路径。 |
| `core-02` | `NAT1 x NAT2` | `EIM+EIF ↔ EIM+ADF` | `Easy ↔ Easy` | 覆盖地址级过滤，引入第一层受限回包条件。 |
| `core-03` | `NAT1 x NAT3` | `EIM+EIF ↔ EIM+APDF` | `Easy ↔ Easy` | 覆盖端口级过滤，是 `easy-like` 范围内更保守的兼容性样本。 |
| `core-04` | `NAT3 x NAT3` | `EIM+APDF ↔ EIM+APDF` | `Easy ↔ Easy` | 作为 `easy-like` 对 `easy-like` 的代表困难上界。 |
| `core-05` | `NAT2 x NAT4` | `EIM+ADF ↔ APDM+APDF` | `Easy ↔ Hard` | 保留一个中间过滤强度对困难 NAT 的过渡样本。 |
| `core-06` | `NAT4(regular) x NAT1` 或 `NAT4(regular) x NAT3` | `APDM+APDF ↔ EIM+EIF/APDF` | `Hard(regular) ↔ Easy` | 覆盖 `hard-like` 对 `easy-like` 的较易成功分支。 |
| `core-07` | `NAT4(irregular) x NAT1` 或 `NAT4(irregular) x NAT3` | `APDM+APDF ↔ EIM+EIF/APDF` | `Hard(irregular) ↔ Easy` | 覆盖 `hard-like` 对 `easy-like` 的更困难分支。 |
| `core-08` | `NAT4(regular) x NAT4(regular)` | `APDM+APDF ↔ APDM+APDF` | `Hard(regular) ↔ Hard(regular)` | 双边困难 NAT 的规律端口变化基线。 |
| `core-09` | `NAT4(irregular) x NAT4(regular)` | `APDM+APDF ↔ APDM+APDF` | `Hard(irregular) ↔ Hard(regular)` | 覆盖非对称困难 NAT 组合。 |
| `core-10` | `NAT4(irregular) x NAT4(irregular)` | `APDM+APDF ↔ APDM+APDF` | `Hard(irregular) ↔ Hard(irregular)` | 当前最保守、最困难的代表场景。 |

### 派生变体

- 在主覆盖集之上，再叠加端口变化规律性：`regular` 与 `irregular`。
- 在主覆盖集之上，再叠加链路条件：`delay`、`loss`、`reorder`、`rate limit`。
- 在主覆盖集之上，再叠加失败路径：超时、候选端口不足、回退触发、诊断信息校验。
- 对于超出 `NAT1-4` 的 `RFC 4787` 组合，作为扩展 case 单独纳入，不强行塞回 `NAT1-4`。

### 记录原则

- 每个场景记录中，`RFC 4787` 标签是主键，`NAT1-4` 与 `frp` 标签是并列附加信息。
- 每个 case 至少记录：两端 NAT 标签、角色分配、链路条件、预期结果、实际结果、失败阶段、诊断信息。
- 覆盖率按“代表场景是否覆盖关键行为与关键算法分支”评估，而不是按组合数量评估。
- 多个 case 可以共存定义在同一 VM 内，但任意时刻只允许一个 active case 运行；不同 case 通过切换脚本进入与退出。
- `smoke` 用于日常快速回归；主覆盖集用于关键覆盖；派生变体用于算法与诊断深挖。

## 完成标准

- 可以一键创建、执行、销毁实验拓扑。
- 可以稳定复现 `smoke` 与第一批主覆盖集中的代表场景。
- 可以导出日志、抓包和关键网络状态。
- 可以在失败时给出阶段化、可排障的诊断信息。
- 可以作为后续 OpenSpec change 和实现工作的统一测试底座。

## 开放问题

- `QEMU VM` 的具体启动参数、镜像拉取方式与快照命名规则如何约定。
- 是否需要在 `P0` 引入 `conntrack` 状态采集脚本。
- 对于超出 `NAT1-4` 的 `RFC 4787` 组合，何时纳入第一批实验矩阵。
- 何时把真实环境验证纳入阶段性验收，而不是只做额外检查。
