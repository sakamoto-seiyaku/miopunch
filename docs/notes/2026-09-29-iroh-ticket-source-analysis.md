# iroh 拆票前源码接入核查

日期：2026-09-29。本文是实施切片的证据和技术指导，不是新产品决议，也不表示功能已通过运行验证。行为基线为 Plane 的[《miopunch 新基线实施规格：身份、入网、连接、状态与迁移》](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)及 [ADR 0005](../adr/0005-side-channel-control-bootstrap.md)。按该决议复用 iroh 握手，不能退回早期报告中“必须另造前 QUIC 私密握手”的建议。

**结论：可以复用真实 iroh 握手、custom 包载体、QNT、多路径和公开流接口。最先需要解决的不是 MQTT 能否递送字节，而是一个仅允许控制的载体怎样不成为业务主路径或备用路径，以及准入结果怎样控制自动候选发布、探测和应用协议启动。** 现成 selector 和 `after_handshake` 都不足以单独保证这些性质。先验证下文 H1 的 iroh 内部阶段隔离；只有证明需要在同一连接上同时保留控制载体和业务路径时，再进入 H2 的 noq 调度扩展。不要直接开一张“完整 fork iroh”的实施票。

证据标签：**F** 为本次直接读取的固定源码/API 事实；**I** 为由事实推导的接入建议；**V** 为尚需编译、真实握手或故障试验裁决的假说。本次没有运行原型、构建正式 fork、修改运行时或提交代码。下文测试均为拟议验收。

## 1. 证据版本及引用规则

- 本次核对 `/tmp/miopunch-iroh-v1.2.0`：origin 为 `https://github.com/n0-computer/iroh.git`，HEAD 为 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`，工作树干净。本文 iroh 链接均固定该提交。`/tmp/iroh-1.2.0` 不是 Git checkout，不能由它单独声称 origin/commit 已验证。
- 该提交的 [Cargo.lock](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/Cargo.lock#L3114)锁定 `noq`、`noq-proto` 1.3.0。另读取官方发布的 `noq-proto-1.3.0.crate` 到内存，其 SHA256 为 `7c1e5b6fe668491eca022f745a0a9402585626c73a7b839b3424ace15d6a9c8f`，与锁文件一致；包内 `.cargo_vcs_info.json` 指向 `c1f411562e6078852749b8bcf1190523096a107f`，manifest 的 repository 为 `https://github.com/n0-computer/noq`。本文 noq 源码链接固定该提交。
- 未以 `main` 或 `latest` 的后来能力替代上述证据版本。版本只是研究基线，最终依赖组合仍须编译核查；外部 Rust 调用形状近似兼容，不等于与任意上游版本保持线协议互通。
- 源文件/行号属于本报告的证据索引；后续 Plane 票应引用报告并描述行为、接口责任和验收，不把冻结的私有函数或文件布局写成验收契约。

## 2. 从 connect/accept 到路径升级的实际调用链

