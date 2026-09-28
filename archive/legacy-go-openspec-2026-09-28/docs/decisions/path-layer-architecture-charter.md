# 路径层架构分层纲领

日期：2026-05-03

## 文档状态

- 本文档用于收束一次架构讨论，为后续路线图、OpenSpec change 与实现拆分提供共同语境。
- 本文档只定义分层边界、关键判断与待讨论问题，不展开具体协议、字段、函数签名或实现任务。
- 本文档不替代现有 `P0-P3.5`、Door 2、Door 3 或主线网络测试文档；后续若要实施，应另起 OpenSpec change。

相关背景文档：

- `docs/roadmap.md`
- `docs/decisions/p3.5-public-network-charter.md`
- `docs/decisions/door-2-tcp-punching-charter.md`
- `docs/decisions/door-3-signaling-backend-charter.md`
- `docs/decisions/p3-miopunch-transport-charter.md`
- `docs/decisions/mainline-network-test-charter.md`

## 背景

当前 `miopunch` 已经具备较清晰的打洞、连通性增强与数据会话抽象：

- `connectivity / punching` 侧负责候选采集、交换与直连尝试。
- `dataplane` 侧负责 peer transport session、端到端身份绑定与 logical streams。
- 控制面已有 membership、governance、presence、reachability hints、mesh-first 与 mailbox fallback 等语义。

但真实网络讨论暴露出一个更高层的问题：大陆运营商限速、跨运营商互通、跨境链路、移动/高铁网络漂移、用户已有代理、未来私有转发点、以及 WireGuard 等能力，不能继续全部归入“承载层”或“打洞策略”。

这些问题的共同点是：它们本质上影响的是 **A 到 B 应该选哪条路**，而不只是某个 UDP/TCP socket 怎么建。

因此，本文档引入一个明确的 **路径层**，把“路径选择”从承载层和会话层之间独立出来。

## 分层总览

```text
产品/应用层
  sh / ping / file / virtual LAN mode / CLI / UI / user intent

会话层
  secure peer session / logical streams / resumable session / mux / keepalive / payload evidence

路径层
  path planning / path providers / path race / direct / peer-forward / private relay / route path

承载层
  TCP / UDP / QUIC / WebSocket / HTTP/2 / SOCKS / HTTP CONNECT / OS route / WireGuard interface

底层网络环境
  ISP / NAT / GFW / mobile / high-speed rail / cloud provider / cross-border route

控制面（横切）
  membership / governance / presence / node profile / edge history / policy inputs
```

核心边界：

- **产品/应用层** 回答“用户要做什么，以及这个任务需要什么体验”。
- **会话层** 回答“如何向应用提供连续、安全、可恢复的逻辑会话”。
- **路径层** 回答“A 到 B 走哪条路，以及为什么”。
- **承载层** 回答“miopunch 的字节借什么 carrier 发出去”。
- **底层网络环境** 是运营商、NAT、移动网络、跨境链路和云厂商策略等外部现实。
- **控制面（横切）** 提供身份、治理、状态、画像、历史与策略输入，但不承载应用数据。

命名说明：

- 本文档中的 **承载层** 不是 OSI L2 link layer；它是 `miopunch` 使用的 carrier substrate。
- 本文档中的 **会话层** 对应现有 `dataplane` 的长期演进方向，不只是当前单个 transport session。
- **控制面（横切）** 不是辅助旁路，而是横切产品/应用层、会话层、路径层的核心控制平面。

## 分层定义

### 产品/应用层

产品/应用层表达用户能力与用户意图，不直接选择 NAT 策略、代理或承载 carrier。

当前与未来可能包括：

- `ping`
- `sh`
- 文件传输
- 虚拟局域网 / TUN / route mode
- CLI / GUI 的状态展示、诊断报告与用户操作

产品/应用层可以向路径层表达 intent，例如：

- 交互优先：适合远程 shell，高铁/蜂窝场景需要更快恢复。
- 稳定优先：允许更早使用已知稳定路径。
- 直连优先：愿意等待更多 direct attempt。
- 成本优先：避免消耗 peer forwarding 或私有 relay 带宽。
- 吞吐优先：适合文件传输或未来虚拟局域网。

产品/应用层不应直接理解某个公网映射、运营商、STUN view 或 proxy 配置。

虚拟局域网需要双重表述：

