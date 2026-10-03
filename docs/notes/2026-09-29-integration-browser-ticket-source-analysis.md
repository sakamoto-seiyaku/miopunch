# iroh / p2panda 组合与浏览器实施切片：固定源码核查

日期：2026-09-29。用途：为[新基线实施规格](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)拆票提供源码依据与可运行的验收边界。本文是研究与建议，不是新增产品决议、依赖锁定或已完成实验。未构建、运行原型或写入 tracker。

## 1. 证据版本与主要结论

- p2panda v0.7.1：固定提交 `083b48215964e92a564f3c83a1c607b00e94aa64`。本轮主代理从官方固定提交 codeload archive 验证 `/tmp/p2panda-0.7.1` 的 364 个普通文件，全部相同；archive SHA-256 为 `bf5cbf9d6bc03b420b0ffe060a75eff164fb5e11241e33bb91347889292803c5`。核查元数据在 `/tmp/miopunch-p2panda-source-verification.json`。
- iroh v1.2.0：`/tmp/miopunch-iroh-v1.2.0` 的 origin 是官方 `n0-computer/iroh`，干净 HEAD 为 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`，本轮对 `/tmp/iroh-1.2.0` 的 237 个普通文件逐个 SHA-256 比较，全部相同。目录名含 miopunch 不表示已有产品 fork 改动。该提交同时有 v1.2.0/v1.3.0 标签；所用 `iroh`、`iroh-base`、`iroh-relay` 清单版本是 1.2.0，`iroh-dns` 则为 1.3.0，不能把所有 workspace crate 一律记为同一个版本。[iroh 清单][i-manifest]
- 下文 **事实** 指固定源码直接支持的结论；**建议/推论** 指从这些事实提出的接法；**未验证** 指必须编译或运行才可确认的结果。上游存在测试源码不等于本轮运行通过。

**主要结论：**原生第一条通信链无需先泛化整个 p2panda。优先在其 endpoint 构造处加一个小的可配置入口，安装 fork 的配置及现有 `EndpointHooks`，保留地址簿发布与连接结果回报，再修正身份已知但地址未到时的提前失败。浏览器则有真实、可静态定位的构造、存储和 feature 障碍；不能声称只改 `default-features = false` 就能运行整个 `p2panda::Node`。[构造与提前失败][p-endpoint-actor] [高层组合][p-network] [feature 图][p-net-manifest]

## 2. 实际构造与依赖图

### 2.1 高层 Node 确实做了什么

**事实：**`p2panda::NodeBuilder::spawn()` 从 URL、pool 或内存选出具体 `SqliteStore`，创建 `OperationForge`、`Pipeline::new::<SqliteStore>`，然后调用内部 `Node::spawn_inner`。`Node` 的字段就是 `SqliteStore`，并非泛型 store。默认 signing key 每次生成；持久化 key 必须由产品加载并传给 `.signing_key(...)`。设置数据库 URL 不会自动持久化这个外部传入的节点 signing key。[NodeBuilder][p-node-builder] [Node 字段及启动][p-node]

`Network::spawn` 的顺序是：

```text
SqliteStore -> AddressBook
                 |
                 +-> 插入已配置 bootstrap 的 NodeInfo
                 +-> p2panda_net::Endpoint::builder(...).spawn()
                            |
                            +-> mDNS（可在运行配置中 Disabled）
                            +-> Discovery（自动启动）
                            +-> Gossip（自动启动）
                            +-> LogSync(store, endpoint, gossip)（自动启动）
