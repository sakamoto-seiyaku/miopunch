# 准入秘密持有证明：实施票的源码依据与验证边界

2026-09-29。输入为已发布《miopunch 新基线实施规格：身份、入网、连接、状态与迁移》0.1，重点是 Implementation Decisions §6–7 与 Further Notes。本文只提供拆票证据及候选接法，未修改规格、创建票或运行验证，也不重新讨论已接受的旁信道 QUIC 引导方向。

核查版本：iroh v1.2.0 / `17c0612f80f78f5288e97b818b1360ae6ea0a51a`，p2panda v0.7.1；另外下载检查 rsasl 2.3.1 发布源码，其 `.cargo_vcs_info.json` 指向 `ee7065d92976ceb1fc1b0c27545f8a816e3de166`。协议依据为 RFC 5802、7677、9266；PAKE 只作有条件的备选比较。

## 结论与建议范围

**把「验证 SCRAM-SHA-256-PLUS 与 iroh TLS exporter 的准入接法」设为一个具体技术资格验证任务是可行的；现在不能把 rsasl 2.3.1 写成已经选定、可直接投产的依赖。** 它有真实的双端 SCRAM 实现和 channel-binding 注入接口，不需要增加另一套加密载体；但静态检查发现必须验证的强制绑定与成功判定细节。先用真实 iroh 会话验证这个窄候选，通过后固定应用认证 profile；失败时根据失败原因换成熟实现或评估 PAKE，不退回手写挑战/HMAC 组合。

三个结论应独立保存：

| 已获事实 | 能推出什么 | 不能推出什么 |
| --- | --- | --- |
| iroh TLS 验证节点密钥 | 连接另一端控制经过认证的 EndpointId 对应密钥 | 它拥有本网准入秘密、当前是成员或有管理权 |
| 当前连接通过秘密持有证明 | 当前节点会话具备指定网络/秘密代际的受限引导资格 | 当前成员有效、所有 ALPN 放行、下条连接自动免验证 |
| 取得并验证管理记录，计算本地授权 | 按当前本地视图开放相应同步/业务范围 | 已知全网最新状态或撤销已瞬间到达所有成员 |

这三层来自现有产品契约；底座接入必须保留区别。已知被撤销身份仍拒绝；旧秘密持有者换新身份所保有的引导窗口是已接受边界，不靠本次密码选型暗中改变。

## SCRAM 候选的具体输入、输出和限制