- 在产品/应用层，它表现为一种用户可见的 network mode。
- 在会话层，它可能表现为 packet/TUN payload 类型。

### 会话层

会话层从路径层接收一条已选或已验证的 path，然后建立端到端安全逻辑会话。

会话层负责：

- 端到端身份绑定。
- TLS 1.3 / QUIC TLS 等安全握手。
- peer transport session 生命周期。
- logical stream 生命周期。
- resumable logical session。
- 多路复用。
- keepalive。
- close reason。
- payload evidence。

会话层不应关心底层 path 是直连、peer 转发、私有 relay、代理 carrier，还是已有 OS route。

只要路径层交付的是可承载字节流或报文的路径，会话层就应该能建立统一的 secure peer session，并向产品/应用层提供 logical streams 或其它 payload session。

会话层职责可以分成两个演进档位：

- **peer transport session**：当前已有方向，覆盖 TLS/QUIC/yamux、logical stream、session 生命周期与 payload evidence。
- **resumable logical session**：未来高移动性场景需要，允许底层 transport session 或 path 重建后恢复同一个应用逻辑会话。

高移动性 `sh` 的目标应写成：

- 承载层可以断。
- 路径层可以换路、重连、重新竞速。
- 会话层可以重建 secure transport，并恢复同一个 logical session。
- 产品/应用层的 `sh` task 不应退出；用户最多感知卡顿、延迟变大或吞吐下降。

### 路径层

路径层是本文档新增的核心边界。

路径层负责：

- 收集可用 path candidate。
- 根据用户 intent、网络画像、历史结果、治理约束与本地配置生成 path plan。
- 执行或调度 path race / fallback。
- 输出 selected path、failed paths、原因、预算、证据与 stop condition。
- 将可用 path 交给会话层。

为避免路径层变成新的“大泥球”，路径层内部应至少区分两个子边界：

- **Path Control**：planner、policy、scoring、edge history、预算、path race 决策。
- **Path Provider**：实际提供候选路径的实现，例如 direct provider、peer-forward provider、private relay provider、route provider。

路径层中的 path provider 可以包括：

- direct path provider：现有 TCP/UDP/IPv6/portmap/punching 流程。
- peer-forward provider：未来可能由网内 peer 转发密文。
- private relay provider：未来可能由用户私有网络内的稳定转发点承载密文。
- proxy/carrier provider：未来可能通过用户已有代理或标准 carrier 连接到下一跳。
- WireGuard/OS-route provider：当系统已有可路由路径时，把它作为 path candidate。

路径层的关键判断：

- 大陆运营商限速、跨运营商互通、跨境链路、移动网络漂移和用户已有代理，优先建模为路径选择问题。
- 直连仍是重要 path provider，但不应垄断“能不能连”的全部语义。
- 高铁/蜂窝/跨境/历史失败多等场景可以影响 path plan 与预算，但具体策略仍需后续讨论。

`auto` 语义需要区分当前与未来：

- 当前 `p2p_network=auto` 属于 direct provider 内部固定顺序：`tcp6 -> tcp4 -> udp6 -> udp4`。
- 未来 path-layer `auto` 才是跨 direct、forward、relay、carrier 的 planner。
- 后续文档和实现不得把这两个 `auto` 语义混用。

### 承载层

承载层负责字节承载，不表达 `miopunch` 的身份、治理、membership 或应用语义。

承载层可能包括：

- TCP
- UDP
- IPv4 / IPv6
- QUIC over UDP
- TLS over TCP
- WebSocket over TLS
- HTTP/2 stream
- HTTP CONNECT
- SOCKS5
- 用户已有代理 egress
- WireGuard interface
- OS route

承载层能力可以被 direct path、relay path、proxy/carrier path 或 route path 使用。

承载层不应承担：

- peer 身份认证。
- network governance。
- 是否允许某个 peer 转发。
- 是否应当优先跨境。
- 是否为某个产品/应用 intent 选择某条路径。

### 底层网络环境

底层网络环境是系统无法完全控制的现实环境。

可能包括：

- 中国大陆 / 非中国大陆网络环境。
- 运营商与跨运营商链路。
- NAT 类型、过滤行为与端口映射行为。
- UDP/TCP 限速、丢包、阻断。
- GFW、DPI、干扰、阻断与跨境不稳定。
- 蜂窝、高铁、Wi-Fi、公司网、校园网等高变化环境。
- 云厂商端口、备案、合规与网络策略限制。

