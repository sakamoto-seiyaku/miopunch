# miopunch 传输适配调查：TCP、节点中继与共同 ACL

日期：2026-09-27。状态：探索笔记，不是实施决定。

阅读提示：第 1 至 6 节保留前一轮能力对照；第 7 节补充允许 fork、维护风险和去中心化改造边界。最新渠道/流程边界、Rust 新实现及 fork 职责见第 7.6、7.7 节；中继 R 的例子不再是预设总流程。

后续已按“深入修改 iroh 内部并保留原生 relay”进行固定源码走读。更具体的流程、状态所有者和修改位置见 [iroh 内部走读](2026-09-27-iroh-internal-redesign-walkthrough.md)，不再以外部适配是否够用作为主要分析框架。

本次对照官方文档、协议与固定版本源码；未运行候选实现，未测大陆网络成功率、吞吐或切换效果。核查版本为 iroh 1.2.0、p2panda 0.7.1、go-libp2p v0.50.0、rust-libp2p v0.57.0。“可接入”不代表已有通过测试的适配器。

## 1. 本轮需求怎样改变设计

共同对象是“源节点 -> 目标节点 -> 服务协议/端口”。可选 TUN 与无 TUN 的代理/端口转发是入口，TCP、UDP、专用 relay、普通节点 relay 是承载。两者不应改变 ACL 含义。

```text
TUN 入口             代理 / 端口转发入口
    \                 /
       服务目标与共同 ACL
       已认证的端到端身份
       连接 / 会话实现
    /        |          \
UDP路径    TCP类路径    节点或专用中继
```

这是建议的职责分工，不是要求预先实现通用 transport 框架。例如 T 经 R 访问 S 的 TCP 8080：S 按原始已认证身份 T 检查权限，不能把它当成 R。有效范围仍是 S 的暴露范围与 T 的获准范围交集。R 的带宽准入是单独的中继政策。业务 ACL 不阻断 miopunch 自身的组网与权限补齐通道，但后者仍有各自的认证和限额。

