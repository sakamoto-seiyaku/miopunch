# 私密信令：优先复用 iroh 安全会话的核查

2026-09-28。源码核查；用户随后接受可替换旁信道承载受限 iroh 握手与控制的设计方向，见[对应 ADR](../adr/0005-side-channel-control-bootstrap.md)。未改造或运行库，具体集成推论与验证项仍保持其原有证据等级。服务于「明确入网前后私密信令的信任材料与保护边界」，不重新讨论可信个人邀请、独立节点身份及第三方不能解密这些已确认要求。iroh 固定 v1.2.0 / `17c0612f80f78f5288e97b818b1360ae6ea0a51a`，p2panda 固定 v0.7.1；同时核对协议原文。

用户随后重申：旁信道不限于 MQTT，也包括 IRC 或其他产品支持的消息递送途径；保护要求不随渠道改变，外部转交者不能解密私密内容，但可观察必要元数据。下文 MQTT 是具体核查例子，候选接入应面向可适配的消息载体；使用 iroh 的代码和安全机制不要求使用官方服务器。具体渠道能否承载候选握手，要按消息大小、往返和生命周期等能力验证。

## 建议：调整建连顺序，优先只使用一套安全会话

**允许临时 MQTT topic 承载有界的 iroh QUIC/TLS 握手和连接控制报文，先取得端到端安全会话，再用其中的 QNT 协商打洞，转入获准的数据路径。** MQTT 仍只承担联络与连接控制，不承担普通记录补齐和业务通信。这把旧草案的「另建加密信令 → QUIC」调整为「临时消息载体上的 QUIC → 直接或获准中继路径」，符合内部改造 iroh、尽量少改 p2panda 的方向。

这是**有源码接入依据、尚待集成实验**的设计方向。不能声称上游已经提供 MQTT 适配，也不能声称一旦接上 `CustomTransport` 就自动兑现流量边界。用户已确认这个载体角色与顺序调整；密码套件、报文编码、退避参数应由后续工程核查处理，不再重新盘问已确认的产品原则。

## 源码已经支持什么