底层网络环境事实可以进入 node profile、edge history 或一次性 exchange 证据，但它们不应直接污染会话层协议。

### 控制面（横切）

控制面为路径层、会话层和产品/应用层提供事实与约束。

已有或未来可能包含：

- membership。
- governance。
- decls。
- presence。
- reachability hints。
- node profile。
- edge history。
- relay / forwarding capability。
- network-level policy。
- 本机 policy override。

控制面事实需要标注：

- `source`：事实来自用户配置、自动探测、对端上报、STUN view、历史 edge，还是测试夹具。
- `ttl`：事实多久后过期。
- `scope`：事实仅本机可见、点对点可见、邻居可见，还是网络级可见。
- `confidence`：确定、可能、推断、用户声明。

## 与现有架构的关系

### 与 direct connectivity 的关系

现有 `gather / exchange / attempt / punching` 可以视为路径层里的 **direct path provider**。

该 provider 仍然负责：

- IPv6 direct。
- IPv4 direct。
- portmap direct。
- UDP punching。
- TCP direct。
- TCP punching。
- STUN view selection。
- attempt path 证据。

但它不应负责：

- 选择是否使用 peer 转发。
- 选择是否使用私有 relay。
- 选择是否使用用户代理作为 carrier。
- 根据 shell / file / virtual LAN intent 决定全局路径策略。

### 与 dataplane / 会话层的关系

路径层输出可用 path 后，会话层建立 secure peer session。

会话层继续维持既有原则：

- 端到端身份绑定。
- logical stream 独立于 transport session。
- payload exchange 必须可观测。
- 关闭 logical stream 不应关闭 peer session。

未来如果要支持高移动性场景，会话层还需要补齐 resumable logical session：

- 底层 transport session 可以重建。
- logical session 可以在新 transport 上恢复。
- 产品/应用层 task 不应因为底层重建而退出。

未来即使引入 peer-forward 或 private relay，relay 也只应转发密文，不拥有解密能力或网络授权权威。

### 与 signaling backend 的关系

Door 3 讨论的是 control-plane backend：bootstrap、mailbox、exchange。

路径层讨论的是 data path / peer path：应用 payload 经过哪条 path 到达对端。

二者可能共用某些底层 carrier 或部署设施，但语义必须分开：

- signaling backend 不等价于 data relay。
- data path 不能因为走了某个 backend 而降低端到端身份绑定。
- backend/relay/carrier 一律按不可信承载建模。

### 与 WireGuard 的关系

WireGuard 不应被定义为中国大陆复杂网络的默认保底路径。

WireGuard 更适合两个位置：

- 作为已有 OS route / interface，被路径层识别为一条可用 path candidate。
- 作为未来虚拟局域网方向的底层能力或参考对象。

WireGuard 不应替代产品/应用层 `sh` 的 lightweight path planning，也不应把当前产品主线强行拉向 L3 VPN。

## 私有转发点与公开 relay 边界

本文档只为未来私有转发能力预留路径层位置。

当前讨论边界：

- 不提供公开 relay 服务。
- 不鼓励用户公开部署 relay。
- 不承诺抗封、伪装或绕过审查能力。
- 不在本文档选型 relay 协议或 carrier。
- 未来若存在 relay/forwarding 组件，应优先定位为用户私有网络内的能力。

未来可讨论的问题：

- 是否存在 `peer-forward`：由网内 peer 临时转发密文。
- 是否存在 `private relay`：用户私有网络内的稳定转发点。
- 是否允许 relay capability 进入 network-level governance。
- 是否允许本地配置把某个 proxy/carrier 作为连接 relay 的 egress。
- relay 是否只提供 path forwarding，不参与 membership、governance 或数据解密。

## 高移动性场景：高铁 / 蜂窝 / 频繁切换基站

高移动性场景不是单纯的承载层问题，也不是单纯的会话层问题，而是路径层与会话层协作问题。

典型场景：

- 控制端在高铁、蜂窝或其它频繁切换基站的网络中。
- 被控端可能在家庭网络、公网 IP、NAT1、portmap、私有转发点或其它稳定入口后面。
- 业务流量可能很小，例如远程 shell，但用户要求任务持续可用。

目标口径：

- 产品/应用层 `sh` task 保持运行，不要求用户重新执行命令。
- 会话层负责恢复同一个 logical session，而不是把底层断连暴露成应用退出。
- 路径层可以采用更激进的 reconnect、候选刷新、path race 或备用 path 预热。
- 承载层可以频繁断开和重建。

