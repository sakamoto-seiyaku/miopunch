# iroh 内部流程走读与改造落点

日期：2026-09-27。状态：源码研究和候选架构，不是实施规格。

基线为 iroh v1.2.0，固定提交 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`。完整源码下载到 `/tmp/miopunch-iroh-v1.2.0`；本文行号来自本地文件。没有构建、运行或修改 iroh，没有创建维护分支。下文把现有行为和建议改法分开。

## 1. 最新讨论前提

用户希望深入修改 iroh 本身，不是在外面再包一套连接管理层。对外尽量保留其连接接口，必要时增加配置或接口；p2panda 尽量保持原样，只做必要接入适配。

原生 relay 是用户感兴趣并希望最终产品具备的能力，不应因去中心化目标而被默认删除。目标是不必依赖官方或固定专用服务器，不是禁止用户部署或使用 relay。MQTT 等仅消息渠道与 relay 数据路径并存。

当前优先工作是理解 iroh 的实际步骤，把明确需求嵌入这些步骤。现有 miopunch 的 UDP 建连流程和实验经验是参考，不要求逐文件移植 Go，也不预设所有策略已经设计完成。

## 2. 现有内部职责

| 模块 | 现有职责 |
| --- | --- |
| `Endpoint` / `Builder` | 对外连接、接受连接、身份、配置和生命周期接口 |
| `EndpointInner` / `Socket` / socket actor | 共享 QUIC endpoint、物理收发、本地地址、网络变化、分发 peer 消息 |
| `RemoteMap` | 按远端 EndpointId 创建/复用 actor；维护地址映射 |
| `RemoteStateActor` | 一个远端的候选、多个 QUIC 连接、QNT 调度、路径选择与恢复 |
| `RemotePathState` | 潜在远端地址、来源、可用性、等待地址的请求 |
| `Transports` | IP、原生 relay 和 custom 的包收发汇合 |
| `RelayActor` / `ActiveRelayActor` | relay 连接、重连、home relay 与转发路径 |
| `noq` | QUIC/TLS 握手、流、丢包恢复、拥塞控制、路径验证、多路径与 QNT |

一个远端 actor 可以管理同一 peer 的多个 QUIC 连接，不应误画成全局“每个 peer 只有一条连接”。[Socket](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs)、[RemoteMap](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map.rs)、[RemoteStateActor](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs)

## 3. 当前建连流程

```text
Endpoint.connect(peer, ALPN)
  -> 检查配置和 before_connect
  -> resolve_remote
       -> RemoteMap / 对应 RemoteStateActor
       -> 合并显式地址和 AddressLookup 结果
       -> 只要有潜在地址，即可完成解析
  -> noq 创建 QUIC 握手
  -> 初始包交给远端 actor
       -> 已选路径，或已知 IP / relay / custom 候选
  -> QUIC 握手完成、验证远端身份
  -> 注册回同一远端 actor
       -> 安装路径事件和候选更新
       -> 后台发起 QNT、验证新路径、选择路径
  -> 对外提供 Connection / streams
