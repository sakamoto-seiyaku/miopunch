# 已确认场景与实现职责对照

日期：2026-09-28。状态：源码研究与候选分工，不是架构定稿或实施授权。

本轮按用户要求，由三路 GPT-6 Sol / xhigh 子代理分别核查 iroh、p2panda 和集成风险，主代理交叉阅读关键源码并综合。基线为 iroh 1.2.0、p2panda 0.7.1；版本范围、源码事实和待验证推论分开。没有编译候选组合、运行网络实验或修改运行代码。

产品要求以[主线草案](2026-09-27-product-mainline-draft.md)和[场景推演](2026-09-28-network-scenario-walkthrough.md)为准。ACL 暂停，不新增两套访问配置、通用业务恢复或运行时插件平台。

## 1. 当前建议

**继续以 iroh 深度 fork 为连接实现，优先保留 p2panda 已有的地址簿、发现、gossip、存储与日志同步；通过少量明确的接入修改把两者组合起来。** 不因构造入口不够灵活就重写整个 p2panda-net，也不把“两个 Rust 库都有了”误当成全部场景已经实现。

这一建议尚需下文的小实验验证。若某个高层入口确实不适配，先下沉到已有的模块级接口，不立即退回只用数据格式、其余全写。

更具体的验证起点是模块化的 `p2panda-net` 加 `core/store/stream/sync`，而不是直接把发布版高层 `p2panda::Node` 当成完整产品控制面。高层 Node 虽省样板，却固定组合网络模块，且尚未接入本产品需要的成员准入；模块级复用仍应接上现成 Ingest 流水线，不能省略验证和持久提交。[高层组合](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/src/network.rs#L36)、[高层依赖](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/Cargo.toml)

真正需要自己承担的工作集中在：iroh 建连前的消息协商、普通节点中继与新载体、资源硬约束，以及产品成员证明与引导发现的结合。现有 Go 打洞流程和实验经验仍是 Rust 实现的参考，不要求照搬旧模块。

“薄适配”只描述构造与调用接入，不表示成员证明或建连前信令保护已经由几行配置解决。特别是前置信令需要选择成熟安全机制并验证组合，不能把记录签名、topic 隔离或传输 TLS 当作已经满足全部要求。

## 2. 统一场景索引

| 编号 | 已确认行为 | 研究中必须回答的问题 |
| --- | --- | --- |
| S0 | 老成员利用成功线索恢复，必要时扩展寻址、旁路和长期引导 | 谁保存联系人，谁选择尝试对象，谁执行单次端点建连 |
| S1 | 双方在 NAT 后，经 MQTT/IRC 等交换受保护的打洞信令，再建立安全数据连接 | 不能先要求 QUIC/relay 已连通；协商进入哪个现有内部流程 |
| S2 | A-B-C 部分互联，B 保存 C 的记录，C 离线后 A 仍能补齐 | 哪些存储、同步、发现与拓扑维护可直接复用 |
| S3 | A-C 承载路径失效时尽力保留同一连接，最终无法维持就报错 | 新载体是否加入原连接，而不是另拨一条冒充恢复 |
| S4 | A 不认识新成员 C，旧联系人离线，合法 C 仍可成为首个恢复对象 | 候选发现、端点身份与成员证明怎样组合，不能先依赖全量同步 |
| S5 | 官方 relay 可禁，自建和扩展可用；质量优先且遵守资源许可 | 禁用是不是实际约束，而不只是修改默认地址或优先级 |

这里 S0-S5 是研究索引，不是新增线协议或永久产品术语。

## 3. 已有能力与省掉的工作

### iroh

保留端点身份认证、端到端 QUIC 连接与 stream、现有 relay 收发、已知 EndpointId 寻址、连接路径状态及路径变化通知。这些已有能力使我们无需自己重写传输加密、可靠流、多路复用和整套连接基础设施。路径列表对应同一条 Connection，而不是仅保存一张可供重新拨号的地址表。[Connection API](https://docs.rs/iroh/1.2.0/iroh/endpoint/struct.Connection.html#method.paths)、[Address Lookup](https://docs.iroh.computer/concepts/address-lookup)

iroh 也已有自定义 transport 和 PathSelector 接口，不能把每项新路径策略都写成从零改造。但 1.2.0 中这些接口受 `unstable-custom-transports` feature 控制，不承诺语义化版本兼容；路径选择扩展点也不自动解决资源禁用或建连前协商。[Builder 扩展点](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/endpoint.rs#L814)

### p2panda

发布版高层 Network 已经组合 AddressBook、Endpoint、mDNS、Discovery、Gossip 与 LogSync，而不是只有日志格式。优先复用这些组合，能避免再次实现联系人存储、已有成员上的发现、传播拓扑、同步会话及缺失记录补齐。各模块是否完全符合我们的行为，还须按场景验证。[发布版组合入口](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/src/network.rs#L36)

core/store/stream/sync 的价值还包括签名操作、存储与处理流水线。原作者和提供副本的对端必须继续区分；实际领域操作的权限含义、处理幂等和正文可靠保存不能因为引入库就省略。详细边界见[p2panda 分报告](2026-09-28-p2panda-behavior-gap-research.md)。

其中低层 LogSync 收到记录后产生 `OperationReceived`，不能单独视为已经持久化；需要接入现成摄取流水线。高层提供的 `Acked` 跟踪应用消费游标，也不是网络送达确认。因此 S2 可以复用完整记录/存储/摄取/同步组合，而不是只启动一个同步协议就宣称离线补齐已经可靠。[接收事件](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-sync/src/protocols/log_sync.rs#L308)、[消费游标](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda/src/streams/acked.rs#L21)

## 4. 已定位的接入缺口

### 4.1 p2panda 构造入口需要适配，不需要另建连接框架

p2panda-net 0.7.1 的 Endpoint Builder 自建 iroh Endpoint；公开 IrohConfig 只包含 IPv4/IPv6 绑定地址和端口，没有传入我们的信令、custom transport 或 path selector 配置的入口。[Builder](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/builder.rs)、[IrohConfig](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/config.rs)

**候选修改：** 统一 fork 依赖，给既有构造流程增加必要的配置或工厂入口；保持使用同一个 Endpoint 和原有 accept 分发。需验证初次创建及现有生命周期中的重建都能获得一致配置，而不是建好以后临时修改一次。

依赖中的 `iroh = "1.0.3"` 不是精确锁死 1.0.3；满足版本范围并不等于已经证明 1.2.0/fork 的编译及行为兼容。这应由依赖树和小型接入实验核实，不作为必须重写 net 的理由。[依赖声明](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/Cargo.toml)

### 4.2 地址尚未知时，不能在进入 fork 前提前失败

现有 p2panda wrapper 的 Connect 先从地址簿取得可用 EndpointAddr，缺少时返回 `TransportInfoMissing`，然后才调用 iroh。这会阻断我们希望 fork 提供的“只知道身份，也能经其他寻址或信令途径继续尝试”。[Connect 分支](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L244)

**候选修改：** 保留现有地址作为线索，但不要把地址簿缺失直接等同于无法建连；允许请求到达有能力继续寻址的 fork。具体接口、超时与错误转换须验证，不另造上层打洞编排器。

### 4.3 同步 topic 和 NetworkId 不等于成员准入

发布版同步 handler 从连接取得对端身份，再完成 topic handshake，并路由到本地已订阅的 topic manager；这段路径不能作为已经实现本产品成员证明验证的证据。[同步 handler](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/sync/actors/manager.rs#L517)、[topic 路由](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/sync/actors/manager.rs#L406)

S4 应复用现有发现能力和端点安全连接，同时补齐产品成员验证的接入。Endpoint 已支持注册自定义 ALPN handler，可作为在同一 Endpoint 上接入受限引导交互的基础；这不是现成成员证明协议，也不是建议自造密码握手。[Endpoint::accept](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/api.rs#L129)

不能反过来说 p2panda 完全不会发现未知节点：其 discovery 可以通过可达 bootstrap 的发现会话取得新的节点线索。缺口在于 MQTT 等消息入口不能直接充当这种 QUIC discovery 会话的 seed，以及取得候选后仍没有本产品的成员证明取得/准入组合。两种发现入口需要衔接，而不是把现成发现全部丢掉。[Discovery 会话](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L96)

还需看实际默认值而非“PSI”名称：低层 `share_nodes_with_common_topics` 默认 false，此时会遍历地址簿全部节点，导出有可传播 transport 信息的条目；net 会话固定调用默认构造器。因此 topic 集合匹配不等于只向同私网成员共享联系线索。低层可设置这一选项，但发布 net 的 DiscoveryConfig 尚未把它暴露出来，需要薄适配或更明确的准入/共享策略。这里记录现有行为，不新增成员隐藏或“最小通讯录”的产品要求。[默认值与分支](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-discovery/src/psi_hash.rs#L48)、[net 构造](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L123)

p2panda NetworkId 参与协议标识隔离，不应当作秘密或成员资格。产品私网 ID 是否与它一一对应尚未决定；公共引导的兼容性取决于该映射，不能直接推出每个私网都必须另部署服务器。[NetworkId 说明](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/lib.rs#L285)

节点公钥的互通已有基础：p2panda NodeId 与 iroh EndpointId 可通过现有工具按同一 Ed25519 公钥字节转换，未必需要维护两套独立节点身份。但节点密钥相同不等于获得成员资格，也不把创世权限凭证合并成日常 Endpoint 私钥。[身份转换](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/utils.rs#L116)

### 4.4 RelayMap 不是远端 relay 白名单

iroh 1.2.0 的 RelaySender 地址检查直接返回 true；relay actor 对未建立的目标 relay URL 可创建新的连接，RelayMap 查询用于获取该 URL 的认证配置。因此只配置自建 RelayMap，不能保证拒绝显式地址、缓存或其他查询来源带来的官方 relay URL。[地址检查](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket/transports/relay.rs#L282)、[创建 relay 连接](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket/transports/relay/actor.rs#L1321)

**候选修改：** 在候选接入及实际连接/发送位置落实统一资源约束。全部关闭原生 relay 是已有的简单选项；保留自建而禁用指定公共资源，需要更细的约束。若未来支持运行时修改，还需处理活动 actor 和既有路径；热修改不是本轮已经确认的必做项。

### 4.5 新协商与新载体要进入同一连接生命周期

消息型旁路不是 QUIC 数据 transport；只给 AddressLookup 添加 MQTT 地址来源，也不能表达双方准备、协调打洞与重复尝试。普通节点经已有 peer 连接代转数据，同样不等于部署现成 iroh-relay。两者都需要在 fork 内部接入相应的协商或路径机制。具体源码落点、可复用扩展点与限制见[iroh 分报告](2026-09-28-iroh-behavior-gap-research.md)。

此外，固定基线在 RelayMap 为空时会跳过地址 net report 和后续端口映射。因此无原生 relay 的路线还需把地址观察与 relay 配置解耦，并确保打洞观察、探测和实际 QUIC 使用同一合适 socket，不能只关掉 relay 再外挂一次 STUN 查询。[地址更新流程](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket.rs#L778)

更换 PathSelector 也不是全部：现有“路径已经足够好”的探测判断偏向 IP，网络变化恢复提示区分 relay 与 custom。新载体需进入这些内部状态判断，避免虽然可以选中，却仍按不合适的规则探测或放弃。[探测判断](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket/remote_map/remote_state.rs#L751)、[网络变化](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh/src/socket.rs#L1697)

S3 的验证重点是原 Connection 和 stream 是否继续工作，而不是路径失败后能否新建一条连接。TCP/其他载体的可行性、拥塞与恢复表现仍需实验，不能由接口存在直接推出效果。

## 5. 建议的职责归属

| 职责 | 候选归属 | 不应重复建设的部分 |
| --- | --- | --- |
| 本网身份、创世权限、成员证明含义、资源选择 | 产品模块，调用适当的现有授权能力 | 不写死进通用连接库；不把 topic/ALPN 当成员资格 |
| 可保存的记录、联系人、传播与补齐 | 优先复用 p2panda 相应模块 | 不再并行自建一套 EBT 和日志数据库 |
| 为发现/同步选择哪些 peer、订阅什么数据 | p2panda/产品网络模块 | 不让每个业务各自维护整个成员拓扑 |
| 怎样到达指定 EndpointId、如何协商和打洞 | iroh fork 内部 | 上层不逐步调用 gather/offer/punch |
| 实际路径、质量选择、同一连接恢复 | iroh fork/noq 已有机制加必要扩展 | 不在上层伪造通用业务续接 |

p2panda 的“换一个同步对象或重建失败同步会话”，与 iroh 的“为同一端点建立/维护路径”不天然重复。需要对齐的是取消、尝试期限与错误分类，避免上层提前超时取消下层仍有价值的尝试。详细风险见[集成审查](2026-09-28-integration-responsibility-audit.md)。

## 6. 候选验证实验

以下是可供后续设计决策选择的实验，不是自动进入的下一阶段，不是本轮已经运行的测试，也不代表已获实现授权。用户随后指出整体设计仍有未闭环事项；当前先整理这些事项及依赖。每项实验应先明确要回答的问题、前提和判定标准，再决定是否开展，不能用跑通代码替代产品与信任模型的决定。

1. **接入基线：** 统一 iroh fork 与 p2panda 的实际依赖版本，使用同一个 Endpoint 跑既有发现/同步；确认新配置能传入，缺少缓存地址时请求仍能进入 fork。
2. **第三方持久补齐：** C 发布，B 保存并重启，C 离线，A 从 B 取得完整记录与正文；验证原作者身份和重复交付行为，不只观察 gossip 到达。
3. **未知成员恢复：** A 本地没有 C，旧联系人全部不可用；经引导发现 C，取得必要证明后正常同步；无效候选不能因此获得正常同步权限。同步全量状态不能成为发现/验证前提。
4. **同一连接换路：** A-C 经 B 传递有序数据，切断 B，切至可用直连或 D 路径；检查同一个 Connection/stream 的延续。恢复失败时确认错误明确，而非静默创建新连接。
5. **资源约束：** 保留自建 relay，分别经显式地址、旧缓存和查询结果注入被禁 relay URL；观察实际连接/发送行为，证明禁用有效，而不仅是配置显示正确。
6. **旁路打洞：** 双 NAT 节点不依赖初始 relay 数据路径，经消息渠道完成受保护协商，再形成可用连接。协调使用的 socket/地址观察必须与实际传输相符。

这些实验分别验证复用边界，不能以一次直连 demo 或一次 Cargo 编译通过代替全部场景，也暂不扩展为全平台性能工程。

## 7. 分报告

- [iroh 行为覆盖与改造缺口](2026-09-28-iroh-behavior-gap-research.md)
- [p2panda 行为覆盖与复用范围](2026-09-28-p2panda-behavior-gap-research.md)
- [集成职责与风险审查](2026-09-28-integration-responsibility-audit.md)

库本身的维护与版本风险、源码接口存在、以及真实环境效果是不同证据层级。当前结论支持继续验证这条复用路线，不是对尚未实现组合的稳定性承诺。
