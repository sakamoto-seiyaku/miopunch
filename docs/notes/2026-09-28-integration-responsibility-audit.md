# iroh + p2panda 集成职责独立审查

日期：2026-09-28。状态：临时架构审查，不是实现规格或测试报告。本轮未修改、构建或运行候选库。以[产品主线](2026-09-27-product-mainline-draft.md)和[场景 S0-S4](2026-09-28-network-scenario-walkthrough.md)及其资源约束 S5 为需求基线；旧研究仅作线索。iroh 核查固定在 v1.2.0、提交 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`；p2panda 核查发布版 v0.7.1（[Cargo.toml](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/Cargo.toml#L1)）。以下分别注明源码事实、推论和待验证项。

## 按严重度排序的集成风险

这里的 P0/P1 表示兑现本产品场景时的设计与集成优先级，不是对上游项目的漏洞定级或线上事故结论。

### P0：S4 的未知成员恢复仍缺少一条无循环的启动链

**事实。** iroh `AddressLookupServices::resolve` 的输入是已知 `EndpointId`，而 `RemotePathState::resolve_remote` 只保证存在潜在地址，并不验证可达性（[iroh address_lookup.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/address_lookup.rs#L544)、[path_state.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state/path_state.rs#L161)）。p2panda Discovery 的 walker 从地址簿中的 bootstrap 候选起步，再建立 QUIC discovery session；它不是在所有已知 bootstrap 都离线时凭空找出本地从未记录的 C（[walker.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/walker.rs#L164)、[session.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L80)）。

**后果与最小修正。** 若 A 的旧联系人 B 离线、只有新成员 C 在线，把“先连接成员才能同步成员名单”和“先同步名单才能知道连接谁”接在一起，S4 必然卡住。产品已有可用入口这一前提下，仍需明确入口如何给出**未知 EndpointId 的候选**，以及 A 如何有界取得、验证 C 与该 EndpointId 绑定的成员依据。入口只给线索；A 保留的本网信任锚/已验证管理依据、C 提供的身份及必要授权依据及其依赖是准入依据。证明可以在受限的端点握手或连接阶段取得，具体先后待选型；不能强制证明先于 QUIC，也不能为了取证明先开放普通记录同步。撤销信息未送达的离线窗口按产品共识处理，不把入口升级为发证者。这里没有预设“最小通讯录”或某个目录协议。

### P0：S5 的资源禁用不能只写成 relay 配置或路径评分

**事实。** iroh 的 `remove_relay` 更新 `RelayMap`；`RelaySender::is_valid_send_addr` 当前直接返回 `true`；对陌生远端 relay URL，relay actor 可以新建 `ActiveRelayActor`，`RelayMap.get` 在该分支主要用于取 auth token（[socket.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L1244)、[relay.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay.rs#L280)、[actor.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs#L1321)）。`N0` preset 另安装官方 DNS/Pkarr 查找与发布，和 relay 使用是不同作用域（[presets.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/presets.rs#L116)）。p2panda wrapper 自建 iroh Endpoint，向它传入自己的 relay URL 集（[p2panda builder.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/builder.rs#L66)、[endpoint.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L176)）。

**后果与最小修正。** 只把 home relay 改成自建或只在质量选择器里排除官方路径，不能保证从显式地址、p2panda 地址簿、iroh lookup、已缓存远端路径或故障回退中来的官方 URL 不被拨通。资源资格应在候选进入、实际拨号/发送和恢复重试时共同生效；若未来支持运行时禁用，还需明确已活跃路径的撤销结果。运行时热修改尚未被确认为必做能力。官方 relay、官方地址查询/发布、QAD/其他观察源、消息入口分别配置其权限，不能声称关掉其中一项就关闭全部官方访问。普通成员的**信令转交默认允许**与**业务数据中继显式启用**也必须在候选资格处区分；质量比较只在获准集合内进行。这是由源码与 S5 推导的修正位置，并非已实现的开关。

### P1：p2panda wrapper 可能让 fork 的新寻址和前置协商根本无法启动

**事实。** v0.7.1 `p2panda-net` 的 `Endpoint::connect` 先向地址簿索取 `NodeInfo`，缺记录或无法转为 iroh `EndpointAddr` 就返回 `TransportInfoMissing`；随后才调用 iroh `connect_with_opts`（[endpoint.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L244)、[connection.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/connection.rs#L97)）。它的 `AddressBookDiscovery::resolve` 本身也按给定 EndpointId 查同一本地址簿（[discovery.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/discovery.rs#L115)）。

**推论与最小修正。** 对已知目标却没有可用地址簿条目的 S0/S1，fork 内即使能通过消息入口寻找、协商和打洞，也收不到调用。先做薄接线：为目标 EndpointId/当次线索建立一条能进入 fork 的 connect 路径，并保留 p2panda 对身份、ALPN 与连接结果的既有用法。可选做法包括把经验证的候选投影到地址簿，或在 wrapper 缺地址时把身份交给 fork 自行恢复；取舍取决于候选是否应持久化、缓存是否过期。无需因此整体重写 p2panda net，也不能让两处各自维持一份互不回流的地址真相。p2panda v0.7.1 builder 目前以地址簿建 Endpoint，未见注入既有 Endpoint 参数；扩展启动配置属于薄适配候选（[builder.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/builder.rs#L14)）。

### P1：应区分全网选人与单端点路径维护的重试职责

**事实。** p2panda Discovery walker 选择下一节点并按发现成效退避，Discovery session 对选定节点发起 QUIC（[walker.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/walker.rs#L140)、[session.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L80)）。LogSync 的 TopicManager 对失败的同步会话五秒后再试，同步 session 会重新 `Endpoint::connect`（[topic_manager.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/sync/actors/topic_manager.rs#L405)、[session.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/sync/actors/session.rs#L81)）。iroh `RemoteStateActor` 则为同一 EndpointId 管理多个 QUIC connection 的路径、QNT、选路与恢复（[remote_state.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L385)）。

**推论与最小修正。** 这两层都“重试”并不天然重复：前者是下一位成员/下一次同步，后者是指定 EndpointId 的一次连接尝试及存活连接的路径恢复。真正冲突发生在产品再加一套 per-peer gather/offer/punch，或 p2panda 反复新拨号覆盖仍在 fork 内恢复的同一意图。需明确一条 attempt 的 id、取消、超时/预算和结果回报：产品或复用的 p2panda net 选择**与哪些成员**尝试连接/同步、何时增强 A-C；fork 独占**怎样联系该 EndpointId、协商、建立及维护路径**。失败要区分候选不可达、连接终止、同步协议失败；不能把一次拨号失败直接解释为该成员失效。S2 的部分互联不要求 fullmesh 或每对常驻保活。

### P1：S2 的“收到记录”与“有可供第三方离线补齐的副本”仍差一个明确提交点

**事实。** p2panda `LogSync` 按 topic 查得各作者日志，并从本地存储发送其持有的操作，不限于本节点作者（[topic_log_sync.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-sync/src/protocols/topic_log_sync.rs#L128)、[log_sync.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-sync/src/protocols/log_sync.rs#L360)）。底层协议收到 header/body 时向应用发送 `OperationReceived`，这一步本身不写入应用存储（[log_sync.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-sync/src/protocols/log_sync.rs#L308)）。高层 p2panda 的 processor pipeline 另接 ingest；该处理器验证操作并事务写入，SQLite 保存正文（[pipeline.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/src/processor/pipeline.rs#L105)、[ingest](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-stream/src/ingest/operation.rs#L38)、[SQLite store](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-store/src/operations/sqlite.rs#L35)）。

**后果与最小修正。** 若为减少 net 适配只取 core/store/sync，不能把同步事件或“B 收到了 C 的记录”当成 B 已验证并持久保存了正文及验证依据。S2 应由记录/成员视图的产品层定义：验原作者签名、检查该记录所需授权依据、持久提交 header+body+依赖，再宣布可由 B 供 A 补齐。发布版提供可复用机制，但不替 miopunch 决定哪些管理记录有效、哪些副本该保留。若直接采用高层 pipeline，仍需核对它的默认 ingest 与产品验证顺序，而不是并行造第二套记录状态机。

### P1：S3 的普通 peer circuit 与原生 relay 不是同一种已实现路径

**事实。** iroh 原生 relay actor 按 relay URL 建专用服务器连接；源码没有“借 B 的 A-B 安全连接自动转发 A-C 内层包”的普通节点 circuit（[relay actor.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs#L1321)）。已有同一 Connection 多路径的底座：远端 actor 可在已建连接上打开选中路径、将其它路径设为 backup（[remote_state.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L675)）。默认 selector 把 IP/custom 放在 primary、relay 放 backup，先比较层级再比较 RTT（[biased_rtt_path_selector.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs#L78)）；网络变化时 custom 当前被视为不可恢复（[socket.rs](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L1715)）。

**后果与最小修正。** 不能把 p2panda 的记录传播、B 的信令转交或重新拨一条 A-C Connection 算成 S3 的原连接连续性。若普通成员明确启用数据中继，需在 fork 内把其受控转发形成可验证的 A-C QUIC 路径，保持 A-C 端到端保护，并验证 B 断开后由直连或 D 路径承载**同一个** Connection/已有 stream；新载体的拥塞、MTU、排队及路径恢复能力属待实验。没有及时恢复则让旧 Connection 关闭并报错，上层业务的 resume 不在此职责内。质量优先还要求修正现有 direct/custom 必胜的默认策略，但路径选择不能替代 S5 的硬资格过滤。

### P1：S1 的前置私密信令与成员准入不是同一把网络密钥

**事实。** p2panda v0.7.1 `NetworkId` 只混入 protocol id/ALPN，源码明确它不是秘密（[lib.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/lib.rs#L285)）。地址簿的签名 transport info 可验证是谁发布该地址，但字段只描述 node id、transport 和本地连接指标，不携带 miopunch 成员授权结论（[addrs.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/addrs.rs#L42)）。p2panda Discovery 的 topic PSI 在已建立 QUIC 的 session 内运行（[session.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L80)）。

**推论与最小修正。** A、B 各自接入 MQTT/IRC 后，前置协商仍须以成熟的端到端机制保护，且其目标身份必须与后来 QUIC 验证的 EndpointId 对上；broker/topic、`NetworkId`、邀请中的共用秘密都不能替代每对端点的身份认证与私密性。S4 未知 C 时，先通过受限发现取得候选身份/验证材料，或由 C 向已知 A 发可验证的受保护首条消息；具体顺序依赖所选成熟机制。不要假设 A 在发现前已经持有 C 的公钥，也不要把全网 PSK 当成默认解法。产品的成员、根凭证恢复和指定节点 precode 核销继续独立于连接库；本轮不展开 ACL。

### 补充：topic 匹配不等于默认地址线索只在同 topic 内传播

主代理交叉核查发现，发布版 `PsiHashDiscoveryProtocol::new()` 使用的 `share_nodes_with_common_topics` 默认为 false，`gather_transport_infos` 在该分支遍历本地所有节点，输出有可导出 transport 信息的条目。net discovery session 调用这一默认构造器，公开 DiscoveryConfig 只暴露 walker 数量和重置概率。因此没有共同 topic 的 bootstrap 也可能提供未知 C 的联系线索，前提是它已有该信息；同时不能把 PSI 名称当成已经实现私网成员隔离。低层有配置接口，向 net 下传是可研究的薄适配。共享范围是否需要进一步限制仍由产品决定，不在这里重新施加最小通讯录限制。[低层默认与分支](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-discovery/src/psi_hash.rs#L48)、[net 使用方式](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L123)、[DiscoveryConfig](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/config.rs)

## 候选职责图与状态流

```text
产品成员/记录视图（持久、可验证；p2panda core/store/sync 候选）
  ├─ 成员依据、记录正文与依赖；决定准入及授权范围
  ├─ 选择需要发现、连接或同步的成员与副本覆盖（whole-network）
  └─ 投影获准的候选身份/联系线索 → p2panda 地址簿或 fork 输入
                         ↓                       ↑ 结果/新线索