```

`Network` 为 crate 内部类型，没有公开的替换网络对象入口。高层配置公开了 key、network ID、relay URL、bootstrap、mDNS mode、socket 和 discovery/gossip 参数，但没有接受已建好 iroh endpoint、endpoint builder callback、准入策略或任意 store 的方法。[高层 Network 全链][p-network] [NodeBuilder 接口][p-node-builder]

**建议：**第一条原生切片同时比较两个有界候选：在现有高层 Node 的构造链传递一个 endpoint 配置入口；或者直接组合 `p2panda-net` 的现有组件。选择以最少改动通过真实通信和摄取为准。源码已显示高层不是无缝可插拔入口，但这不构成“必须重写高层 Node”的理由。授权 CRDT/空间处理器作为依赖出现，也不表示 `Node` 已实现本产品的创世与日常管理员规则。

### 2.2 Cargo feature 的真实边

| 入口 | 静态启用关系 | 对拆票的影响 |
| --- | --- | --- |
| `p2panda` 自身 `default=[]` | 仍显式为 net、store、stream、sync 开启 `default-features=true` | 在应用依赖里关闭 `p2panda` 默认 feature 不会关闭这些子依赖默认项 |
| `p2panda-net/iroh_endpoint` | `address_book` → `p2panda-store/sqlite` 与 `macros` | 仅选 endpoint feature 仍拉入具体 SQLite 地址簿 |
| `p2panda-net/sync` | endpoint + address_book + gossip + p2panda-sync | 当前 LogSync 的自动选邻居实际依赖 Gossip；Discovery/mDNS 不是该 feature 的强制边 |
| `p2panda-sync` 自身 `default=[]` | store 依赖仍为 `default-features=true` | 使用底层同步协议也需要清理 store feature 边，不能靠调用处关闭默认项解决 |
| `p2panda-stream` | 默认 groups、ingest、log_prune、orderer、spaces，store 默认开启 | 可按实际处理器缩小所用 feature，但 store 的默认项仍需单独检查 |
| `p2panda-store` | 默认 SQLite + macros；SQLite → SQLx 0.8.6 `runtime-tokio,sqlite` | traits 可复用，现有 high-level 地址簿/Node 并不会自动使用自定义后端 |

依据：[p2panda Cargo][p-node-manifest]、[net Cargo][p-net-manifest]、[sync Cargo][p-sync-manifest]、[stream Cargo][p-stream-manifest]、[store Cargo][p-store-manifest]。

**事实：**net 对 iroh 的要求写为 `1.0.3`，不是 `=1.0.3`，关闭 iroh 默认 features、显式开 `tls-ring`；另直接依赖 `iroh-base 1.0.3`、`iroh-gossip 0.101.0`、`iroh-mdns-address-lookup 0.5.0`。p2panda 的最低 Rust 版本写为 1.96；iroh 为 1.91。[net 清单][p-net-manifest] [iroh 清单][i-manifest]

**建议/未验证：**组合验证应固定 fork commit、Cargo.lock、target 与实际 feature 图；用 root workspace 的 patch 覆盖所有需要共享类型的 iroh workspace crate，确认 `iroh::Endpoint` 和 `iroh-base::EndpointAddr/PublicKey` 不形成不同来源的同名类型。不要仅新增一个应用直接依赖就假定 gossip/mDNS 已使用 fork。半兼容版本范围不能证明 custom transport 的不稳定 API、线协议或所有上游版本互通。实际解析后的 `cargo tree -e features`、重复版本与目标构建结果属于接入票产物，而非现在已有结果。

## 3. 最小 native 接入点：构造、身份拨号与结果回流

### 3.1 Endpoint 构造的窄入口

**事实：**`p2panda-net::Endpoint::builder(address_book)` 保存 network ID、signing key、`IrohConfig`、relay 配置。真正绑定发生在 `IrohEndpoint` actor 的 `Bind` 分支：

```rust
iroh::Endpoint::builder(presets::Minimal)
    .relay_mode(iroh::RelayMode::Custom(relay_map))
    .address_lookup(AddressBookDiscovery::new(signing_key, address_book))
    .secret_key(from_signing_key(...))
    .transport_config(...)
    .bind_addr(ipv4)?
    .bind_addr(ipv6)?
    .bind().await