```

对外 Connection 不要求等待 UDP 打洞成功：首条 relay 或直连路径上的握手完成后即可承载业务，后续路径改善在后台进行。

`Endpoint::connect_with_opts` 内部先解析远端，之后调用 noq。解析完成只意味着有一个可能的地址，不意味着双方已经协商好打洞计划，更不意味着路径已可达。[endpoint.rs:1100](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1100)、[path_state.rs:161](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state/path_state.rs#L161)

noq 通过内部合成地址标识要联系的 EndpointId；iroh 将初始包发送到已选路径，或发送到多个已知候选。底层 IP、relay、custom 收包统一送回 QUIC 引擎。这是可以继续利用的汇合结构，不必为每种联络渠道单独创建一套对外会话。[transports.rs:1167](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports.rs#L1167)、[初始包发送](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L788)

连接建立后，`conn_from_noq_conn` 和 `register_connection` 把入站/出站连接汇入同一管理结构。普通入站协商消息本身不是一次成功的 `accept`；仍需实际 QUIC 握手及身份验证。[connection.rs:316](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L316)、[socket.rs:1354](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L1354)

## 4. relay 在其中不是单一的备用转发器

原生 relay 的包转发路径能承载最初的 A-B QUIC/TLS 握手；端到端连接建立后，QNT 在这个连接内部交换候选、协调探测。直连不成或路径失效时，relay 还能继续承载业务数据。

因此 relay 是 QUIC 下面的一条路径，不是代替业务层重建另一套连接。转发服务器不负责终止 A-B 的端到端 QUIC，不负责 miopunch 的成员/服务授权，也不因转发而获得业务明文。但它仍能观察连接元数据，不能把内容加密说成匿名。

QAD 外部地址观察与上述包转发是不同机制，QNT 打洞协调又属于端点的 QUIC 连接。即使这些功能部署在同一台机器上，改造时也应分别认识它们的职责。[relay actor](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs)、[relay QAD](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-relay/src/quic.rs)

QAD 服务端是单独的 QUIC listener，包转发则使用 HTTP(S)/WebSocket。客户端 v1.2.0 已复用主 noq Endpoint 发起 QAD 连接，并保留连接及地址观察 watcher，用于后续复用/变化通知；不是随便新建一个与数据路径无关的 UDP socket。改造候选来源时应保留这个重要性质。[net_report](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/net_report.rs)

两端的 home relay 不同时，不应画成固定 R1 -> R2 服务器互转：客户端会按目标的 relay URL 建立额外连接，也可能复用此前已收到该 peer 流量的 relay 路由。home relay 不是一个 endpoint 唯一允许连接的 relay。[active_relay_handle_for_endpoint](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs)

建议保留原生 relay 的路由、连接维护和恢复能力，令它与新建连机制共同参与内部策略，不全局等待 MQTT 协商才允许尝试一个已可用 relay。

## 5. 新需求应进入哪个内部步骤

### 5.1 在 QUIC 建立前增加真实协商状态

目前 `RemoteStateActor` 收到地址就可以推进初始握手；QNT 则在已有连接注册后触发。仅把 MQTT 做成地址查询，不能表达 offer/answer、双方准备完成、打洞角色与调度。

候选改法：在现有 peer actor 内加入建连前协商状态，并由 Endpoint/socket 管理信令收发。MQTT、已有节点转交等消息按远端身份进入同一个 actor；本地主动 connect 和对端主动协商也在此汇合。不能由每个渠道回调另起一套独立连接管理。

区分“地址已知”和“本次尝试已准备好”，保留清楚的解析语义，必要时新增内部建立路径请求。共享的 peer 级协商与多个应用 connect 请求也要区分，取消其中一个请求不应误杀其他请求。

允许两条路线汇合：已有可用 IP/relay 可以启动 QUIC；需要建连前打洞的路线先协商准备，再进入相同握手。MQTT 只搬运控制消息，不要求承载 QUIC 包。接收侧在协商后准备套接字/候选，最终仍从现有 QUIC incoming/accept 接口交付连接。

### 5.2 将 miopunch 已有流程融入其所有者

| 现有 miopunch 经验 | fork 内部候选落点 |
| --- | --- |
| 采集本地 UDP/NAT 快照 | Socket 本地地址与观察管理 |
| offer/answer、关联与去重 | 内置信令模块及 peer actor 协商状态 |
| NAT 类型、角色与策略决策 | peer actor 的尝试计划 |
| 发送探测、端口复用、预算/取消 | 具体 transport 的尝试执行及统一请求生命周期 |
| 可用路径和 socket 所有权交付 | 现有 IP/transport 收发及 QUIC 握手/路径验证 |
| 失败证据 | 当前连接/路径诊断体系，增加协商阶段信息 |

旧代码参考：[Dial](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/service.go)、[交换与决策](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/exchange.go)、[尝试及 PathResult](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/runtime.go)。

候选地址、探测有回应、QUIC 身份握手完成、最终选中路径，是四个不同状态。尤其必须保留用于观察和打洞的 UDP 套接字/映射，不能探测完成后换一个 socket。必要时在 IP transport 内增加非 QUIC 探测的分流，但不绕过 QUIC 的路径验证。

### 5.3 地址观察不再依附是否配置 relay

保留现有 relay QAD，同时让本地地址、端口映射、其他可用观察来源独立更新。当前 `DirectAddrUpdateState::run` 在 RelayMap 为空时返回，后续端口映射请求也不执行，属于明确的内部改造落点。[socket.rs:778](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L778)

观察结果需要对应 socket/网络代次、来源和寿命，再送入已有本地候选更新路径。目的相关 NAT 下，C 看到的地址只可作为与 B 通信的候选；不能把 MQTT 的 TCP 源地址误作 UDP 映射。

### 5.4 新载体应融入内部路径语义

TCP、伪装载体和普通节点中继不应只是全部标为“Custom”就结束。可以在 fork 的 `TransportConfig`、地址和路径元数据中表达其真实特性，再进入现有收发/管理结构；具体枚举或结构形式尚未决定。[Transports](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports.rs)

必须显式讨论：TCP 是承载同一 QUIC 会话的包，还是另一个独立会话引擎。前者更容易保留 Connection/stream 和跨路径语义，但有可靠载体队头阻塞等代价；后者不是简单新增路径，会扩大会话抽象与兼容工作的范围。本文不悄悄替用户选定。

普通节点中继可以借鉴现有 relay 的加密包转发和连接维护，但在已有 peer 链路上建电路，还需定义准入、路由寿命、防环、排队与预算；不能把服务器 relay 更换名称就视作完成。

### 5.5 选路、停止探测与换网恢复必须一起改

固定版本中发现三项相关内部假设：

1. `BiasedRttPathSelector` 把 IP 和 Custom 当 primary，原生 Relay 当 backup，之后才比较 RTT。因此仅添加自定义路径，可能使一个普通节点中继无条件排在原生 relay 前面，也不能解决低 RTT 但被限速的 UDP。[选路实现](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs)
2. `check_connections` 用“有延迟足够低的 IP 路径”判断连接是否足够好。即使 TCP/其他路径是主动选择的好路径，也可能继续驱动 UDP 打洞。[检查实现](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L751)
3. 换网时，`notify_quic_network_change` 对原生 relay 返回可恢复，因为 relay actor 会重连；对 Custom 一律返回不可恢复。新 TCP/peer relay 需要按实际恢复能力分类。[换网提示](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L1697)

因此应一起改路径属性、选路策略、探测预算和恢复策略，而不是在上层再放一个监控器，与内部选路来回争夺控制。

## 6. 保留与修改的总轮廓

```text
尽量保留 Endpoint / Connection / stream 的对外用法
                    |
       改造现有内部 peer 连接状态管理
       /             |              \
  已知直连路径    原生 relay 路径    新的建连前协商
                                  MQTT / 其他消息渠道
                                         |
                                  NAT 策略与路径准备
       \             |              /
        汇入现有 QUIC 握手、验证、连接注册
                    |
          扩展后的选路、探测与恢复策略
```

这是内部改造，不是新增包裹 Endpoint 的外层连接框架。现有模块可以拆分或重组，但连接状态、socket 所有权和取消/重试不能各管一套。

暂以复用 noq 的成熟 QUIC 能力为出发点。若后续证据要求调整 QNT、路径验证或拥塞/恢复行为，则把 noq 修改列为明确任务；不能从“深度 fork”推出所有下层都必须重写。

## 7. 讨论与验证顺序

先确认 QUIC 建立前的协商怎样嵌入现有 peer actor，并让原生 relay 与新路径正确并存。这比先设计所有伪装方式更能检验整体架构。

最小验证目标不是额外写一个外层调用顺序，而是上层仍调用一次 connect：内部可以选择原生 relay，也可以经 MQTT 协商直连；接收侧正常 accept，成功后提供相同 Connection。覆盖双向同时发起、多渠道重复、取消、协商期间换网和路径胜出后的其他尝试收尾。

随后再确定 TCP 会话语义、质量选路、普通 peer relay，并检查 p2panda 接入是否只需构建配置适配。接口兼容还包括超时、取消、身份与流行为，不只是函数签名；线协议与未修改节点的互通另行定义。

本文结论来自源码，不表示上述新流程已经实现或经过 NAT 实验验证。
