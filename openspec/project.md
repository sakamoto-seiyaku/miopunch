# Project Context

## Purpose
`miopunch` 是一个以 `frp xtcp` 为起点、持续演进的 `Go NAT traversal` 项目。

项目当前目标：
- 抽离一个最小可运行、可独立演进的 NAT traversal 内核。
- 在经典 UDP 打洞基础上补齐辅助连通性能力。
- 将“打洞成功”与“数据传输”解耦。
- 建立可复现的 NAT 测试与回归体系。

项目当前聚焦工程与技术演进，不以 GUI、包装和产品化表达为当前重点。

## Tech Stack
- `Go`：主语言；生产代码、测试代码、CLI 工具默认都优先使用 Go。
- `OpenSpec`：用于需求、变更、约束和实施顺序管理。
- `Linux networking tools`：`netns`、`veth`、`nftables/iptables`、`tc`，用于 NAT 仿真和回归环境。
- `Virtualization`：`P0` 优先使用单个 `QEMU VM` 作为隔离实验母机。
- `Linux lab networking`：在 VM 内使用 `netns`、`veth`、`nftables/iptables`、`tc` 构建实验拓扑。
- `Containerization`：`Docker` 可作为后续进程打包手段，但不作为 `P0` 的拓扑主控。
- `STUN`：用于地址发现与 NAT 相关信息获取。
- `UDP hole punching helpers`：`UPnP`、`NAT-PMP`、`PCP`。
- `Transport protocols`：P2P 数据面基线支持 `KCP / QUIC`；后续引入 `HY2` 风格的 `QUIC` 调度与进一步拥塞控制优化。
- `Target platforms`：优先 `Linux`，后续扩展到 `Android`、`Windows`。

## Project Conventions

### Code Style
- 遵循 Go 最佳实践与 idiomatic Go，优先简单、直接、可读的实现。
- 能不用抽象就先不用抽象；先把协议流程、状态机和边界条件写清楚。
- 包、类型、函数命名必须贴合网络语义，避免模糊缩写。
- 错误处理必须显式，错误信息必须能帮助定位建链阶段和失败原因。
- 可观测性优先：用户在打洞失败后应能知道失败发生在哪个环节、看到了什么网络条件、系统做过哪些重试或回退。
- 涉及超时、取消、重试的接口优先使用 `context.Context`。
- 优先依赖标准库；引入第三方库前要有明确收益。
- 注释只解释协议原因、网络假设和实现取舍，不解释显而易见的代码。
- 文档默认使用中文；代码标识符、包名、spec/change ID 使用英文。

### Architecture Patterns
- 明确区分 `control plane`、`connectivity`、`transport`，不要早期耦合成单体设计。
- `NAT traversal` 是项目内核；`overlay / mesh`、`VPP`、`TCP punching` 都属于后续方向，不应反向污染早期核心抽象。
- 优先拆成可测试的阶段：`discovery/classification`、`signaling`、`make hole`、`fallback`、`transport`。
- `frp xtcp` 是起点和参考实现，不是必须完全复制的最终架构；任何偏离都应在 spec 或设计文档里说明。
- 在核心模型稳定前，不要过早做“大而全”的平台层、插件系统或通用框架。
- `docs/roadmap.md` 描述阶段主线；OpenSpec change 负责描述具体变更。

### Testing Strategy
- 坚持 `测试先行 / 测试优先`。
- 接受“测试代码多于功能代码”。
- 单元测试覆盖：协议编码、消息交换、NAT 分类、状态机、重试、回退、双栈地址选择。
- 集成测试覆盖：基于单 VM 实验母机内部的 NAT 拓扑、链路质量扰动、回归基线。
- 真实环境验证覆盖：独立于虚拟实验台，使用安卓蜂窝、本地宽带、云服务器等真实网络组合。
- 每个阶段至少要有可重复的成功率、建链时延、吞吐、回退率指标。
- 新能力进入主线前，先证明可复现、可回归、可解释。

