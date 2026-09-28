# iroh 调查与 miopunch 重新讨论的起点

日期：2026-09-27。

状态：独立研究与待讨论推论，不是已批准的产品决策、OpenSpec proposal 或实施任务。
范围：阅读当前仓库、iroh 官方文档与固定版本 API；未运行 iroh、未进行真实网络对比实验。性能、地域可达性和迁移体验均未实测。

## 1. 当前项目事实

当前事实源为 `openspec/project.md`、`openspec/specs/miopunch-poc-v1-current-mainline/spec.md` 和 `internal/pocv1/*`，不是旧 P0/P3/MNT 的全部愿景。

- 产品是可演示的 P2P remote-control POC，已有 Linux/Windows 桌面、CLI 和 Android control-lite。
- MQTT 是不可信信令承载；成员凭证、可信 roster 和消息保护由项目负责。
- 建路为 UDP direct-first + UDP punching fallback；当前主线不承诺数据面 relay。
- 会话固定为 KCP + TLS 1.3 + yamux。
- `internal/pocv1/punch/types.go:147` 的 `PathResult` 携带选定 UDP endpoint、身份、证据和 socket 所有权。
- `internal/pocv1/session/types.go:87` 的 Dial/Accept 从这个结果建立 PeerSession。
- `internal/pocv1/runtime/runtime.go:894` 的 keepalive 失败会关闭对应会话；`proxyShell` 和 `bridgeRemotePTY` 的现有流桥接遇到结束/错误会关闭资源。这些代码不能作为透明业务恢复的证明；tmux 等外部会话的持久性需要单独理解。

旧的 `docs/decisions/path-layer-architecture-charter.md` 已提出高移动性、私有 relay 和可恢复会话，属于有价值的历史思考，不是当前已实现能力。2026-06 的回归笔记也应结合后续 resolution 和当前源码阅读，不能直接视为仍未修复的缺陷。

## 2. iroh 已核实的设计

### 版本与证据