SCRAM 定义客户端证明和服务端证明；`-PLUS` 把认证 transcript 绑定外部安全信道，SHA-256 变体由 RFC 7677 指定。SASL 不要求 TCP 或某种中心服务器，应用可在一个有序 QUIC 双向 stream 中运送其不透明消息。角色可直接跟随该 connection 的发起/接收方向；双向同时拨号形成两个独立认证实例，不需要两边同时充当同一实例的 client。[RFC 5802 §5–6](https://www.rfc-editor.org/rfc/rfc5802.html#section-5)、[RFC 7677 §3](https://www.rfc-editor.org/rfc/rfc7677.html#section-3)、[rsasl 协议接入说明](https://docs.rs/rsasl/latest/rsasl/#protocol-implementations)

候选 profile 的接口范围如下，具体编码尚未冻结：

- **输入：** 已完成握手的 iroh connection、双方已认证节点身份、待验证网络及秘密代际、准入秘密、明确 client/server 角色、消息/时间/计算预算。准入秘密是系统产生并安全交付的高熵材料；验证票必须核对生成与保存接口，不能把普通短口令默认为等价输入。
- **TLS 绑定：** 按 RFC 9266 使用 label `EXPORTER-Channel-Binding`、空 context、32 字节输出，命名为 `tls-exporter`，仅供 channel binding；不能把这份公开绑定值当作新的加密秘密。应使用完成握手后的普通 exporter，不能拿 0-RTT / early exporter 替代。[RFC 9266 §2、§4](https://www.rfc-editor.org/rfc/rfc9266.html#section-2)
- **SCRAM 配置：** 只允许 `SCRAM-SHA-256-PLUS`，绑定类型固定为 `tls-exporter`；禁止 PLAIN、无绑定 SCRAM 和 `tls-unique` 回退。`AuthId` 用于选择本地准入验证域，不能据它相信对方自报的节点身份；任何节点字段仍须等于 TLS 认证结果。网络/代际选择必须进入经过认证的协议上下文并由两端检查，不能仅在未认证封套中决定授权范围。`AuthzId` 不用于取得管理员或代理身份。
- **输出：** 只产生与当前 connection、网络、秘密代际、对端 EndpointId 绑定的「引导准入通过」事实或明确失败。客户端须验证服务端 final；服务端须确认库的验证成功结果。不得输出网络共享会话解密密钥，也不将该事实扩展成永久成员资格。
- **状态与计算：** SCRAM 服务端需要 salt、迭代次数、StoredKey、ServerKey；本产品各节点本来都持有准入秘密，可本地建立这组验证材料，无须预先知道陌生节点名单。缓存须按秘密代际和 SCRAM 参数隔离，敏感派生材料也不能进入共同状态复制。迭代次数与消息长度须有上下限，拒绝恶意候选造成无界计算。[验证材料接口](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/properties.rs#L23)

**密码学适用限制。** SCRAM 不是 PAKE。其 transcript 可供离线猜测；未知候选即使通过了自己的 TLS 节点密钥验证，也可能是主动恶意对端。这使高熵随机准入秘密成为选择该候选的必要前提，而不能宣传「有 TLS 就能安全使用任意短密码」。本研究未给出密码强度的新产品选项。[RFC 5802 §9](https://www.rfc-editor.org/rfc/rfc5802.html#section-9)

**每连接一次认证。** RFC 9266 §4.1 限制同一 TLS connection 上的绑定值不能用于多个认证机制实例，并要求上层协议结束时结束对应 TLS 连接。候选接法应把认证后的网络协议视为一个持续的会话，定义清楚其生命周期；不能在不同 stream 上反复独立认证、换网认证或任意重认证而仍声称自动符合该 profile。轮换后的重新证明或迁移至另一网络，优先使用新的 connection；同一 connection 的路径切换无需重做证明。正式 profile 必须说明此条的符合性，运行成功不能替代标准语义核对。[RFC 9266 §4.1](https://www.rfc-editor.org/rfc/rfc9266.html#section-4.1)

## rsasl 2.3.1：真实可复用处与不能跳过的资格检查

已检查发布源码中的 client、server、mechanism 选择、签名计算和完成状态，没有仅依据 crate 名字推断能力。

| 代码事实 | 接入含义 |
| --- | --- |
| `SCRAM_SHA256_PLUS` 注册 client 和 server 构造器；`SASLClient/Server::with_cb` 可注入按名称提供数据的 callback | 能把 iroh exporter 传入已有 SCRAM 状态机，应用只处理有界 framing、角色、时限及授权结果。 |
| client 先取 OverrideCBType，否则尝试 `tls-exporter`，不存在则尝试 `tls-unique` | 明确固定类型并失败关闭；不能依赖自动选择恰好符合 TLS 1.3 要求。 |
| server 的 `new_plus` 设置支持绑定；GS2 `y` 分支拒绝，`n` 分支为空操作；仅 `p=<type>` 分支取绑定数据 | `-PLUS` 名称本身未形成「必须使用 exporter」的完整强制条件，见下文。 |
| server 错误 proof / 不匹配 binding 可输出 `e=...` 后返回正常的 Finished 状态；成功路径另外调用 `validate` | `State::Finished` 仅表示交换结束，绝不能直接视为通过。要求成功 validation，失败消息与本地失败状态一致。 |
| client 解析并比对 server-final verifier；不符产生 `MutualAuthenticationFailed` | 客户端不能在发送 client-final 后提前开放引导状态。 |
| client 把对端 iteration_count 解析为 u32，拒绝零值后可进入 PBKDF2；提供参数的 callback 会先获得 iterations/salt | profile 要在昂贵计算前限制参数和输入长度；超时不能单独替代 CPU 预算。 |

源码：[机制构造](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/mechinfo.rs#L194)、[channel-binding 接口](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/channel_bindings.rs#L7)、[client 绑定选择](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/client.rs#L167)、[server 输入与验证](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/server.rs#L149)、[client final](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/client.rs#L415)、[计算参数](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/client.rs#L284)。

**GS2 检查已追到末端。** `server_start_suggested` 按机制名选构造器；进入 `n` 分支后保存的只有 GS2 header，final 只检查它与解码后的 `c=` 相同。`compute_signatures` 使用给定的 `c=`，没有后补 exporter；成功后进入 validation。因此静态上不能依赖 rsasl 原样强制绑定。本轮没有运行恶意 transcript，不把它表述为已经复现的漏洞；验证任务须包含 `-PLUS` 机制下的 `n,,`、`y,,`、错误类型、缺失/错误绑定负例，并选经验证的入口约束、最小上游修复或另一实现。正确解决强制策略不是另造密码算法。[机制选择](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/sasl.rs#L183)、[final 检查](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/server.rs#L306)、[签名输入](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/src/mechanisms/scram/tools.rs#L26)

维护证据只到发布记录：2.3.1 的 CHANGELOG 标记 2026-04-23，并记录 SCRAM parser 扩展属性处理修复；这证明存在具体版本及修复记录，不能据此声称已审计或安全无缺陷。本轮不是该库的完整安全审查。[CHANGELOG](https://codeberg.org/dequbed/rsasl/src/tag/v2.3.1/CHANGELOG.md#L22)

## iroh 的实际门控位置

`EndpointHooks::after_handshake` 在进、出连接均执行，收到已知 EndpointId 和 ALPN 的 `&Connection`，可返回异步 future；拒绝会关闭连接。它可以在交给正常调用方前运行受限 stream 上的准入交换，`export_keying_material` 已可直接调用。因此它是首个应验证的接点，不必先重写 TLS。[hook 契约](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/hooks.rs#L87)、[exporter](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L1075)

但它不覆盖全部条件：

1. **QNT 先启动。** `conn_from_noq_conn` 先注册 connection、等待路径信息，之后才 await hook。remote actor 添加 connection 会填本地候选并触发 QNT。若准入前禁止详细候选交流，必须在 fork 更早处延迟候选发布/探测；单挂 hook 不够。[注册顺序](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L345)、[QNT 启动](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L397)
2. **0-RTT 可提前取得句柄。** `into_0rtt` 在最后的握手/注册/hook 完成前返回早期 connection；自定义 ProtocolHandler 的 `on_accepting` 可接管这条路径。首个 profile 应不让正常协议使用此旁路，入网核销、交付和准入证明也不得在 early data 里生效。[0-RTT API](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L497)、[handler 接管点](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/protocol.rs#L228)
3. **正常连接返回只适合一种资格。** 证明成功后，未知节点只应获引导记录范围，不应直接返回给所有普通协议。需要明确 connection 的当前阶段，允许受限引导取得授权依赖，并在成员验证后才进入正常范围；不能把 hook Accept 与成员授权合并。哪个受限 ALPN/内部控制 stream 承担此阶段属于实现 profile。
4. **每个协议和连接都要覆盖。** Discovery、Gossip、LogSync、业务可能各自拨不同 ALPN/connection。不能把某个 endpoint 已证明的 bool 用于跳过另一 connection 的绑定验证。连接缓存应持有具体认证会话事实；移除、轮换和授权修正还要使相应资格失效或关闭连接。
5. **同一连接用途隔离仍独立。** 通过秘密证明并不许可仅信令的旁信道运送普通同步/业务；这个限制仍由载体用途门控执行，不能由 SCRAM 结果或路径评分替代。

p2panda v0.7.1 的出站 actor 等待常规 `Connecting` 完成后才向调用方交付；入站经过 `protocol_handler.on_accepting` 再调用 `accept`。因此正常路径可以复用上述门控，但自定义 early handler 和所有协议必须逐项覆盖。其 Endpoint 目前在 actor 内部自行 builder/bind，需薄适配传入策略/hook 或接入选定 fork 的等价机制；不是所有准入逻辑重写进 p2panda。[连接 actor](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/connection.rs#L97)、[Endpoint 构造](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L197)

## 首次申请例外与未知发现：两个有限缺口

**首次申请不能套用必须持有长期秘密的入口。** 可信邀请给出指定联系端的 EndpointId；TLS 必须对上该目标。申请端没有准入秘密，只进入申请、审批/核销、已批准材料交付及幂等恢复范围；无权读取通用管理历史。服务端绑定真实 TLS 节点身份而非请求正文自报身份，批准/核销之后才交付准入秘密。失效邀请、改变节点身份或切换 ALPN 都不能扩大这个例外。此处「向批准节点安全交付秘密」与「向未经准入验证的陌生候选直接发送秘密」是不同的授权动作。

**持有证明不解决首次找到谁。** iroh lookup 接受已知 EndpointId；p2panda 的 discovery session 在连接上运行。返回 A 不知道 C 时，仍须有一份有限的候选交换 profile，交代双方如何取得至少一个可拨目标身份及回复路由，再开始 TLS 和秘密证明；不能先假设已有成员同步结果。[iroh lookup](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/address_lookup.rs#L544)、[p2panda discovery session](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/discovery/actors/session.rs#L80)

该 profile 必须列明命名空间、随机投递标识、候选 EndpointId 各由谁看见，怎样限制主动候选滥用和过期线索；若想让某些字段不被 broker 看见，需指出实际保护机制。SCRAM 要在先有 TLS 后执行，不能循环声称它加密了找到 TLS 对端之前的候选消息。临时 topic 不等于私密命名空间，公开非秘密路由标识也不等于身份授权。不得把准入秘密本体用作 topic、ALPN 或候选声明字段。p2panda 的 `NetworkId` 明确不是秘密，也不能拿它替代持有证明。[NetworkId 文档](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-net/src/lib.rs#L285)

这只是一个受限 bootstrap profile 缺口，不要求发明 DHT、专用最小成员证明包或第二套全网目录协议。

## 为什么当前不直接选 PAKE 或 p2panda-encryption

| 候选 | 此任务的适用性 | 尚需付出的接入工作 |
| --- | --- | --- |
| SCRAM-SHA-256-PLUS | 与现有 TLS 身份/机密信道契合；有明确双端 Rust 实现和 exporter 接口，适合首先资格验证 | 高熵秘密前提、强制绑定、成功状态、作用域及预算、全协议门控；候选库资格尚未通过。 |
| PAKE，例如 SPAKE2 | 若输入必须是低熵人类口令，PAKE 提供的防离线猜测属性会成为相关要求；不能把 SCRAM 宣传成同类协议 | 选择与标准匹配的现成实现、角色及身份/信道上下文绑定、双方 key confirmation、错误/重试和兼容性验证。RFC 9382 还需要应用定义密码映射等参数；不是装上算法即可取代所有准入门控。 |
| OPAQUE | 标准化 aPAKE，主要有登记、客户端记录及服务端设置，不能因「更新」而直接更适合共享秘密的对等引导 | 需要解释节点对等角色怎样映射登记/服务端模型；当前没有这个额外产品需求。 |
| p2panda-auth / encryption | auth 表达组成员和访问授权；encryption 的 2SM 用 X3DH/HPKE 为 DCGKA 分发秘密 | 都未提供本规格的「既有 iroh connection 上验证共同准入秘密」现成 profile；不能把有签名记录/组加密当作这个证明已实现。 |

依据：[SPAKE2 标准与 key confirmation](https://www.rfc-editor.org/rfc/rfc9382.html#section-3.1)、[OPAQUE 总体流程](https://www.rfc-editor.org/rfc/rfc9807.html#section-3)、[p2panda-auth](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-auth/src/lib.rs#L1)、[p2panda 2SM](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-encryption/src/two_party/two_party.rs#L20)。此比较不评定所有 PAKE 库的生产成熟度；本轮没有核查其实现或审计。

## 可直接用于拆票的三个切片

### 一、确认一个成熟持有证明 profile 与实现

**输入：** 本文固定源码、可运行的最小真实 iroh 双节点入口。无需等待 MQTT 实现，用真实 QUIC connection/exporter 即可检验密码接线。

**产出：** 协议/库/版本、角色、网络/代际绑定、成功状态与生命周期、凭据缓存和预算的短 profile；可复现资格验证及证据；若 rsasl 不通过，给出具体失败及最小替代，不交付临时密码组合。

**通过条件：** 双端正确秘密通过；错误秘密、错误服务端 final、错误 exporter、跨 connection 搬运、旧 transcript、非 PLUS/非 `p=tls-exporter` 失败；输入长度/迭代数受限；验证成功与 Finished 区分；真实 TLS 两端 binding 一致，新 connection binding 不复用；至少与标准向量/独立实现之一交叉检查。测试只证明这些行为，不宣称完整密码审计。整个过程中准入秘密不在协议报文、封套或日志原样出现。

### 二、接入受限引导与所有协议门控

**依赖：** 切片一通过及选定 iroh/p2panda 最窄接线；载体用途资格按相关实现票接入。

**产出：** 以 connection 为单位的阶段事实；hook/更早 QNT 门控的接入；受限引导、首次申请例外、成员验证与正常协议放行的统一执行边界。产品授权政策仍从外部输入，不写死为 iroh 成员角色。

**通过条件：** 未证明节点无法取得管理历史；有秘密但无成员资格只能获准引导；成员验证后才正常同步；Discovery/Gossip/LogSync/测试业务均不绕过；首次申请无秘密仍可向指定节点申请，但不能读通用历史；错误邀请/身份替换失败；0-RTT、并发连接和不同 ALPN 不提前放行；超时/取消清理受限会话；QNT 信息披露遵守当前阶段；撤销影响已有和新连接；正常业务不回落仅信令载体。

### 三、闭合「只剩未知合法成员在线」的启动链

**依赖：** 切片一、二及至少一个真实旁信道适配已可用。

**产出：** 仅涵盖初始候选及路由的 bootstrap profile 和可观察元数据清单；与既有成员历史补齐串接的窄场景证据。

**通过条件：** A 本地只知离线 B、C 唯一在线且 A 不知 C；A 可由获准入口发现候选，核对实际 TLS 身份、完成秘密证明，取得并验证必要管理历史后进入正常范围。假候选、过期路由、错误网络/代际、缺秘密者和已知撤销身份不能获得越权范围；旧秘密持有者新身份按规格只保留被接受的引导窗口。抓取 broker/转交端的可见内容验证元数据说明，不能以「随机 topic」替代该检查。

切片一是机制资格验证；切片二、三把它接成真实行为。源码已经确认接口和缺口，后续票不必再以「研究一下是否有 TLS exporter」为任务，也不能把上述尚未运行的验收写成已通过。