p2panda net 可复用的发现/同步调度 ─── 薄接线 ─── 改造的 iroh Endpoint
  地址簿：可持久的 transport 声明、bootstrap、尝试指标     每 EndpointId：联络、协商、打洞、
  不是成员数据库或当前 live path                        QUIC、实际路径、质量、故障恢复
                         ↓                       ↓
                p2panda sync 在获准连接上交换记录；验证/提交回到持久视图
```

这是候选分工，不要求 p2panda net 全部启用。若用其高层 `Network::spawn`，它会一起启动地址簿、Endpoint、mDNS、Discovery、Gossip 和 LogSync（[network.rs](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/src/network.rs#L36)）；因此先选一个 whole-network 调度所有者，再决定保留哪些现成 actor。地址簿里的签名地址是可复用线索，近期成功/失败是本地观测，fork 的 live path 是当前连接状态；三者要回流，但不能互相冒充。产品可保留完整的获授权网络状态，不受“最小通讯录”限制。

## 可删掉的重复工作

- 不另造 EBT 或并行运行第二套管理记录同步栈；先验证 p2panda 的 core/store/sync 与现成 ingest 能否完成 S2。
- 不在产品层再写一遍每 peer 的 gather/offer/punch、路径切换和故障重拨状态机；外部仍以 `Endpoint`/`Connection` 为主。
- 不把 p2panda 的 Discovery walker、产品后台扫描与 fork 的 peer actor 三处都配置为独立的“连接全网”循环。保留一个全网选人者；Discovery 与同步会话重试可保留其各自目的。
- 不为 MQTT、已有成员转交、原生 relay 分别复制一套身份、尝试与结果数据库；它们作为联络/承载能力进入同一 per-peer 尝试。
- 不因普通节点能转交消息就自动建数据 circuit；不叠加两套服务 ACL 配置，也不建设通用运行时插件平台。

## 最小可验证组合与高价值 spike（均未执行）

**起点组合。** 固定 iroh fork + p2panda 0.7.1 的 core/store/sync，按需保留 net 的地址簿、Endpoint wrapper 与 Discovery/LogSync；产品只提供成员依据验证、获准资源和全网连接意图。先用单一端点身份、受控 bootstrap 和两个端点跑通一次 S1/S2，再决定高层 `Node` 是否比模块组合更省适配。`iroh = "1.0.3"` 在 p2panda-net Cargo 声明中是兼容版本范围，实际 fork API 与依赖解析仍须编译确认（[Cargo.toml](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/Cargo.toml#L75)）。产品私网标识是否映射到 p2panda `NetworkId`、topic 或别的成员视图，尚未决定；不能由 `NetworkId` 的名字推断私网准入或要求每网一台 bootstrap。

1. **S0/S1 接线实验：** A、B 只有消息入口且地址簿无可用远端地址，双方主动接入，单次 `connect` 内完成受保护协商、直连 QUIC 与正常 `accept`；并测双向同时发起、取消和失败退避。记录 p2panda wrapper 是否早退、同一意图出现多少次真实拨号。
2. **S4 启动实验：** A 只知离线 B，C 是唯一在线且未列入 A 名单；获准入口返 C 候选，A 通过受限引导或连接有界取得/验证 C 的成员依据并核对 EndpointId，满足准入条件后进入正常补齐；此实验不固定证明与安全握手的先后顺序。加入伪造候选、无效证明、过期线索与缺依赖，确认入口不能授予成员资格，也不需先全量同步。
3. **S2 副本提交实验：** C 发签名记录，B 接收、验证 header/body/依赖并落盘后 C 离线；A 仅连 B 取得同一原作者记录。B 重启、重复/乱序与“仅收到事件未提交”要能区分；不要求 A-C 常驻连接。
4. **S3/S5 路径与禁用实验：** A-C 经获准 B 数据 circuit 建立业务 stream，B 故障后经 D 或直连继续同一 Connection；超时后明确失败。将官方 relay URL 分别置于显式地址、p2panda 地址簿、iroh lookup 和回退结果中，在启动前配置禁用后验证无拨号/发送；若未来支持运行时禁用，再增加旧 live path 撤销用例；同时检查自建 relay 可用、仅信令 B 不转业务、获准 relay 质量优于直连时可被选中。

以上是设计验证目标，不表示 iroh 原生支持普通 peer circuit，也不表示 p2panda 0.7.1 已提供 miopunch 成员证明、前置消息加密或全网拓扑策略。正式实现前还需确定受限 proof 交换的成熟机制、fork 对同会话新载体的实际能力；若决定支持配置热修改，再明确资源资格更新语义。