**F — 发起端。** `Endpoint::connect_with_opts` 先调用 `before_connect`，随后 `resolve_remote`，建立绑定目标 EndpointId 的 TLS client config，再调用 noq `connect_with`。`resolve_remote` 只需一个可能地址就能完成；它不证明入口已经可达。未知地址可由 `AddressLookup::resolve(EndpointId)`补入，custom 地址可直接装在 `EndpointAddr` 中。没有选中路径时，Initial 经 peer actor 发往所有已知路径。因此一个按身份解析出 MQTT/IRC 路由地址的 lookup 可以接入现有流程；“发现还不知道身份的合法成员”仍需独立候选发现输入。[connect_with_opts](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1100)、[resolve_remote](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state/path_state.rs#L156)、[Initial fan-out](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L788)、[AddressLookup trait](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/address_lookup.rs#L317)。

**F — 接收端。** `Endpoint::accept` 返回 `Incoming`；此时可查看本地/远端载体地址，决定 `accept`、`accept_with`、`refuse` 或 `ignore`。握手完成后双方进入 `conn_from_noq_conn`，提取 TLS 认证身份与 ALPN，先注册连接到 socket/peer actor，再执行 `after_handshake`；正常 await 只有 hook 接受后才得到 `Connection`。[Incoming](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L138)、[注册和 hook 顺序](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L310)。

**F — 自动路径动作已经早于 hook。** `AddConnection` 立即接上路径事件/NAT 地址事件，调用 `update_qnt_candidates`，记录初始路径，然后 `trigger_holepunching`、`select_path`，才返回 path watcher 给 `conn_from_noq_conn`。仅在 hook 中等待秘密证明，不能阻止此前的地址发布、QNT 或路径选择。`max_remote_nat_traversal_addresses(0)` 也不是现成关闭开关：iroh builder 对小于 8 的值直接忽略。[AddConnection](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L382)、[配置下限](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/quic.rs#L525)。

**F — QNT 与同一连接。** QNT 从当前连接取得远端候选，只在 client 侧发起；同一 peer 多连接时选择 stable id 最小的 client 连接运行。候选变化、定时升级和网络变化会触发重试。noq 为成功探测的地址 `open_path_ensure(..., Backup)`；iroh 接到 Established 后重选，并把选中路径开到同 peer 其他连接上。因此 custom 起步后升级到 IP 有现成连接后机制，不必重新创建 TLS 身份或另拨一个业务连接。[QNT 调度](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L500)、[initiate_nat_traversal_round](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L921)、[noq 打开探测路径](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L5690)、[路径应用](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L680)。

**V — 证据边界。** 上述代码没有证明 MQTT/IRC 的实际消息大小、时延、重复与订阅丢失条件下能完成握手及升级，也没有证明双 NAT 下可取得有用 UDP 候选。已有 upstream custom 测试证明作者提供了 custom-only echo、local address 和 custom/IP 选择测试材料；它们不是本产品真实消息载体或业务隔离的测试结果。[现有测试入口](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/test_utils/test_transport.rs#L469)。

## 3. 可复用接口及不能从接口推导的保证

| 接口或接入点 | 可直接复用的事实 | 实施约束和建议 |
| --- | --- | --- |
| `Builder::hooks` / `EndpointHooks::before_connect` | 出站初始拨号前可拒绝；顺序执行多个 hook。 | 可做目标身份/ALPN 初筛；看不到后来 lookup、缓存或 relay 替换的实际资源，不能充当最终资源闸。 |
| `EndpointHooks::after_handshake` | 双向握手后拿到认证后的 `Connection`，可异步检查并拒绝。 | 可在返回应用前完成有界控制交换；需要额外的“注册但未允许候选/QNT/业务”内部阶段。持久索引用 `weak_handle`，避免 hook 强引用使连接不能按 drop 关闭。 |
| `Incoming::local_addr` / `remote_addr` / `accept_with` | 握手前能识别本地接收载体及使用特定 server config。 | 本地资源实例/用途可先绑定；此时远端地址不能当作认证身份。 |
| `CustomTransport::bind`、`CustomEndpoint::poll_recv/watch_local_addrs/create_sender` | 可把消息服务适配成包收发端点，local watcher 提供 custom 地址。 | bind 为同步工厂；持续异步订阅、重连、分片/重组、队列、限流、过期和取消由适配器负责；不得把消息字符串直接当作任意大小数据报。 |
| `CustomSender::is_valid_send_addr/poll_send` | 可判断本载体接受的地址，发送到 remote/source custom 地址。 | 地址合法性/资源允许性可在这里做第二道检查；收到的是密文包及分段信息，不含 stream/frame 用途标签。 |
| `RecvInfo::new(remote, local)` | 可携带本地接收 custom 地址，进而出现在 `Incoming::local_addr`。 | 是传输元数据，不是可信邀请、网络资格、用途或对端身份的证明；本地 adapter 必须据实际订阅/电路填写，不照信消息中的自报标签。 |
| `CustomAddr::from_parts(id,data)` | transport id + 任意 opaque bytes，可表达 route/circuit。 | 不自动绑定 EndpointId、网络、资源资格或有效期；适配器须校验长度/格式并管理本地映射。不要将秘密编码进可公开的地址。 |
| `AddressLookup::resolve` | 接收已知 EndpointId；多个服务结果并入候选。 | 可产出受控 custom 路由，不解决 A 不知道 C 的首次发现；必须过滤结果及之后实际发送。 |
| `Builder::path_selector` / `PathSelectionContext` | 可看各连接的已开路径和 `PathStats`，选择一个路径。 | 只能选已开路径；`none()` 保留旧选择，不表示禁用全部业务；同一地址可重复出现于多连接。没有网络授权/资源用途/连接用途字段，需内部补充可信策略上下文。 |
| `Connection::remote_id/stable_id/paths/path_events` | 认证身份、连接生命周期内稳定 id、路径快照与事件可观测。 | stable id 是本地观测标识，不是线上 CID 或跨节点相同的全局编号；用同一个持续 stream 和数据完整性一起验收连续性。 |
| `Connection::export_keying_material` | 提供现有 TLS session exporter。 | 是成熟协议做会话绑定的可用原料，不等于完整秘密持有证明已选定；不在此拼接 HMAC/哈希发明新握手。 |
| 内部 `MappedAddrs` / `open_path_on_conn` | 将 relay/custom tuple 映射给 noq，client 用 `open_path_ensure` 给同一连接打开新路径。 | 外部 API 不暴露任意开路径控制；适合 fork 内部复用。PathId/远端 CID 不足会排队重试，不应把有 candidate 就当成路径已验证。 |

接口证据：[hooks](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/hooks.rs#L46)、[custom traits](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/custom.rs#L24)、[Transmit/RecvInfo](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports.rs#L550)、[CustomAddr](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-base/src/endpoint_addr.rs#L333)、[selector](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L1284)、[观测及 exporter](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L1067)、[映射](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map.rs#L75)、[开路径](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L1031)。

## 4. 控制载体隔离：两个有明确裁决条件的实现假说

### 4.1 已证实不能靠 selector、Backup 或密文 adapter 完成

**F。** 默认 selector 把 IP 和 custom 都列为 Primary，relay 列为 Backup；custom 可能成为业务首选。选中 IP 后，`apply_selected_path` 仍保留 custom 路径并设为 `PathStatus::Backup`。锁定 noq 的调度在没有 Available 路径时允许 Backup 发送数据；握手中还允许 early data。QNT 的 ADD_ADDRESS、REMOVE_ADDRESS、REACH_OUT 也处在同一 `may_send_data` 条件下。简单把所有 1-RTT 包禁发会同时切断必需控制；简单把 carrier 设 Backup 会在故障时承载业务。[默认分类](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs#L81)、[保留 custom](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L700)、[noq 调度](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L1177)、[QNT 帧](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L6247)。

**F/I。** `Transmit` 已是包字节，外部 CustomSender 看不到其中业务流、DATAGRAM、QNT、确认或重传内容。由 MQTT/IRC 适配器解密并重写 QUIC 不是合适接缝，也不应制造第二套密码实现。用途判断要发生在仍知道连接/帧用途的边界，或采用下面严格分阶段、使业务连接根本没有控制路径的模型。[CustomSender](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/custom.rs#L83)。

### 4.2 H1：优先验证仅修改 iroh 的阶段隔离

**I/V — 候选实现，不是已定结构。**

1. 初始连接处于受限阶段，复用真实 TLS；只运行内部明确允许的申请/准入/路径控制。应用和 p2panda 正常协议拿不到可用连接，禁止使用 0-RTT/0.5-RTT 绕过阶段。
2. 经准入策略允许后发布候选并运行 QNT；验证至少一条合格数据路径。
3. 在同一连接交付业务之前，关闭该连接所有仅控制的 custom 路径，并保证自动选路、候选合并和重试不会重新打开它们。关闭须成功；noq `close_path` 会拒绝关闭最后路径。[关闭语义](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L593)。
4. 保留原 `Connection`、身份和已有内部控制 stream，随后才允许普通业务 stream。路径消息此后可经合格数据路径发送。
5. 数据故障且原连接尚存活时，可创建同身份的独立受限控制连接重新取得候选/运行 QNT，再向原连接打开合格数据路径。它只帮助原连接恢复，不能作为替换业务连接或把旧 stream 搬到新连接的伪连续性。

**需要的 iroh 内部改动。** 现有 actor 以 peer 为单位共享 `selected_path`，会把选中路径开到该 peer 所有连接；故至少增加连接阶段/用途及可传播路径资格，在注册、候选发布、QNT、路径事件和 `apply_selected_path` 一致执行。恢复控制连接必须能被选为 QNT 发起连接，不能继续无条件选最小 stable id 的旧 client 连接。此处复用现有 QNT 和开路径能力，改变资格与调度，不复制一套密码握手。[共享选择及传播](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L643)、[QNT client 选择](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L518)。

**必须验证的边界。** 数据路径刚验证就失效、控制路径关闭尚未两端收敛、业务已排队/有重传、并行多 ALPN 连接、双方同时引导、远端新开控制路径、载体重连、进程取消。入口本地绑定与对端认证后上下文须保持一致。测试需要双方真实实现及包/载体计数；不能只让正常业务客户端“自觉不发送”。接收侧至少保证受限阶段不交付普通协议、不接受其副作用，并明确拒绝违规用途；网络无法阻止第三方发来无效字节。

**H1 的停止条件。** 不能只在 `Established` 事件之后才发现业务连接被远端重新打开了控制路径，再异步关闭：事件已经发生在 noq 处理包/验证路径之后，是否存在先发出排队业务的窗口必须验证。控制载体的本地路由绑定若不能在包进入对应连接前排除错误连接/用途，H1 就不能声称纯 iroh 已保证隔离。此时先评估最小的 noq 连接/路径准入钩子与可信元数据传递；这仍小于 H2 的帧级调度改造。不得靠可猜测或远端自报的 route id 绕开该检查。

**恢复限度。** 独立控制连接的保活不会延长原连接寿命。iroh 默认 path idle 15 秒、relay path 30 秒，受限 builder 不允许任意放大；noq 对“远端放弃最后路径”另有约 3 PTO 的恢复 grace。具体失败原因的窗口要实测，不能一律写成最后路径立即死亡或保证永久保留。[iroh 超时](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L109)、[noq grace](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L646)。若 H1 恢复窗口无法满足已确认行为，应说明具体失败和代价再调整；不能静默增加无限超时。

### 4.3 H2：确有需要时，扩展 noq 的控制/业务调度边界

**I/V。** 若要在原业务连接仍有待发/重传业务的同时，保留同一连接上的 MQTT/IRC 路径用于控制和保活，H1 的“业务前移除控制路径”条件不成立。此时需要在 noq 组包/重传选择之前，区分路径用途与帧/stream 资格，且处理接收侧违规、early data、path validation、ACK、flow control、连接关闭及 QNT 的允许规则。若准入协议使用普通 QUIC stream，光区分 STREAM 与非 STREAM 仍不够，需要确定哪些内部控制 stream 可走受限路径，业务 stream 不可走；DATAGRAM 也必须覆盖。

这不是“重写 QUIC”或改变算法的理由；首先提出最小本地策略扩展，复用既有 TLS、加密、拥塞与重传，并运行安全和一致性验证。源证据已经证明现成两个 PathStatus 不能表达这种资格，但本次尚未验证最小扩展的接口/线协议影响。不预先承诺 noq 无需修改，也不预先把整个 noq fork 当必做项。

## 5. 准入、元数据绑定和 p2panda 的最小接缝

**I。** 将“本地 carrier/resource 实例与用途 → 本次连接尝试 → TLS 认证 EndpointId/ALPN → 网络上下文及准入阶段”保存在连接引擎的可信状态中。`CustomAddr` 和 `RecvInfo` 提供运输映射，不自动提供这条绑定。策略不得使用消息中自报的 `member=true` 或“这是控制帧”来授予资格；remote custom 地址由 TLS 认证完成后才能关联到身份。迁移准备可能临时双网，因此不能把“同一个 peer actor”误当成“同一个网络授权”。[接收地址映射](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L646)、[认证身份提取](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L375)。

**F/I。** 正常 Router 先 await `on_accepting`，再调用 `ProtocolHandler::accept`；默认 `on_accepting` 等完整连接，因而可复用 hook 作为普通协议的放行点。但自定义 handler 可以在 `on_accepting` 转入 0-RTT 并运行长逻辑。产品集成必须审计所选 p2panda 构造/handler 路径，确保 Discovery、Gossip、LogSync 和业务协议不会走早期旁门；首次申请、持有证明和必要验证材料各有自己的受限协议范围。不能把所有成员证明都挡在“业务连接返回后”而造成无法完成准入的循环。[Router](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/protocol.rs#L625)、[on_accepting](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/protocol.rs#L228)、[early data](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L600)。

首次申请没有长期准入秘密，不能直接复用“现成员返回”的同一门槛；邀请绑定目标、申请权限、秘密证明、成员资格和数据路径可用是不同事实。具体成熟持有证明的选择是生产准入切片的真阻塞；它不阻塞隔离实验使用明确标记的固定准入判定来证明连接/路径行为。实验替身不能成为产品密码实现。

## 6. 资源禁用、候选观察、质量和维护

### 6.1 资源许可必须覆盖最终实际资源

**F。** `Minimal` 只补 crypto provider；`N0` 安装官方 Pkarr 发布/查询、DNS lookup 和 relay，`N0DisableRelay` 仍含官方 lookup。完全关闭 native relay 可用 `RelayMode::Disabled`。但允许自建 native relay 时，`RelayMap` 不是出站 URL 白名单：`RelaySender::is_valid_send_addr` 恒真，actor 可按任意得到的 URL 创建连接，map 查询只取得 auth token。更隐蔽的是 `active_relay_handle_for_endpoint` 可能改用已有、知道该 peer 路由的另一个 relay。[presets](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/presets.rs#L45)、[RelaySender](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay.rs#L282)、[实际 relay 替换及创建](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs#L1278)。

**I。** 配置组装用 Minimal + 显式获准资源；内部统一策略既过滤邀请/地址簿/lookup/缓存候选，也在实际拨号、订阅、relay 替换、包发送和回退处检查最终资源及用途。分清官方 relay、官方 lookup、观察服务、消息服务和普通成员业务 relay 的作用域。资源更新合并必须保留用户禁用；不能只测默认配置中没有官方域名。运行时热撤销尚未定，若暂采用重启生效要明确这一边界，不把现成 `remove_relay` 当作已关闭所有缓存路径和连接的保证。

### 6.2 没有 native relay 的 UDP 候选需要单独完成

**F/I。** 当前 `DirectAddrUpdateState::run` 在 `RelayMap` 为空时提前返回，连随后的 portmapper 都跳过。现有 `Endpoint::add_external_addr/remove_external_addr` 可注入/撤回候选，但不负责从任意 STUN 获取同一 socket 的映射，也不证明地址新鲜或目标无关。应拆开“观察资源”与“业务 relay”，复用现有 UDP socket 和 direct-address watcher，把可靠来源、观察时间/失效及资源许可纳入候选控制。MQTT 的 TCP 出口地址不能充当 UDP 候选。[空 RelayMap 分支](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L778)、[external address API](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1013)。这是真实双 NAT 旁信道升级的前提，却不是 LAN/明确地址隔离实验的前提。

### 6.3 质量策略可以从公开 selector 起步，维护策略仍需内部扩展

**F。** selector 可用 `PathStats`，现成默认只做分层 biased RTT 和 5ms 黏性，不测产品吞吐/稳定性目标。它只在连接加入、路径建立/放弃等 `select_path` 调用时重选；周期 `check_connections` 只检查是否存在 RTT≤10ms 的 IP 路径并触发 holepunch，不是定期按新 RTT 重选。因而只替换 selector，不能保证“同样两条仍存活路径的质量后来变化”会及时换路。[默认比较](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs#L137)、[check_connections](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L751)、[actor 事件循环](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L278)。

**F。** 同候选 holepunch 最小间隔 5 秒，升级检查 60 秒，无连接 actor 60 秒空闲退出。builder 把默认 path keepalive 大于 5 秒的值忽略、path idle 大于 15 秒的值钳制；只调整公开 QUIC 参数无法表达手机/网关的完整维护预算。网络变化 hint 又固定把 relay 视为可恢复、custom 视为不可恢复，custom endpoint 没有与原生网络变化 sender 等价的接口。[常量](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L45)、[参数限制](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/quic.rs#L484)、[恢复 hint](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L1697)、[network change sender](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports.rs#L482)。

**I。** 内部策略接受本节点预算、业务活跃度、失败历史、新线索/网络变化；协调 lookup、旁信道订阅/重连、观察、QNT、质量采样和空闲回收。先按许可/用途筛选，再比较质量，增加有界质量重评触发，避免“已有 IP 就永远更优”。指标/阈值通过可重复测量选择，不在拆票时编造。全网选哪些成员由产品/p2panda 调度，给定 peer 的路径恢复由 iroh 统一；长期离线状态不能只靠会退出的 actor 缓存。

## 7. 普通成员中继的可复用部分与新增责任

**F/I。** native relay actor 面向 URL 指定的 relay 服务，不会把 A-B、B-C 两条成员连接自动变成 A-C 电路。可用 custom 包载体把 A-C 的 QUIC 包放进既有 A-B/B-C 连接中的受限转发协议；B 解开外层连接后仍只能看到 A-C 密文和必要路由元数据。内层 A-C 继续复用原认证、流与多路径。`CustomAddr` 可定位 B 与电路，`MappedAddrs` 可把它变成原连接路径，但电路并不是现成 API。[native relay 创建](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/relay/actor.rs#L1340)、[CustomTransport](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/custom.rs#L24)。

**I/V。** 新增的是显式启用、建立/撤销电路、端点/电路绑定、允许信令与允许业务的独立检查、有界队列和背压、寿命/取消、故障及资源计数；从一个中间节点做起，不隐含任意多跳路由。把信令转交包的大小放大成任意业务包不是实现中继。TCP/WS 是另一种外层载体，还需分帧、背压与队头阻塞验证；它们不应阻塞 UDP + 已支持 relay 的第一条生产路线，也不能因 trait 存在便宣称可用。

## 8. 建议的八个纵向切片

以下是供总体拆票整合的候选切片，编号不是已创建的 tracker 票。先技术门槛后依赖实现；同一项可合并进更完整的产品切片，但不要把“研究已读源码”当作“运行门槛已过”。除依赖于外部成熟证明或目标平台外，不需要为普通内部接口选择重新请求产品授权。

### I1 — 两节点在获准资源内完成真实连接，禁用资源无访问

- **可见交付：** 两个独立身份通过显式允许的 direct/native relay 连接并 echo；保留自建 relay 时，禁用 relay/lookup 从任意候选来源注入也无实际访问；全部资源不可用时返回可解释失败。
- **真阻塞：** 无旁信道/完整成员制度依赖；需要选定可编译的 iroh 依赖及一个真实受控 relay。可与其他接入研究并行。
- **指导：** Minimal 显式配置、持久用户 override、输入与最终实际资源两道闸；覆盖 actor 的 relay 替换。复用 Endpoint/Connection，避免先构造庞大业务外壳。
- **验收：** 邀请式显式地址、lookup、地址簿/缓存、已有 relay 路由、配置更新和失败回退分别注入被禁资源；抓网络访问与 adapter 请求并证明获准资源仍工作。测试无须绑定私有 actor 调用次数。

### I2 — 按目标身份经真实消息旁信道完成受限 iroh 握手

- **可见交付：** 普通一次 connect/accept 经至少一个真实支持载体（优先一个 MQTT 服务；IRC 作为另一独立适配）完成双方身份认证和有限私密控制；没有数据路径时明确只处于控制阶段。
- **真阻塞：** 可独立验证握手，不依赖完整秘密证明、p2panda 全同步或复杂 NAT；产品化资源访问遵守 I1。
- **指导：** 复用 custom transport、lookup、TLS；载体 adapter 限制消息大小、分片重组内存、过期、重复、顺序和取消；公开封套无长期秘密。身份由 TLS 决定，carrier 标签只作路由。
- **验收：** 第三节点同订阅、同邀请、注入/重放；替换目标身份失败；断订阅、重复/乱序、消息截断、双方同时发起、取消均有界；真实控制内容不向第三方泄露。此切片不声称业务隔离/升级门槛已过。

### I3 — 同一连接升级且业务永不回落控制载体的可行性门槛

- **可见交付：** 从 I2 的同一连接获得合格数据路径，沿用 stable id；公开 stream 可传输，所有数据路径故障时普通业务不进入控制载体。输出 H1 可行实现界限或 H2 的最小必要扩展与兼容影响。
- **真阻塞：** I2 的真实载体；可先用 LAN/明确可达地址隔离调度问题，不等待双 NAT 观察能力。后续宣称安全业务可用的切片必须等待本门槛。
- **指导：** 优先 H1，注册受限阶段、关闭 control paths 后再交业务、按连接用途过滤 peer 共享路径；若保留控制路径与业务并发是必需，转 H2，不能用降低 selector 权重“通过”。
- **验收：** 数据未就绪先排队、数据路径刚建立即失败、持续大流/重传中故障、并行控制/业务连接、远端开新路径、正常 stream 与 DATAGRAM；两端行为及实际载体流量共同证明无普通数据回退。重拨出来的连接不得替代同一连接成功断言。

### I4 — 禁用 native relay 后仍能从消息引导取得同 socket UDP 路径

- **可见交付：** 两个 NAT 后节点经获准消息入口和独立地址观察形成实际 UDP 路径，无官方依赖，也无需用户手动填写地址。
- **真阻塞：** I2，以及 I3 对安全数据交付的门槛；候选观察工作可与 I3 并行。需要真实可控 NAT/观察设施，不以假的成功结果验收。
- **指导：** 解开 RelayMap 与观察/portmapping 的耦合；复用同一个 UDP socket、direct-address watcher、QNT，管理候选新鲜度/失效。数据 relay 并非强制依赖。
- **验收：** relay map 为空、映射过期、外部地址变化、错误观察结果和目的相关 NAT；可达时升级，不可达时停留明确不可用或用其他获准数据路径，不承诺任意 NAT 成功率。

### I5 — 准入结果控制候选披露、协议启动和关闭

- **可见交付：** 首次申请者仅能申请/接收获准材料，返回成员按成熟秘密证明及成员验证进入正常同步；本地收到有效撤销后新连接拒绝、现有网络/业务连接关闭。
- **真阻塞：** 生产密码路径依赖总体计划中的成熟持有证明选择/会话绑定和成员授权判定；I3 阻塞普通业务资格。可先用固定判定验证接线，但不得把它作为完整准入交付。
- **指导：** 连接引擎接收抽象准入结果，不写死创世/管理员角色；受限注册后分阶段放行 QNT 与应用；审计 p2panda 的构造、早期 handler、主动/被动拨号；保留结果索引用 weak handle。
- **验收：** 无秘密、旧秘密但无成员授权、已知撤销身份、首次申请、错误网络/目标、0-RTT 重放/0.5-RTT 提前发送，均不能取得越权历史或触发副作用；同步完成不是首次建连的循环前提。

### I6 — 在获准路径中按测得质量切换同一条业务流

- **可见交付：** direct 与获准 relay 同时存活，质量随时间变化时按策略切换，原 stream 保持；显示路径变化及没有合格路径的状态。
- **真阻塞：** I1 许可和 I3 用途隔离；不依赖普通成员 relay 或所有 NAT 类型。两个实际可用路径足够验证。
- **指导：** 公开 selector 先做质量评分，内部增加有界重评触发和可信资格上下文；质量采样开销纳入预算，避免 RTT 单指标被误称吞吐最优。保持去抖，不保留固定“所有 IP 永远胜 relay”规则。
- **验收：** 无路径建立/关闭事件、仅带宽/延迟/丢包变化时仍能重评；弱直连可让位于更好 relay；被禁或 control-only 路径再好也不选；无意义振荡不突破既定预算。数值阈值由测量产出。

### I7 — 每节点预算下自动恢复联系并尽量保留原连接

- **可见交付：** 手机保守、网关积极两种节点在未发起业务时均能按预算维护；连续失败退避，新线索/换网/业务触发可恢复尝试；旧连接可恢复时旧 stream 继续，否则明确关闭，节点仍可继续找新连接。
- **真阻塞：** I3 对 H1/H2 恢复接法的裁决、I5 准入接线；真实双 NAT 恢复依赖 I4。未知成员发现依赖总体计划的候选发现，不能以 AddressLookup 代替。
- **指导：** 单一 peer 连接引擎合并同端点需求、取消及超时，持久线索由产品供给；按载体能力修正 recoverable hint，避免 custom 网络变化一律死亡。独立控制连接只帮助原连接开新数据路径，不替代它。
- **验收：** 新旧线索混合、无成员回应、进程重启、当前路径失效有/无备用、恢复晚于旧连接关闭；同时观测身份、旧 stable id/stream、恢复状态及资源消耗。所有预算都不能暗开被禁资源或业务中继。

### I8 — 显式启用的普通成员提供一跳端到端数据中继

- **可见交付：** A-B、B-C 已连时，B 只开信令可帮助控制但不转普通业务；明确启用后 A-C 经 B 建立内层端到端连接并传输，B 看不到 A-C 明文；中途换 direct/D 时保持可保持的原连接。
- **真阻塞：** I3 用途隔离、I5 成员/电路资格；I6 用于质量选路，I7 用于完整故障恢复。它不应阻塞 native relay/direct 的首条可用路线。
- **指导：** 从单中间节点电路协议与 custom path 做起；有界队列、背压、寿命/撤销、用途/资源双重许可；不新增固定角色系统或任意多跳自动路由。
- **验收：** B 未开启、开启后关闭、非成员/伪造电路、超预算、B 掉线、D 获准/被禁、有/无可恢复原连接；比较内外层身份并验证 B 只能解外层封装。TCP/WS 的具体载体矩阵另凭证据扩展。

## 9. 拆票时的最小改动归类

| 需求 | 现有配置/适配足够的范围 | 必需或有条件的内部改动 |
| --- | --- | --- |
| 自建资源、完全不用官方默认 | Minimal + 显式 lookup/relay；全部 native relay 禁用。 | 保留部分 relay 时的全入口许可与最终实际资源闸；配置更新保留禁用。 |
| 消息载体真实握手 | CustomTransport + 已知身份 lookup + TLS。 | 受限阶段、元数据绑定、载体预算；不能把握手成功直接视作业务可用。 |
| 初次同一连接升级 | QNT、MappedAddrs、noq 多路径已有。 | H1 所需连接用途和关闭控制路径；真实载体/NAT 门槛；H2 才涉及帧/stream 用途调度。 |
| 准入后才披露/探测/启动协议 | after_handshake 和 Router 是可用接缝。 | hook 前已注册的自动动作必须受阶段控制，覆盖 early-data/自定义 handler；可提最小上游扩展，不需把领域角色嵌进库。 |
| 质量优先 | PathSelector 可替换默认比较。 | 新质量变化触发、资格上下文、探测“足够好”判断和预算一致性。 |
| 每节点维护与恢复 | 复用现有网络事件、QNT、relay 重连及 path events。 | 节点预算与失败退避、custom 恢复能力、原连接存活窗口内的路径恢复；不能仅调现成 keepalive 参数。 |
| 普通成员中继 | 已有连接承载转发协议 + CustomTransport 包入口。 | 电路/用途/资源控制和路径恢复；是否需要 noq 按 H1/H2 门槛决定，不自动扩大到多跳或全传输支持。 |

这些切片保留已确认的身份、安全会话、可替换设施和失败语义。若实验表明需改变“同一连接升级”、准入可见性或载体用途等行为，应提交具体失败证据、最小替代方案和代价再更新决议；内部 API 和 actor 划分可在上述行为约束内调整。