```

p2panda API 的 `.endpoint().await` 只返回已绑定 endpoint 的 clone；无法借此补装 builder-only 的 `.hooks(...)`、`.add_custom_transport(...)`、`.path_selector(...)`。后两个需要 iroh feature `unstable-custom-transports`，并明确不在 semver 保证内。[net Builder][p-endpoint-builder] [Bind 分支][p-endpoint-actor] [已绑定 endpoint getter][p-endpoint-api] [iroh transport builder][i-transport-builder]

**建议：**在 `Bind` 前提供一个可重用的 builder 配置/工厂 seam；先只用于安装 fork 策略、hooks、测试所需 custom transport 和平台绑定差异。保留同一 signing key、`AddressBookDiscovery`、关闭/accept 所有权。不要同时给 p2panda 的 accept loop 与另一个 iroh Router 消费同一 endpoint 的 incoming queue。若使用 supervisor，`on_start` 会 clone 启动参数重新创建 actor，配置 seam 必须能在重启后重用；单次消耗的 builder 或旧 endpoint clone 不应被误当重建机制。[actor 启动与关闭][p-endpoint-actor] [supervisor 重建][p-endpoint-supervisor]

### 3.2 身份已知、地址尚未知的失败发生在哪

**事实：**`ToIrohEndpoint::Connect` 在调用 iroh 前查询 `address_book.node_info(node_id)`；没有 NodeInfo，或转换不到 `TransportAddress::Iroh`，立即回复 `TransportInfoMissing`。因此 fork 即使具备按身份触发旁信道的能力，也可能完全没有被调用。[Connect 分支][p-endpoint-connect] [NodeInfo 转换][p-nodeinfo-convert]

限制的精确范围是“缺 NodeInfo/缺 Iroh variant”，并非该转换检查 variant 内必有非空地址。`EndpointAddr::new(id)` 可无地址，转换会复制所存 variant；这与完全缺失 NodeInfo 不同。不要用虚构一个可信地址记录作为长期解决办法。[NodeInfo 转换][p-nodeinfo-convert]

iroh 自身 `connect` 接受 `EndpointId` 或 `EndpointAddr`，地址为空时可调用配置的 AddressLookup；无地址又无法解析才失败。p2panda 的 `AddressBookDiscovery::resolve` 已订阅特定 ID 的地址簿变化，缺记录/缺 transport 时等待；stale 时发错误。它没有搜索未知成员身份的能力。[iroh connect][i-connect] [AddressBookDiscovery][p-addresslookup]

**建议：**在正常 store 错误仍返回错误的前提下，缺地址时向 fork 传 `EndpointAddr::new(from_verifying_key(node_id))`；仅保留确有意义的拒绝条件。真实验收必须包括：A 已知 B 身份但无地址；候选延迟到达；取消；无可用候选超时；不重复创建产品/同步/fork 三层拨号循环。

### 3.3 key 转换和结果观测

**事实：**net 的 `NodeId` 是 p2panda `VerifyingKey`；转换是 32 字节的 `VerifyingKey::from_bytes` / `iroh_base::PublicKey::from_bytes`，signing key 直接 `iroh_base::SecretKey::from_bytes(key.as_bytes())`。这支持一份节点身份同时用于签名记录和 TLS 身份，无需第二个节点 ID。库使用不同 ed25519-dalek/rand 版本不改变转换方式，但应让实际组合验证这个边界。[key 转换][p-key-conversion] [core 依赖][p-core-manifest] [iroh-base 依赖][i-base-manifest]

连接 actor 对拨号成功/失败向 endpoint actor 报告；endpoint 将结果写入地址簿，但自身不具全局可达地址时忽略主动拨号失败。actor 在连接交给调用方后终止，actor 生存期不等于连接生存期；incoming 也可能由 handler 将 Connection 移走。[连接 actor][p-connection-actor] [report 过滤][p-endpoint-report]

**建议：**首个可运行切片需断言恢复的 p2panda key、iroh `Endpoint::id()`、对端 `Connection::remote_id()` 对应同一身份；成功后地址簿可观察到结果。区分“策略/准入拒绝”和“联系路径失败”，避免 hooks 的 `LocallyRejected` 被无差别计为网络失败、使有效节点错误变 stale。撤销已有连接需独立保存连接弱引用或等价的生命周期索引；不可依赖已经退出的连接 actor。

## 4. ALPN、准入和协议启动：现有 hooks 比全面改 handler 更窄

### 4.1 实际控制流

| 协议/路径 | 出站 | 入站 | 未做产品准入时的行为 |
| --- | --- | --- | --- |
| Discovery | `Endpoint::connect_with_config`，连上就 `open_bi` 跑 PSI discovery | net ProtocolMap → handler → session | 传递共同 topic 和 transport info，不校验本产品成员角色 |
| LogSync | `Endpoint::connect`，随后 topic handshake，再第二条 stream 跑 sync | net ProtocolMap → `SyncProtocolHandler` 先 topic handshake | topic 订阅过滤不能替代准入秘密与成员授权 |
| Gossip | `IrohGossip::builder().spawn(endpoint.endpoint().await?)`，直接持底层 endpoint | 注册 iroh-gossip handler 到 net ProtocolMap | 仅改 net `Endpoint::connect` 覆盖不到 gossip 的底层拨号 |

依据：[Discovery session][p-discovery-session]、[Sync session][p-sync-session]、[Sync handler][p-sync-handler]、[Gossip 构造][p-gossip-manager]。LogSync 在当前组合中用派生的 gossip topic 做 HyParView 邻居采样；`Joined`/`NeighbourUp` 驱动 sync，故 gate 需要同时覆盖采样与同步路径。[sync membership task][p-sync-membership]

**事实：**net 自己维护 ProtocolMap：注册时用 `BLAKE3(protocol_id || network_id)` 形成 ALPN，调用底层 `.set_alpns`；接受时先确认 ALPN，再调用 `on_accepting`、报告成功、调用 `accept`。NetworkId 明确不是秘密；ALPN 分区、已知 topic、签名 transport info 都不能充当共享秘密持有或当前成员资格。[ALPN 定义][p-network-id] [router 调度][p-connection-actor]

### 4.2 可优先验证的现成 seam

**事实：**iroh 1.2.0 已有 `Builder::hooks(impl EndpointHooks)`：

- `before_connect(&EndpointAddr, alpn)` 在 `connect` / `connect_with_opts` 内、发起连接前运行，返回 Accept/Reject。
- `after_handshake(&Connection)` 对 incoming/outgoing 都运行，可访问已认证 remote ID、ALPN，拒绝会关闭连接。
- 普通 await 路径在 hook 通过后才返回 `Connection`，因此将同一策略 hook 安装到实际共享的底层 endpoint，有机会同时覆盖 p2panda 自己与 gossip 发起的连接。
- hooks 不应持有强 `Endpoint` 造成引用循环；要追踪 connection 用 `weak_handle()`。持久授权视图应由产品层提供给 hook，连接引擎无需硬编码管理员规则。

依据：[EndpointHooks 完整接口][i-hooks]、[before_connect 调用位置][i-before-hook]、[after_handshake 调用位置][i-after-hook]。

**建议：**首先验证这个 seam；不要先给 Discovery、Gossip、LogSync 每个模块各写一套授权入口。若准入采用单独控制 ALPN，控制流程和后续每条正常连接的绑定不能仅靠“这个 ID 以前验证过”完成。若选择当前连接的证明序言，应证明双方 hook 的 stream 次序、角色、超时、取消和错误关闭不会与协议首条 stream 冲突。最终接法依赖成熟秘密持有协议的选择，不在本文拼接密码原语。

**事实与局限：**`Connection::export_keying_material` 给出 TLS 会话相关材料；这是选择成熟会话绑定机制的可用接口，不等于完整持有证明。上游 `auth-hook.rs` 示例证明 hooks 可以包住无感知的协议，但它用另一个连接认证并缓存 endpoint ID，不能直接作为本产品要求的双向、当前连接绑定、轮换、撤销与重放防护实现。[exporter][i-exporter] [auth hook 示例][i-auth-example]

必须保留两项边界：

1. `conn_from_noq_conn` **先**把连接注册到 socket，**再**等待 `after_handshake`；所以应用 handler 的 gate 不证明底层 QNT/地址协商在准入前完全静默。控制报文的允许范围和 signal-only 路径限制须在 fork 相应位置落实。[hook 顺序][i-after-hook]
2. `into_0rtt` 可先给出弱安全的连接对象，hook 在后续 `handshake_completed` 路径才经过；服务端 0.5-RTT 数据也可能在客户端身份确认前发出。默认 `ProtocolHandler::on_accepting` 只是 await，当前 p2panda 拨号也是普通 await，但新增控制入口必须覆盖自定义 early data 路径；不可让申请核销、秘密交付、管理历史或正常业务从此绕过准入。[0-RTT 实现及安全说明][i-0rtt] [handler 默认实现][i-handler]

**建议验收矩阵：**无秘密、错误秘密、已有秘密但无有效成员授权、已知撤销 ID、新身份旧秘密持有者、合格成员分别尝试控制入口、Discovery、Gossip、LogSync 和测试业务；无数据路径时正常协议不得借 signal-only 载体继续。首次申请者尚无长期秘密，须能通过可信邀请进入单独的受限申请范围。轮换/撤销后再测已有连接和新连接。准入成功不意味着普通成员自动得到管理权限。

## 5. 未知候选与默认隐私：不能把“按 ID 寻址”写成“发现新成员”

**事实：**RandomWalker 从本地 bootstrap 或其它已知节点选起点；Discovery session 先向这个 ID 建立连接，才交换 topic/地址信息。地址簿为空时 walker 延迟重试，等待外部过程增加节点。已有在线成员可作为发现更多节点的桥，但 A 只知道离线 B、C 是唯一在线陌生成员的场景，没有由随机游走凭空产生 C 的路径。[random walk][p-random-walk] [空地址簿分支][p-discovery-walker] [先 connect 后发现][p-discovery-session]

`AddressBookDiscovery::publish` 把本机 endpoint 地址签名保存到本地地址簿，`resolve(id)` 订阅该 ID。PSI discovery 可分享有共同 topic 的节点并总包含自身 transport info，其保护对象与成员授权不同。p2panda 的签名 transport info 证明地址声明来源，不证明该节点属于产品网络。[地址簿 publisher/resolver][p-addresslookup] [PSI transport gathering][p-psi-gather]

默认值需要按层区分：

- iroh `presets::Minimal` 只选择 crypto provider；`N0` 才添加公共 pkarr/DNS lookup 与默认 relay。p2panda 用 Minimal + 自有地址簿 lookup，relay map 默认空，不会因“用 iroh”就自动具备官方公共引导。[iroh presets][i-presets] [Node 默认网络配置][p-network-defaults]
- 高层 Node 的 mDNS 默认 **Active**，公开局域网地址/公钥。低层 `iroh_mdns::MdnsDiscoveryMode` 默认 **Passive**，但此版本 actor 的 `Initialise` 在 `!mode.is_active()` 时直接 return，实际不会建立订阅；不可按文档字面声称 passive 会监听。现有 mDNS 测试显式开 Active，没有证明 passive 的宣称行为。[高层默认][p-network-defaults] [低层模式][p-mdns-config] [actor 分支][p-mdns-actor] [mDNS 测试][p-mdns-tests]

**建议：**未知候选票应形成最窄完整链：A 没有 C 的 ID，B 不在线；A/C 经一条产品允许的真实消息入口取得候选 ID 和建立受保护连接所需线索；候选未获信任时不释放管理历史；按选定持有证明进入允许的引导，再由现有授权记录决定正常同步/业务。假消息入口必须能提供假候选、重复候选、无秘密节点与已知撤销 ID。消息 topic/目录条目只是路由线索，发现材料如何隐藏、限流及更新需明确。该票依赖准入接口和真实消息入口能力，不必等待所有邀请 UX、迁移或全部历史同步完成。

## 6. 浏览器 relay-only：已有能力、确定缺口、真正需要的 spike

### 6.1 iroh 已回答的部分

**事实：**iroh 把 `wasm_browser` 定义为 `all(target_family="wasm", target_os="unknown")`；`bind_addr` 在该 target 不存在，浏览器 `watch_addr` 只给 relay 地址。relay client 的浏览器分支用 `ws_stream_wasm::WsMeta::connect`，把 HTTP(S) relay URL 转为 WS(S)，没有先引入 WebRTC 的要求。[cfg 定义][i-build] [bind_addr][i-bind] [browser watch_addr][i-browser-addr] [browser relay client][i-browser-relay]

官方浏览器说明要求关闭 iroh 默认 features，并明确当前浏览器连接经 relay，端到端内容仍加密。使用 `Minimal` 时仍需选择 crypto provider；因此候选配置是关闭默认项、保留实际所需 `tls-ring`，随后验证最终 feature unification。首期只需浏览器可访问的获准公共或自建 relay，不依赖浏览器直连、WebRTC 或消息载体上的 custom packet transport 先完成。[官方浏览器能力说明](https://docs.iroh.computer/languages/wasm-browser) [Minimal crypto provider][i-presets]

**事实：**`iroh/tests/integration.rs::simple_endpoint_id_based_connection_transfer` 已提供 ID 拨号和 echo 的 wasm/native 测试形状，但使用真实 staging relay、pkarr 服务；`run_in_browser` 配置在源码中被注释。它不能被称作本次产品在真实浏览器通过，也不应把无控制的公共设施依赖复制成唯一验收路径。[固定测试源码][i-browser-test]

relay token 是基础设施凭据：native 走 Authorization header，browser 走 URL query；不要将产品网络准入秘密拿来当 relay token，亦不可把接入 relay 视为成员授权。`Endpoint::online()` 仅代表至少一个 relay 完成注册，且无配置 relay 时可一直 pending；它不是合法远端连接验证。[relay token 分支][i-relay-token] [online 语义][i-browser-addr]

### 6.2 整个 p2panda 组合并非“再试一下 feature”

| 静态已定位的障碍 | 当前可以下的结论 | 尚未证明的部分 |
| --- | --- | --- |
| net `Bind` 无 cfg 调用两个 `.bind_addr(...)`；iroh 浏览器无该方法 | 固定代码直接套到浏览器存在明确 API 编译缺口 | 修改/绕过后其余目标构建是否通过 |
| net endpoint feature 强制 SQLite 地址簿；高层 Node/store 固定类型 | 插入 IndexedDB 对象不能自动替代当前地址簿或 Node | 应复用多少 net actor、高层 API，还是更薄地组合库 |
| sync/stream 显式启用 store 默认项 | 调用方 `default-features=false` 不足以去掉 SQLx | 最小 feature patch 能否保留所需底层协议/处理器 |
| SQLx 0.8.6 SQLite 每连接启动 `std::thread::Builder::spawn` | 标准 `wasm32-unknown-unknown` 的原生 SQLite worker 不是可直接工作的浏览器存储适配 | 选择 browser storage adapter 还是另一个有证据的 SQLite/Wasm 组合 |
| net actor 广泛使用 `ractor::thread_local::ThreadLocalActorSpawner` | 调度入口需要目标平台核查；名称中的 thread_local 不等于浏览器可用 | ractor 实际解析版本、线程/Send/Future 要求如何适配 |

依据：[net Bind][p-endpoint-actor]、[iroh cfg][i-bind]、[具体地址簿 store][p-addressbook-builder]、[SQLx 0.8.6 worker 源码](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-sqlite/src/connection/worker.rs#L24-L28)、[worker 的 thread spawn](https://github.com/launchbadge/sqlx/blob/v0.8.6/sqlx-sqlite/src/connection/worker.rs#L96-L104)、[Rust 标准 wasm target 限制](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)。这是源码推导的障碍集合，并非已得到的编译错误日志。

**建议：**把 browser gate 设为独立且较早的窄切片：先用 fork iroh + p2panda-core 身份/记录 + 所需授权验证代码跑真实 relay-only 页面对 native 节点；再接可持久化的记录同步和浏览器后端。若选择底层 p2panda-sync/stream，需要清除上表 feature 牵连。已有 store traits 说明有适配依据，但应围绕“该浏览器能保存并再次提供一条已验证记录”实现刚好需要的操作，不先泛化所有 store、net actor 和客户端框架。[OperationStore traits][p-store-traits] [LogSync 泛型 store 要求][p-logsync-builder]

浏览器持久化需覆盖节点 signing key、网络授权/准入秘密、已验证控制记录及其依赖和完成标记。first-run 生成身份、reload 恢复原身份、关闭/冻结后重建连接、记录提交中断和存储材料丢失应有可观察结果。浏览器独立持有身份并验证授权，网页分发服务器不代理成这个节点。确定一个页面 origin 的材料作用域及并发页面写入策略，并验证所选存储 API 的提交结果；不把页面 unload 的 best-effort 回调当 durable commit。具体 IndexedDB/OPFS/SQLite-Wasm 技术选择仍是该切片产物，本文未验证或选择。

## 7. 可拆为端到端票的最小集合与真实依赖

以下是建议的票边界，不要求逐条照搬为独立 ticket；同一条能够交付可运行行为的工作可以合并。特别不要增加一个先完成“所有平台通用网络/存储抽象”的前置大票。

| 建议切片 | 交付的外部行为 | 最小内部改动/关键验证 | 真实阻塞依赖 |
| --- | --- | --- | --- |
| N1：固定版本组合下两原生身份连通 | A/B 用持久 key 经显式 relay/地址交换测试流；重启身份相同 | 固定依赖图；net constructor 配置入口；key byte conversion；现有结果回流。先不上未知候选或新密码机制 | 仓库最小 Rust 测试入口；选定依赖构建实际通过 |
| N2：已知身份无地址仍启动连接底座 | A 只有 B ID；fork 经配置的解析/候选输入取得线索并连通；无结果按期失败 | 去掉缺地址提前失败；保留 store 错误；取消/延迟线索；禁止重复拨号循环 | N1；实际可用的解析/候选提供能力。完整 MQTT 安全控制升级不必为一个地址 lookup 验证所阻塞 |
| A1：所有正常协议受统一准入控制 | 同一测试入口证明 Discovery/Gossip/LogSync/测试流的允许与拒绝矩阵 | 安装 EndpointHooks；实际 raw gossip 路径；受限申请例外；正常 connection/0-RTT；拒绝分类；已有连接撤销 | N1；成熟持有证明选择及其会话绑定验证。可提前验证 hook 接线，但不得把替身准入当完整安全完成 |
| D1：旧联系人离线、未知合法成员恢复 | A 不知 C，B 离线，经真实获准入口发现并验证 C，再补齐一条记录 | 最小候选封套/输入；身份与秘密门槛；限制历史读取；假候选和重放；复用地址簿并触发调度 | A1 的必要引导门槛；一条真实候选入口；记录验证/保存最小能力。没有理由等待迁移/全邀请 UX |
| B1：浏览器独立身份 relay-only 授权通信及重载 | 真实页面与 native 节点用相同授权语义通信；reload 身份/授权保留；无 relay 明确失败 | 最小 wasm wrapper；浏览器 target feature 收敛；持久 key/材料；协议 gate；存储适配方向有实际验证 | N1 的身份/协议契约；授权 gate。可先并行验证 iroh-only+core 构建；不依赖 WebRTC、MQTT custom transport 或全部 native UI |
| B2：浏览器作为持久控制记录副本 | 保存一条作者记录与依据，页面重启后仍可向第三节点供给，原作者离线 | 必要 store traits/feature patch 或最小 adapter；commit/完成标记；真实同步和重复记录 | B1；跨节点摄取/持久化契约。是否与 B1 合并取决于最终票大小，不应只验 echo 就宣称浏览器产品完成 |

另有必需但属于 fork 连接层的 ticket：真实消息载体承载 iroh 握手/受限控制、取得数据路径并保留同一 Connection；业务排队与数据路径全失效时不能落入 signal-only 通道。A1 的 handler gate 与该票的 packet/path 用途限制互补，彼此不能代替。浏览器初期 relay-only 是另一条已允许路径，无需等待这条 custom transport 路径实现才开展 B1。

**需要运行而静态阅读不能替代的检查：**所选版本/feature 的 native 与 wasm 编译；p2panda-net + fork + iroh-gossip 的真实 gate 覆盖；秘密证明与首次申请的 stream 时序、会话绑定和 early data 行为；候选延迟/取消/拒绝后的调度；真实浏览器 relay TLS/WebSocket、暂停/重载、durable storage 和授权记录验证；同一连接换路与 signal-only 严格隔离。

**静态阅读已经回答、不值得另开泛研究票的事实：**Node/store 是具体 SQLite；net 缺地址有提前失败；Gossip 获得原始 iroh endpoint；NetworkId 不是秘密；hooks 存在；浏览器 iroh 无 `bind_addr`、现行数据路径经 relay；高层默认 mDNS Active 而非低层的 Passive。源码测试可作为修改起点：`iroh_endpoint::tests::establish_connection`、`protocol::tests::test_limiter_hook`、`iroh/tests/integration.rs::simple_endpoint_id_based_connection_transfer`。这些测试没有覆盖产品准入、未知成员恢复、消息载体升级或浏览器持久化，不能以复跑它们代替产品验收。[net echo 测试][p-net-test] [hook 测试][i-hook-test] [wasm 测试][i-browser-test]

[p-node-builder]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/src/builder.rs#L21-L244
[p-node]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/src/node.rs#L23-L85
[p-network]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/src/network.rs#L24-L95
[p-network-defaults]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/src/network.rs#L123-L172
[p-node-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda/Cargo.toml#L15-L43
[p-net-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/Cargo.toml#L15-L93
[p-store-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-store/Cargo.toml#L24-L64
[p-sync-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-sync/Cargo.toml#L24-L44
[p-stream-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-stream/Cargo.toml#L24-L55
[p-core-manifest]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-core/Cargo.toml#L24-L46
[p-endpoint-builder]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/builder.rs#L14-L106
[p-endpoint-actor]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L119-L241
[p-endpoint-connect]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L243-L291
[p-endpoint-report]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L266-L350
[p-endpoint-api]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/api.rs#L105-L184
[p-endpoint-supervisor]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/supervisor.rs#L10-L35
[p-nodeinfo-convert]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/addrs.rs#L117-L135
[p-addresslookup]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/discovery.rs#L53-L172
[p-key-conversion]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/utils.rs#L115-L128
[p-connection-actor]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/actors/connection.rs#L88-L207
[p-network-id]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/lib.rs#L280-L325
[p-discovery-session]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/discovery/actors/session.rs#L96-L143
[p-sync-session]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/sync/actors/session.rs#L80-L134
[p-sync-handler]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/sync/actors/manager.rs#L512-L553
[p-sync-membership]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/sync/actors/manager.rs#L130-L205
[p-gossip-manager]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/gossip/actors/manager.rs#L128-L184
[p-random-walk]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-discovery/src/random_walk.rs#L92-L158
[p-discovery-walker]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/discovery/actors/walker.rs#L140-L189
[p-psi-gather]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-discovery/src/psi_hash.rs#L108-L152
[p-mdns-config]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_mdns/config.rs#L5-L35
[p-mdns-actor]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_mdns/actor.rs#L97-L118
[p-mdns-tests]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_mdns/tests.rs#L10-L87
[p-addressbook-builder]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/address_book/builder.rs#L9-L37
[p-store-traits]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-store/src/operations/traits.rs#L7-L60
[p-logsync-builder]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/sync/log_sync/builder.rs#L17-L66
[p-net-test]: https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net/src/iroh_endpoint/tests.rs#L75-L130
[i-manifest]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/Cargo.toml#L1-L164
[i-base-manifest]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-base/Cargo.toml#L17-L54
[i-transport-builder]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L773-L850
[i-connect]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1037-L1079
[i-hooks]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/hooks.rs#L51-L107
[i-before-hook]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1100-L1138
[i-after-hook]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L306-L365
[i-exporter]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L1067-L1092
[i-auth-example]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/examples/auth-hook.rs#L175-L324
[i-0rtt]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs#L497-L644
[i-handler]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/protocol.rs#L228-L276
[i-presets]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/presets.rs#L45-L139
[i-build]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/build.rs#L1-L10
[i-bind]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L350-L375
[i-browser-addr]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint.rs#L1297-L1325
[i-relay-token]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-relay/src/client.rs#L234-L244
[i-browser-relay]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh-relay/src/client.rs#L387-L427
[i-browser-test]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/tests/integration.rs#L1-L137
[i-hook-test]: https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/protocol.rs#L713-L756