调查时 docs.rs 最新版为 1.2.0。以下优先使用固定版本 API，避免将早期 iroh-net/magicsock 或 Quinn 时代的描述混入当前设计。[固定版本入口](https://docs.rs/iroh/1.2.0/iroh/)

1.2.0 发布记录列出 noq 1.3.0。noq 从 Quinn 分出，支持 QUIC 原生多路径；这不是替换成任意 QUIC 库就能获得的完整 iroh 行为。[发布记录](https://github.com/n0-computer/iroh/releases/tag/v1.2.0)、[noq 官方说明](https://www.iroh.computer/blog/noq-announcement)

### 稳定身份与可变地址

EndpointId 是端点公钥身份，地址查找负责将它解析到 relay 或直接地址。默认 Pkarr 发布与 DNS/HTTPS 查找、可选 mDNS/DHT 是地址发现机制；它们不等于产品的成员名册或授权系统。[Address Lookup](https://docs.iroh.computer/concepts/address-lookup)

推论：可以分别回答“它是谁”“现在去哪找它”“它能做什么”。miopunch 已有身份和 roster 基础，值得审视的是这些语义是否必须与某一种 MQTT 投递流程绑定。地址可查不等于在线，更不等于有权打开 shell。

### 可用连接与直连优化

常见建链流程先借助 relay 建立连接，然后尝试直接路径；已知直接地址也可以直接建链，并非强制所有连接先过 relay。relay 同时帮助交换建路信息和转发端到端密文。[连接建立](https://docs.rs/iroh/1.2.0/iroh/)、[Relays](https://docs.iroh.computer/concepts/relays)

推论：relay 改变的是等待模型。允许中继时，打洞可以成为连接建立后的成本/性能优化，而不必一直阻塞首个业务字节。但这一结论依赖 relay 实际可达及容量足够，不能宣称无条件可连接。

### 路径存在于连接生命周期中

1.2.0 Connection 提供 `paths()`、`paths_stream()` 和 `path_events()`；事件包括路径打开、关闭、选中发送路径变化，及事件消费者落后。API 明确描述一个连接通常同时具有 relay 路径和打洞后的直接路径。[Connection API](https://docs.rs/iroh/1.2.0/iroh/endpoint/struct.Connection.html)

推论：`selected_path` 应是随时间变化的事实。一次性的 PathResult 可以继续服务于初始建路，但若目标包括持续迁移，仅增加一个 relay 枚举值不足以建立完整生命周期。迁移也需要传输层正确处理每条路径的 RTT、拥塞和验证，不能简单改写远端地址。

多路径不等于自动聚合带宽。当前 API 区分 open paths 与 selected transmission path；连接仍受超时和网络可达性限制。源码由远端状态管理候选、连接、打洞与路径选择。[固定版本远端状态实现](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket/remote_map/remote_state.rs)

### 安全连接之外还有产品授权

iroh 验证对端身份，但官方明确由应用决定是否允许该 peer 连接。[认证边界](https://docs.rs/iroh/1.2.0/iroh/)

推论：采用 iroh 后仍需定义邀请、设备所有权、成员权限、撤权以及 shell 授权。relay 使用权限与被控机器上的操作权限也必须分别处理。连接迁移不能解决进程重启、连接彻底失效后的终端恢复、输入去重或输出补发。

### 基础设施与语言集成

公共 relay 有限流且不保证可用性或性能；官方提供自托管源码。默认便利不等于没有基础设施依赖。[Relays](https://docs.iroh.computer/concepts/relays)

官方 FFI 仓库提供多语言集成，并将 Go 标注为 community maintained。由此不能推出必须全项目改 Rust，也不能假设 Go 集成与原生 Go 库同样简单。[iroh-ffi](https://github.com/n0-computer/iroh-ffi)

资料存在版本漂移：crate 总述仍保留旧 relay 协议措辞，当前企业部署文档描述 HTTPS/WebSocket。TCP 443 可通不能证明 WebSocket Upgrade 可用。具体协议应以固定版源码为准，不能把文档中的“总能连接”当作部署保证。[企业网络配置](https://docs.iroh.computer/configuring-networks)、[固定版本 socket 源码](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket.rs)

推论：后续若做验证，Rust 独立小程序可作为能力基线，再分别评估 Go FFI 与独立网络进程的构建、取消、流控、诊断及分发成本。此处没有选择集成方案。

## 3. 我的独立推论

### 产品目标需要重新排序

如果主目标是远程操作，应首先衡量首个可用 shell 的时间、操作延迟、掉线后的恢复、现场保留和用户需要干预的次数。直连率仍重要，但它解释成本和部分性能，不能代表完整可用性。

如果主目标是 NAT 穿透研究，自研 mode 决策、候选策略、实验室矩阵和失败证据则应继续成为主体。两种目标都合理，但会产生不同的研发投资顺序。

### 隔离打洞研究与日常使用的成功条件

一个可能的方向是保留纯直连实验模式，同时让日常远程操作允许受控 relay。这样困难 NAT 场景可以继续积累研究数据，使用者也不必等待每次实验成功。前提是用户重新接受 relay 的产品边界及部署成本；这不改变当前规格。

### 先审视抽象能否被消费，再扩展抽象

当前 PeerSession/LogicalStream 是可利用的上层边界；PathResult 仍包含 RuntimeKCPPacket 等实现相关事实。iroh 自己管理 Endpoint、建链与动态路径，把它当作“又一个返回原始 UDP socket 的 punching provider”可能失配。

若将来比较网络引擎，较自然的实验位置可能在已认证连接/流的边界，而不是强迫所有实现经过同一种原始 socket handoff。这只是基于当前结构的设计假设，需要小实验验证。

### 少做一层也可能比再抽一层更有价值

iroh 将可靠流、安全握手和多路复用集中在 QUIC 内。miopunch 的 KCP/TLS/yamux 组合并非因此错误，但维护三者组合的收益应通过弱网交互、吞吐、CPU 和复杂度来说明。没有对照实验，不能声称 QUIC 全面优于 KCP，也不能用未来可扩展性证明每一种 recipe 都值得维护。

### miopunch 的差异化可能在完整远程操作体验

值得投入的候选方向包括私有部署、清楚的设备授权、Windows/WSL/Android 工作流、不中断现场的 shell 和解释得清楚的网络诊断。这些是候选价值主张，不是市场调研结论。

## 4. 可比较的路线

| 路线 | 获得什么 | 必须承担什么 | 倾向 |
| --- | --- | --- | --- |
| Go 自研网络栈继续演进 | 算法控制、可解释性、现有代码复用 | relay、持续路径管理和兼容性维护 | 穿透研究为主时合理 |
| 采用 iroh 作为连接引擎 | 利用现成连接与路径能力 | FFI/进程边界、平台分发、升级与基础设施 | 远程操作产品为主时值得优先验证 |
| 完整 Rust 重写 | 统一语言并直接使用生态 | 同时承担产品重写与网络选型风险 | 当前证据不足以支持 |

## 5. 下一轮最有信息量的实验设想

这些实验尚未执行，也不是实施授权。

1. 相同主机、相同网络条件，对照当前 miopunch 与固定版 iroh 的首次业务字节时间；分别记录直接路径和 relay 路径。
2. 连接后切换 Wi-Fi/蜂窝或改变 NAT 映射，区分路径切换成功、连接重建成功、业务现场恢复成功。
3. 禁 UDP、断 relay、断地址查找服务分别测试；同时区分已建立连接和新建连接。
4. 在 Windows/WSL/Linux/Android 组合上验证，不能从桌面 Linux 成功外推手机后台存活。
5. 跑交互流与大流量并存、丢包/延迟场景，观察输入延迟、拥塞、CPU 和 relay 字节成本。
6. 确认未知身份、授权过期和撤权的行为；连接成功不能作为授权成功。

保留现有 NAT baseline，新增比较场景，不改变旧用例以制造更漂亮的结果。公开宣传的直连率不能直接用于预测本项目用户的网络分布。

## 6. 留给正式讨论的三个命题

1. miopunch 下一阶段主要交付穿透能力，还是可靠的远程操作体验？
2. 对直连失败的用户，私有 relay 是否属于可接受的正常运行方式？
3. 哪部分必须自己掌握：NAT 算法、连接引擎，还是身份授权与业务现场？

本轮倾向：先以远程操作体验为候选主目标，将 iroh 作为对照基线验证；保留已工作的 Go POC 和 NAT 实验资产，待实际差异出现后再决定网络引擎投资。