待讨论方向：

- V1：应用任务不退出，底层自动重连，恢复到同一个 shell 现场。
- V2：resumable logical stream，具备 `session_id / seq / ack / ring buffer / input queue` 等能力。
- V3：多路径或迁移能力，在多个 path 间并行维持或快速切换。

本文档不决定上述方向的实现顺序，只记录该场景对分层职责的要求。

## 待讨论问题

### 1. 产品/应用层问题

- `ping`、`sh`、文件传输、虚拟局域网是否需要不同默认 path intent。
- `sh` 是否应默认更偏稳定和快速恢复，而不是完整等待所有 direct attempt。
- 文件传输是否需要吞吐优先、成本约束和更强路径质量评估。
- 虚拟局域网是否应作为独立产品模式，而不是 `sh` 能力的自然扩展。

### 2. 会话层问题

- peer transport session 是否需要支持路径切换或迁移。
- path 切换时 logical stream 是否应保持，还是允许会话层恢复新的 transport 后继续同一个 logical session。
- payload evidence 应如何统一表达 direct、forward、relay、proxy-carrier 路径。
- keepalive 应由会话层统一做，还是由不同 path provider 提供建议。
- relay/forward path 上的会话层身份绑定是否需要额外 transcript 绑定路径信息。
- 高移动性 `sh` 是否需要 resumable logical session。

### 3. 路径层问题

- 未来 path-layer `auto` 的正式定义是什么：固定顺序、加权排序、还是 path planner。
- 是否默认 path racing。
- direct 与 relay/forward 的并行预算如何设定。
- 高铁、蜂窝、跨境、历史失败多等场景是否应缩短 direct 预算。
- path 评分是否应基于 edge history、node profile、实时 probe 或用户 intent。
- peer-forward 与 private relay 是否进入同一 path graph。
- path provider 失败时的 reason_code 与用户建议如何表达。
- Path Control 与 Path Provider 的接口边界应如何表达。

### 4. 承载层问题

- 哪些 carrier 是一等能力，哪些只作为用户已有环境接入。
- WebSocket / HTTP/2 / HTTP CONNECT / SOCKS5 是否应被抽象为统一 carrier adapter。
- 用户已有代理是否只作为 local egress，而不是 `miopunch` 自带代理协议。
- WireGuard route 如何被识别为可用 path，而不把 `miopunch` 变成 WireGuard 管理器。
- 云厂商端口、备案、443 可用性等现实限制是否只进入部署指南和诊断建议。

### 5. 控制面问题

- node profile 应包含哪些字段：
  - 地区。
  - 运营商。
  - ASN。
  - 接入类型。
  - NAT 特征。
  - 移动性。
  - relay/forward capability。
  - proxy egress capability。
- 哪些画像自动上报，哪些仅在一次 point-to-point exchange 中交换，哪些只留在本机诊断报告。
- profile 的 `source / ttl / scope / confidence` 如何表达。
- edge history 是否只本机保存，还是可在邻居之间共享摘要。
- network-level policy 与本机 local override 冲突时谁优先。

### 6. 测试问题

- lab 如何验证 path layer，而不提前实现所有真实网络复杂性。
- MNT 场景是否需要新增 path planner evidence。
- direct path provider 的现有矩阵是否保持独立，不与 relay/forward 组合爆炸。
- 是否需要真实网络 smoke：
  - 大陆家宽。
  - 大陆蜂窝。
  - 跨运营商。
  - 跨境。
  - 高移动性网络。
- path evidence 是否应成为 future gate 的稳定输出。
- 高移动性场景是否需要独立的 session continuity evidence。

## 初步结论

- `miopunch` 应明确引入路径层，把“怎么选路”从承载层和会话层中分离出来。
- 中国大陆复杂网络、跨境链路、代理、未来私有转发点与 WireGuard route，优先作为路径层问题讨论。
- 现有 direct connectivity 是路径层的第一个 provider，不是路径层的全部。
- 会话层继续维持端到端安全 peer session 与 logical streams，并在未来承担应用逻辑会话连续性。
- 控制面负责提供画像、历史与策略输入，但需要明确事实的来源、时效、作用域与置信度。
- 未来 relay/forwarding 讨论应限定在私有网络能力与不可信密文转发，不进入公开 relay 承诺。