- iroh 的 `CustomTransport` / `CustomEndpoint` / `CustomSender` 接口在 QUIC 下方收发报文，并支持自定义本地、远端地址；接口由不稳定 feature 暴露。官方示例确实通过 custom transport 建立 iroh 连接和发送 stream。因此将 MQTT 消息封装成临时报文载体有具体接入点，不需要先自创另一套加密握手。[接口](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/transports/custom.rs#L24)、[示例](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/examples/custom-transport.rs#L73)
- `RemoteStateActor` 把每个新增 connection 接入 NAT traversal 事件流、填充候选并触发 QNT。触发逻辑选择现有 client connection，并没有要求初始路径必须是原生 relay；`initiate_nat_traversal_round` 在该 connection 上执行。选路也会在同一 connection 上打开所选路径。**由此推断** custom 起步后使用同一安全会话升级到 UDP 有合理依据；真实 MQTT 时延、丢包、消息上限和路径升级仍须测试。[新增连接与 QNT](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L385)、[QNT 触发](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L501)、[打开路径](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L675)
- TLS 使用节点 Ed25519 raw public key；客户端 verifier 检查预期目标公钥，已建立连接提供 `remote_id()`。保持原节点身份即可贯穿初始载体和后续数据路径，不必为 MQTT 再造一个与 iroh 无关的节点身份。[验证器](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/tls/verifier.rs#L33)、[连接身份](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L1129)
- `Connection::export_keying_material` 已暴露 TLS exporter，可供成熟的会话绑定机制接入。它只能在已经建立的 TLS 上使用，不能反向保护本次握手之前发送的消息；它本身也不证明对方持有网络准入秘密。[iroh exporter](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L1075)、[TLS 1.3 channel binding](https://www.rfc-editor.org/rfc/rfc9266.html#section-2)

## 三种信任起点仍然不同

| 情况 | 已有信任材料 | 安全会话内还需做什么 |
| --- | --- | --- |
| 首次入网 | 个人可信传递的邀请绑定目标网络与指定联系节点；不含长期网络准入秘密 | TLS 对上邀请指定节点；申请者使用自己的节点身份；按已定规则批准或核销后交付准入材料。另一邀请码持有者有能力发起自己的连接，但不因此取得本连接的会话密钥。 |
| 已知成员重连 | 保存的目标节点身份、本地已验证成员状态及当前准入材料 | 对上目标身份，遵守已知撤销；在获准范围内恢复连接与补齐，不把地址可达当作成员有效。 |
| 从未认识的合法成员恢复 | 本网信任依据和共同准入秘密；发现入口只给候选 | 候选自己的 TLS 密钥只能证明「这是这个节点」，不能证明它已入网；先完成准入秘密的持有证明，再按现有决定有界取得管理状态，交给成员授权逻辑判断正常同步和业务权限。 |

本表把已有 [CONTEXT](../../CONTEXT.md) 与 [准入秘密 ADR](../adr/0004-network-admission-secret.md) 映射到候选流程，**不是已存在的 iroh / p2panda 功能声明**。本研究没有选定未知成员发现封套或准入证明 wire protocol。尤其不能把准入秘密直接发送给尚未证明资格的陌生 TLS 对端；加密传输不能阻止这个对端自己读到秘密。应选择绑定本次安全会话的成熟持有证明机制，而不是临时拼接「随机数 + HMAC」后宣称完成安全设计。此机制的选择是实施前技术工作，不需要重问共享秘密与节点身份的产品分工。

## 隐私与资源边界需要落实在哪

1. **临时 topic 不是加密。** topic、回复路由、消息大小和时序由载体可见；随机名字也不保证无法关联。邀请凭据、准入秘密、设备名和详细协商不能放进明文路由封套。不得因使用临时 topic 就宣称匿名。
2. **QUIC Initial 不是私密数据容器。** 初始密钥可以由公开字段推导，ClientHello 内容可观察。核查到此固定版 iroh 明确 `enable_sni = false`，不会把用于本地目标验证的 synthetic server name / EndpointId 作为 SNI 发出去；仍需检查应用 ALPN 和自定义封套，避免自行加入稳定身份或秘密。[RFC 9001 初始密钥](https://www.rfc-editor.org/rfc/rfc9001.html#section-5.2)、[iroh 禁用 SNI](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/tls.rs#L91)
3. **只有握手完成不够。** 初始阶段只放行受限控制；准入批准、核销、秘密交付等有副作用的操作不能因为 0-RTT 数据到达就执行。iroh 固定版开启 early-data 能力，产品需要在握手确认和自己的准入检查之后处理这些操作。[iroh early data](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/tls.rs#L87)、[QUIC 0-RTT 重放边界](https://www.rfc-editor.org/rfc/rfc9001.html#section-9.2)
4. **信令载体资格必须是硬边界。** 现有 custom path 会像一般可用路径承载 stream；默认选路还把未选中的 custom path 保留为 backup。只给 MQTT 低评分或限制字节数，不能保证业务不经 MQTT。内部改造必须限制初始阶段开放的协议，并在数据阶段禁用它的业务资格；数据路径断开时不能自动把业务 stream 回落到 MQTT。可验证的简化候选是数据路径建立后退出初始 MQTT 路径；若要持续用于恢复控制，需要进一步的调度隔离设计。[默认保留与路径状态](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L675)、[custom stream 示例](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/examples/custom-transport.rs#L190)
5. **自动 QNT 需接准入时机。** 现有 actor 添加 connection 就发布本地候选并触发 QNT；若要在准入证明前限制候选披露或探测资源，需在 fork 内加相应门控。TLS 身份验证不应被误写成已经完成网络准入。[添加 connection](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L397)

上述控制、封套、消息上限、重试和生命周期都是 miopunch 的应用/载体集成职责；复用成熟 TLS/QUIC 不等于这套新增集成已经审计。

## 为什么不先叠加 p2panda-encryption 或 Noise

不能笼统说 p2panda-encryption「只有组加密」。v0.7.1 明确提供 pairwise `TwoParty`：首轮 X3DH、后续 HPKE，用于 DCGKA 的两方秘密分发；它有预密钥生命周期和顺序假设，并使用 X25519 身份材料。[2SM](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-encryption/src/two_party/two_party.rs#L20)、[密钥管理](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-encryption/src/key_manager.rs#L13)

但它不是 iroh EndpointId 认证过的现成 MQTT 建连协议。复用它仍需处理身份绑定、预密钥获取与验证、持久状态和消息顺序；该版本 README 明说尚未安全审计，组控制消息也默认不加密。因此目前不推荐为短暂的在线建连再引入整个组加密状态体系，更不能用同一组密钥保护所有点对点私密信令。[版本 README](https://github.com/p2panda/p2panda/blob/v0.7.1/p2panda-encryption/README.md#security)

只有在「临时消息载体承载初始 QUIC」被否决或实验不可行时，才重新选择独立的成熟握手。Noise 是具备认证握手、会话密钥与可选 PSK 模式的框架，但所选模式、长期密钥与 iroh Ed25519 身份绑定、消息顺序仍是集成责任；HPKE 是混合公钥加密标准，不独自定义完整双向建连和准入协议。不建议现在同时承诺两条路线。[Noise](https://noiseprotocol.org/noise.html#application-responsibilities)、[HPKE 范围](https://www.rfc-editor.org/rfc/rfc9180.html#section-1)

## 接入前验证项

- 两端仅有临时消息载体时完成 iroh 身份握手；错误目标身份失败；第三个持有同一邀请码的节点不能解密或冒充当前会话。
- 在同一个 connection 上完成 QNT 与获准数据路径建立；临时载体断开后继续使用该连接，不能把另拨新连接算成路径迁移。
- 抓取初始与重连报文：检查 topic、回复路由、ClientHello / ALPN 均无邀请秘密、准入秘密及不必要的稳定节点身份；私密字段只在受保护阶段出现。
- 业务数据排队、直连失败、所有数据路径失效时均不回落 MQTT；连接无法维持则明确失败。MQTT 延迟、重复、乱序、超时、消息大小上限和资源预算需实测。
- 未知节点不能靠拥有 TLS 密钥进入普通管理记录同步；持有证明必须绑定当前会话，重放和跨会话转交失败，且不得向无资格候选泄漏准入秘密。
- p2panda wrapper 仍需薄适配：现有缺地址早退、自动启动 Discovery / LogSync 的时机与受限引导阶段不能冲突；详见[现有职责审查](2026-09-28-integration-responsibility-audit.md)。

这些是后续工程验证项，不是本轮已经执行的测试，也不构成重新讨论产品原则的六张新票。