源、目标、协议/端口有成熟语义参考，如 [Tailscale grants](https://tailscale.com/docs/features/access-control/grants)，不意味着采用其控制面。TUN 处理 IP 包，代理处理连接，路由、地址绑定、防源地址伪造、报文/连接状态和目标映射仍需各自实现。共同 ACL 不等于同一个执行器。[Linux TUN/TAP](https://docs.kernel.org/networking/tuntap.html)

## 2. 三种 TCP 不能混为一谈

| 候选 | 能力 | 不自动提供 |
| --- | --- | --- |
| 真 TCP 直连、TCP 打洞 | 真正的可靠字节流，可组合现成认证和复用 | 任意 NAT 都成功；已有 UDP/TCP 会话互相迁移 |
| TCP/TLS/WebSocket 承载 QUIC 包 | 外层真 TCP，内层保留 QUIC 会话 | 消除 TCP 队头阻塞；流量与普通 HTTPS 不可区分 |
| FakeTCP | 把数据报包装成类似 TCP 的报文 | 普通可靠 TCP 流；通过所有 L7 代理；免权限跨平台 |

iroh 官方 Tor transport 展示了将 QUIC `Transmit` 编码到 TCP 字节流。因此不能说 iroh 完全不能走 TCP。适配仍需分帧、队列、背压、失败处理；外层 TCP 丢包会阻塞后续包，内层多个 QUIC 流也会受影响。载体有效报文能力至少满足 QUIC 的 1200 字节，必要时适配器负责分片重组。[官方 Tor 示例](https://www.iroh.computer/blog/tor-custom-transport)

Phantun 保留乱序等 UDP 特性，不提供真 TCP 的重传和流控；针对 L3/L4 设备，不承诺穿过 L7 代理。现成 `fake-tcp` 库可供研究，但需计入 TUN、路由、防火墙及 `CAP_NET_ADMIN` 要求。载体内部 TUN 与是否开启业务虚拟网卡是两件事。[Phantun](https://github.com/dndx/phantun)

[udp2raw](https://github.com/wangyu-/udp2raw) 是另一类现成 raw-socket 封装候选。存在实现，不代表已满足所有平台、维护和性能要求，也不保证规避具体网络的限速或识别。

## 3. iroh：统一会话，补充载体

可复用节点认证、QUIC 流、UDP 路径、现成 relay，以及同一 QUIC 连接的多路径机制。Custom transport 扩展位于 QUIC 之下，发送/接收包，并不是任意 TCP stream 可以直接替换 iroh stream。[CustomEndpoint，核查时为 1.2.0](https://docs.rs/iroh/latest/iroh/endpoint/transports/trait.CustomEndpoint.html)

需要补充的部分：TCP 载体、TCP simultaneous-open 和地址协调、伪装及平台适配。可达普通节点嵌入 relay 服务，与经普通节点已有连接建立 peer circuit 不是一回事；后者仍需电路协议、准入、限额和生命周期管理。

将载体注册为同一 QUIC 连接的路径，具有保留已有流的架构优势，但新适配器是否能正确切换必须实验验证；超过连接超时也不能无限续接。当前扩展仍为 `unstable-custom-transports`，源码明确不受 semver 稳定性保证。[扩展发布说明](https://www.iroh.computer/blog/iroh-0-97-0-custom-transports-and-noq)、[1.2.0 endpoint 源码](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/endpoint.rs)

判断：如果更换外层路径时保留已有连接最重要，iroh 仍有吸引力；不能因为需要 TCP 就排除。但 TCP 打洞、节点 circuit 和伪装适配不能算作它已经替我们完成的工作。

## 4. libp2p：连接方式齐全，已有流不自动迁移

已有 TCP、QUIC、WebSocket、身份认证、安全通道与流复用。Rust 官方 DCUtR 示例组合 TCP + Noise + yamux、QUIC、relay、identify；Go TCP 实现含端口复用与 simultaneous-connect 角色处理。[Rust 示例](https://github.com/libp2p/rust-libp2p/blob/v0.57.0/examples/dcutr/src/main.rs)、[Go TCP](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/transport/tcp/tcp.go)

DCUtR 经已有中继连接交换地址、协调 TCP/QUIC 打洞；不是成功率保证。Go `EnableHolePunching` 仍标 experimental，默认不开。[DCUtR 规范](https://github.com/libp2p/specs/blob/78e75c6f4e5dd9de4bc1524c686c0f45df4b461a/relay/DCUtR.md)、[Go options](https://github.com/libp2p/go-libp2p/blob/v0.50.0/options.go)

关键代价：DCUtR 规范步骤 6 明确新流优先走新直连，旧中继关闭后，已有长流要重建。“支持多 transport”不等于“流无感迁移”。业务层须选择留在旧路径、关闭重连，或另加可恢复会话层；最后一种不是免费收益。

这里说的是 libp2p stream，不代表所有 TUN 内层 TCP 必然断开。若报文转发层能重建外层通道并保留虚拟地址，内层 TCP 可能通过重传跨过短暂中断；这是报文层的行为，仍需验证。代理将本地 TCP 绑定到某条 stream 时，则不能直接得到这种保证。

### 普通节点中继

Circuit Relay v2 提供预约、电路建立、资源限制；终端可以在中继流上建立自己的安全会话，保留原始两端身份。[Relay v2 规范](https://github.com/libp2p/specs/blob/78e75c6f4e5dd9de4bc1524c686c0f45df4b461a/relay/circuit-v2.md)

不能笼统说 R 必须有公网地址。Go 底层 relay 构造不检查公网可达性，客户端向 R 开协议流能复用已有连接，R 向目标开流明确使用 `WithNoDial`。据此推断：若 A-R、B-R 已有可用非 circuit 连接，NAT 后的 R 也值得实验复用现成服务，不必先自制中继协议。[预约](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/protocol/circuitv2/client/reservation.go)、[拨号](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/protocol/circuitv2/client/dial.go)、[relay 实现](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/protocol/circuitv2/relay/relay.go)

这只是源码可行性判断，未实测。默认自动服务生命周期只在公网可达时开启；底层 handler 拒绝经另一条 circuit 递交预约/连接，不是开箱任意多跳 mesh。[自动服务生命周期](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/host/relaysvc/relay.go)

Go 默认电路限时两分钟、每方向 128 KiB，不能直接当私人网络长期大流量通道。可配置限额，仍应保留适合自己设备的资源预算。[资源配置](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/protocol/circuitv2/relay/resources.go)

### 其他边界

- TCP/TLS/WS 不等于伪装；原生 libp2p TLS 含 PeerID 认证扩展与 `libp2p` ALPN，不承诺与普通 HTTPS 不可区分。[TLS 规范](https://github.com/libp2p/specs/blob/78e75c6f4e5dd9de4bc1524c686c0f45df4b461a/tls/tls.md)
- 默认连接排序不是业务吞吐驱动的持续优化器。“UDP 能通但限速”需比较完成时间、延迟和稳定性，不能只看打洞成功或 RTT。[Go 拨号排序](https://github.com/libp2p/go-libp2p/blob/v0.50.0/p2p/net/swarm/dial_ranker.go)
- 流传输不等于 TUN/IP 网络；UDP 业务封装、MTU、地址绑定、代理执行仍需集成。底层 QUIC 用 UDP，不代表上层自动提供原始 UDP 数据报 API。
- Go/Rust、浏览器/原生支持需分别验证。Rust 原生 WebRTC crate 仍带 alpha 版本，浏览器使用另一个 websys 实现。[Rust workspace](https://github.com/libp2p/rust-libp2p/blob/v0.57.0/Cargo.toml)
- 产品的私人网络不自动等于 libp2p 的 `PrivateNetwork` PSK 模式。Go 默认私网 transport 集合只有 TCP/WS，会排除普通默认集合中的 QUIC 等；成员资格应与具体连接握手机制分开判断。[Go 默认装配](https://github.com/libp2p/go-libp2p/blob/v0.50.0/defaults.go)

## 5. p2panda：同步数据不必绑定整套网络

p2panda 提供签名操作、持久日志、补齐和可选组权限，不是现成 TUN/代理网络。对 P 修改规则、S 保存、P 离线、T 从 S 补齐的场景，仍有复用价值。[core](https://docs.rs/p2panda-core/latest/p2panda_core/)、[store](https://docs.rs/p2panda-store/latest/p2panda_store/)

`p2panda-sync` 明确可独立使用，在 `Sink`/`Stream` 上运行两方同步协议。可研究由 libp2p 或其他认证连接承载其消息，省去相应同步算法与状态机；仍须集成分帧、存储、同步对象和权限策略。[sync API](https://docs.rs/p2panda-sync/latest/p2panda_sync/)

但 `p2panda-net` Endpoint 暴露 iroh endpoint、返回 iroh Connection、接收 ALPN/QUIC 配置。移除 iroh 不是改一个配置项，需要适配其网络编排；接受 iroh 才能更完整复用这一集成层。[Endpoint API](https://docs.rs/p2panda-net/latest/p2panda_net/iroh_endpoint/struct.Endpoint.html)

普通冲突已允许采用成熟的自动规则，不应继续以“必须人工裁决”排除 p2panda-auth；仍须核对创世恢复与服务端口 ACL。组管理/数据写权限不会自动成为服务访问权限。[auth API](https://docs.rs/p2panda-auth/latest/p2panda_auth/)

语言前提现已更新：用户明确新实现采用 Rust，现有 Go 代码仅供参考。因此先前“保留 Go 会引入 FFI/双语言运行时”的顾虑，不再作为新核心复用 p2panda 的默认成本；仍须评估其数据语义、接口与维护成本。

## 6. 当前建议

不把全部候选叠在一起。先对照两条路线：iroh 统一 QUIC 会话并补最小载体；libp2p 复用多 transport、打洞与 Relay v2，但明确流重建语义。

判别场景：T 经 R 访问 S 的长连接服务，建立直连、限制 UDP 速率、切换外层载体、断开旧路径。观察流是否继续、是否重连、身份是否仍为 T、S 的新 ACL 是否生效。先不把打洞、伪装、浏览器全部叠进同一次实验。

普通节点中继先比较单中间节点，不扩大成任意多跳路由。伪装优先集成已有实现并实测，不手写密码或伪 TCP 栈。p2panda 则独立按持久控制数据能节省多少工作判断。

这轮改变的是选型顺序：先验证完整连接需求与会话取舍，再决定集成栈；不从“p2panda 用 iroh”反推 miopunch 必须用 iroh。本文没有授权或启动实现。

## 7. 后续修正：允许 fork 及去中心化改造边界

### 7.1 用户前提与推荐修正

用户明确允许 fork 和修改 iroh/libp2p 等基础库，以适应大陆实际网络；需要的是明确改造范围，而不是仅比较能否原样使用。当前研究以 iroh 为底座的改造为主线，libp2p 保留为技术对照、协议和实现参考。本次仍不实施。

维护问题需要修正此前的评价：Shipyard 2026-08-24 公告称资金不续，将于 2026-09-30 结束相关工作，并停止向 go-libp2p/js-libp2p 贡献。它将 Kubo 等失去专职维护，与停止上游 libp2p 贡献分别列出，不能扩大成所有语言实现全部弃维；但近期版本发布也不能证明未来专职维护资源充足。[Shipyard 公告](https://ipshipyard.com/blog/2026-the-end-of-ipfs-at-shipyard/)

IPFS 8 月 25 日回应说明维护组织和资助方向调整，不是对全部实现提供同等人员与响应保证。因此撤回“仅凭现成功能，把 libp2p 作为优先采用方向”的建议。[IPFS 回应](https://blog.ipfs.tech/2026-08-beyond-sponsored-gateways/)

### 7.2 官方服务器、协助节点与协议不是一回事

iroh 常规流程通过 relay 建立初始端到端连接，借此协调 NAT 穿透，失败则继续中继。relay 转发加密流量，不是 miopunch 的成员或创世授权服务。[官方 NAT 说明](https://docs.iroh.computer/concepts/nat-traversal)、[relay 说明](https://docs.iroh.computer/concepts/relays)

但 relay 不必由官方运营。`N0` preset 配置官方 relay 与 DNS/Pkarr；`Minimal`/empty builder 可从没有这些服务的配置开始。注意 `N0DisableRelay` 仍保留官方地址发现，不能作为完整去官方化配置。自定义 `AddressLookup`、显式 `EndpointAddr` 是现有接入方式。[N0 API](https://docs.rs/iroh/latest/iroh/endpoint/presets/struct.N0.html)、[presets](https://docs.rs/iroh/latest/iroh/endpoint/presets/index.html)、[Builder](https://docs.rs/iroh/latest/iroh/endpoint/struct.Builder.html)

因此要分别解决：

1. 地址发现：如何把节点身份映射到当前可尝试的联系途径。
2. 外部地址观察：本机 UDP 套接字从对方看来是什么地址，候选何时过期。
3. 首次通信：尚未打洞成功时，两端怎样先交换加密协议流量。
4. 打洞协调与路径验证：已有通信基础上尝试直连。
5. 失败中继及选路：没有直连或直连质量差时怎样继续服务。

这些角色可以由不同普通节点或可选渠道承担，不要求同一台固定服务器。完全没有可达联系人、有效地址或任何投递路径时，协议也不能凭空建立联系；去中心化不是取消初始可达性的物理前提。

### 7.3 QNT 并没有硬编码只能通过官方 relay

固定 v1.2.0 的 `trigger_holepunching` 选择已有 client-side QUIC connection，读取远端 NAT 候选并发起穿透，没有检查该连接必须使用官方 relay 或 `RelayAddr`。与早期博客不同，当前 QNT 已在 QUIC 连接内运行。[remote_state.rs](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket/remote_map/remote_state.rs)

由此得到候选设计，而非实测结论：A、B 各自已有到普通 R 的连接；通过自定义数据报载体在 R 上转发 A-B 的端到端 QUIC 包；内层 QUIC 建立后，复用 QNT 尝试 UDP 直连并验证路径。R 不获得 A/B 的长期身份秘密，也不裁决业务授权。电路建立、转发资源、外层/内层拥塞及无递归路由仍需实现和验证。

MQTT 仅交换几个候选地址，不等于已提供 QNT 所需的 QUIC 连接。可以把它作为联系/提示渠道，但若完全没有初始包载体，需要补另一条建连途径；不能声称替换地址簿便完成 NAT 去中心化。

### 7.4 初步改造清单

| 范围 | 明确工作 | 当前判断 |
| --- | --- | --- |
| 默认公共服务 | 清除官方 relay、DNS/Pkarr 发布与查询；审计所有运行时出站 | 现有配置为主，无须为此重写内核 |
| 成员联系信息 | 接入成员地址簿、已有节点转交、邀请线索和可选 MQTT；管理新鲜度 | miopunch 的 AddressLookup/显式地址集成 |
| 普通节点中继 | 经已有邻居连接转发内层 QUIC 包，提供准入、限额和断开处理 | 优先 custom transport；接口缺口再补 iroh |
| NAT 地址来源 | 接入外部观察、映射更新和撤销；不把一次观察当永久全球地址 | 已有注入接口；探测编排可能需 iroh 补丁 |
| TCP/伪装 | 引入现成 TCP/FakeTCP 载体；定义 TCP 打洞协调与平台套接字生命周期 | 外部实现优先，必要时改套接字/探测接口 |
| 路径质量 | 比较 UDP 受限、TCP 和中继；加入切换迟滞、探测预算、失败回退 | 已有 PathSelector；指标或触发缺口再补丁 |
| hard NAT 策略 | 按实验确定需增加的探测、端口预测或重试行为 | 先定位 iroh/noq 归属，不能预设整套重写 |

已经找到一个具体的 iroh 内部耦合：`DirectAddrUpdateState::run` 在 `RelayMap` 为空时直接返回，后面的 `net_report` 与 `port_mapper.procure_mapping()` 都不执行。无专用 relay 时，地址观察和端口映射应如何独立工作，是有源码依据的补丁候选，而不是泛称“改掉中心化”。[socket.rs，v1.2.0](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket.rs#L738)

已知本机外部地址则可通过现有 `add_external_addr/remove_external_addr` 更新；自定义选路也已有 `path_selector`，无需直接更改 QUIC 算法。这些只是接入口，来源可信度、候选寿命、NAT 按目标变化、探测安全和具体策略仍由集成负责。[Endpoint API](https://docs.rs/iroh/latest/iroh/endpoint/struct.Endpoint.html)、[Builder API](https://docs.rs/iroh/latest/iroh/endpoint/struct.Builder.html)

另一个可选的小补丁是将 noq 已有的 `observed_external_addr` 观察流透出并汇入候选收集；iroh 已有发送/接收 observed-address report 的配置，但检查到的公共 Connection 面和常规候选链未透出/消费该流。也可先不改库：能直接收到 UDP 连接的普通 peer，在认证后通过应用消息报告所见源地址，再注入本机候选。必须观察同一个 UDP 套接字，不能拿 MQTT 的 TCP 源地址或 custom/relay 的合成地址充当 UDP 映射。[iroh 配置](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/endpoint/quic.rs#L411)、[noq 观察 API](https://docs.rs/noq/1.3.0/noq/struct.Connection.html#method.observed_external_addr)、[Incoming 地址](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/endpoint/connection.rs#L188)

### 7.5 fork 的维护边界与验证

建议保留上游身份认证、TLS/QUIC、路径验证及可复用的 QNT；成员/创世权限仍在 miopunch 层。允许在证据指向需要时改 iroh/noq，但不因采用 fork 就重写成熟安全机制。

每项补丁记录需求、固定上游版本、实际缺口、修改范围、协议兼容性、回归用例与是否可回馈上游。区分 iroh 补丁与底层 noq 补丁，锁定兼容版本；维护 fork 包括持续安全更新，不只是保存一次源码副本。

第一项判别实验应是：阻断官方服务，关闭默认 relay/lookup，让三节点通过普通 R 建立初始端到端 QUIC，补充地址观察，复用 QNT 建立直连，并验证 R 离线后的行为。另测冷启动线索、全部地址过期、UDP 限速、网络切换以及端到端身份。抓包验证不能只看“直连成功”，还要证明没有暗中回退官方发现或 relay。

现阶段尚未构建此实验，没有证明该组合已能工作，也没有因讨论而创建 fork 或修改运行代码。

### 7.6 最新修正：不预设 R 与原有流程，采用 Rust 新实现

用户指出联络可能依靠 MQTT 或其他渠道，并不一定有可转发内层 QUIC 的节点 R。各渠道的推送/轮询、延迟、容量和消息可靠性可能不同；共同的是建连协商语义，不是统一模拟一条数据报管道。

因此第 7.3 和 7.5 的 R/QNT 实验只是一个可选策略，不能证明全部渠道需求已满足。需要同时讨论“没有端到端 QUIC 时，通过外部消息渠道协调直连尝试”的流程，必要时重排 iroh 的建连编排及探测/传输衔接，而非预设只能小补丁扩展。业务路径与联络渠道分开，直接连接失败时，只有确实具备中继能力的路径才可承担回退。

用户已明确采用 Rust 全新开发，现有 Go 实现仅作参考。新设计不以兼容旧代码结构、旧接口或逐文件翻译为前提；现有实验和失败案例仍可用于验证设计，但是否沿用具体行为需重新判断。尚未开始创建工程或实现。

### 7.7 最新修正：完整连接流程封装在 fork 内部

用户希望直接改造 iroh，并尽量保留其对外接口、仅增加必要接口；p2panda 尽量保持原样，需要时适配。此前将“miopunch 拥有完整连接流程”解释为上层必须自行编排，再调用 iroh，偏离了用户希望的封装方式。

当前候选设计：改造后的 iroh 连接库内部负责多渠道信令、NAT 信息交换、探测/打洞、连接建立、传输与路径管理。上层仍主要面对 Endpoint/Connection 等接口，不需要自行执行一套 gather/offer/answer/punch 步骤。MQTT 是库内可选渠道适配，不是写死的唯一 broker，更不要求把它当作 QUIC 数据路径。

当前代码作为参考的具体依据：

- [punch/service.go](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/service.go)：Dial 采集 UDP 快照、构造 offer、交换 answer，再执行 runPunch。
- [punch/exchange.go](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/exchange.go)：消息签名、关联、验证，以及接收侧采集快照、决定 UDP 打洞行为、发送 answer。
- [punch/runtime.go](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/punch/runtime.go)：执行尝试、保留 UDP 所有权信息，返回 PathResult 与诊断证据。

这些说明当前 UDP 建连已有完整主流程，值得在 Rust/fork 设计中继承其经验，不等于旧代码或消息格式必须照搬，也不证明新多渠道/TCP/伪装/会话切换要求已覆盖。

对外兼容应同时检查调用方式、身份语义、异步生命周期、取消/超时和 stream 行为，不只是函数名不变。新增接口主要用于注册渠道、输入配置和策略、观察状态；具体是否需要 p2panda-net 小型适配尚未验证。

fork 内部可以细分模块或独立 crate；对外封装成完整连接能力不妨碍内部职责清楚。它不应写死创世权限、入网审批或服务 ACL 规则。接口兼容不自动保证新增信令流程与未修改的上游节点互通，这需要另定兼容范围。

下一轮应对照“现有流程如何在 fork 内部落位”，而非先另造上层连接编排框架。尚未启动 fork 或实现。