### Git Workflow
- 对新增能力或较大变化，先在 `docs/` 下创建纲领文档，先说明目标、范围、约束、边界和关键原则，不急于展开实现细节。
- 在纲领文档基础上，结合 `roadmap`、`project context` 和其他相关文档创建 OpenSpec change。
- OpenSpec change 阶段用于反复讨论、补齐和收敛约束、边界与细节；change 未收敛、未批准前，不进入实现阶段。
- 功能、架构、能力变更优先走 OpenSpec：先写 proposal，再讨论，再实现。
- 未经批准的 OpenSpec proposal，不进入实现阶段。
- 分支尽量短生命周期，分支名优先对齐 OpenSpec `change-id`。
- 提交应小而聚焦，不混入无关改动。
- 提交信息保持简洁明确，直接说明改了什么；spec、docs、code 可以分开提交。
- 如果只是整理讨论或路线图，也应优先更新文档而不是抢先写实现。

## Domain Context
- 本项目讨论的“打洞”默认指 `UDP NAT traversal / UDP hole punching`。
- `frp xtcp` 提供的是“中心协调 + P2P 数据面”的起点，不等于完全去中心化。
- `fallback` 是受控降级机制，不应掩盖失败原因；失败路径同样要可观测、可测试。
- `UPnP`、`NAT-PMP`、`PCP` 属于连通性增强与端口映射辅助，不是经典打洞的替代。
- `IPv6` 是一等公民能力；设计时默认考虑 `IPv4 / IPv6 / dual-stack`，而不是把 IPv6 当附属功能。
- 当前明确主线只有 `P0` 到 `P3`：测试台、XTCP 内核抽离、连通性增强、传输层抽象。
- `overlay / mesh`、`VPP`、`TCP punching`、`udp2raw` 风格伪装属于后续方向，现阶段只保留接口空间和设计余地，不提前承诺实现。

## Important Constraints
- 当前项目首先是工程探索项目，不是面向终端用户的成品。
- 早期阶段不以完整产品体验为目标，而以真实 NAT 场景下的成功率、稳定性、可解释性为目标。
- 早期不承诺完整虚拟局域网能力，不承诺 `TCP punching`，不承诺 `VPP`，不承诺全平台完整支持。
- 开源许可证：仓库默认以 `GPLv3` 发布；复制自 `frp` 的文件保留并遵循其上游许可证与归因要求（例如 `Apache-2.0` 头部与归因文件）。
- 优先做 `Linux-first` 的可复现测试台；`P0` 以单个 `QEMU VM` 作为实验母机，再逐步扩展真实设备与跨平台验证。
- 对新增能力或较大变化，必须先有 `docs/` 下的纲领文档，再创建并收敛对应的 OpenSpec change；未经收敛和批准，不进入实现。
- 架构讨论必须服从实验结果；没有测试和测量支撑的设计，不应直接进入主线。
- 可观测性是硬约束：失败必须可定位、可解释、可复盘，系统应向用户暴露足够的阶段信息、诊断信息和回退信息。
- 不假设用户缺乏理解能力；诊断信息应尽量准确、具体、面向排障，而不是只给出模糊失败提示。
- 项目当前根目录尚未形成最终代码结构；在目录稳定前，不要过早固化大规模工程脚手架。

## External Dependencies
- `frp/`（git submodule）：参考实现，尤其是 `xtcp` 与 `pkg/nathole` 相关逻辑。
- `STUN servers`：用于公网地址发现和 NAT 相关信息获取。
- `UPnP / NAT-PMP / PCP` capable routers or emulators：用于辅助连通性实验。
- `Cloud coordination server`：用于信令协调、真实网络回归和中继/回退实验。
- `Android device + cellular network + home broadband`：用于真实环境验证。
- `Linux kernel networking features`：网络命名空间、路由、NAT、流量控制等实验基础设施。
