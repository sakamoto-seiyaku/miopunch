# SSB、EBT 与 local-first 组件复用调查

调查日期：2026-09-27。

状态：研究笔记，不是协议定稿或依赖选型。本文补充[重新定位临时记录](2026-09-27-network-repositioning-discussion-draft.md)，不改变其中已经澄清的方向。

方法：阅读会议材料，追溯官方协议、作者文章、固定提交源码与版本元数据。未安装运行这些候选栈，未进行中国大陆真实网络、浏览器互通或性能实验。下文将事实与适用性推论分开。

后续澄清：用户已允许普通冲突采用成熟组件的自动处理规则，不再要求一律人工裁决；设备身份迁移暂不设计，但保留创世凭证恢复管理能力的要求。因此下文早期“冲突交用户”的比较条件不再是硬约束。TCP、伪装、普通节点中继及 TUN/代理共同 ACL 的新比较见[传输适配调查](2026-09-27-transport-fit-research.md)；控制数据模块不再与整个网络栈捆绑选型。

最新衔接：用户再次提醒，本调查的持久记录、第三方转交与离线补齐应继续构成产品主线，不能在首次联络讨论中收窄为“最小通讯录”。该限制已撤回，当前整理见[产品主线第 4.1 节](2026-09-27-product-mainline-draft.md#41-本地网络状态与持久同步)。这不新增全量复制或无限历史要求，也不代表已决定采用 EBT 或 p2panda 的全部实现。

## 1. 找到了哪个参考项目

用户给出的 iroh 演讲所在博客，同一目录中另有 2026-09-23 的[《Scuttlebutt 的开发历程》](https://bd.ailishi.ai/blog/scuttlebutt-development/index.html)。其中明确介绍了 EBT。

这里的 EBT 是 **Epidemic Broadcast Trees**。在 SSB 语境中，它是复制有序 feed、追补缺失记录、减少重复传输的机制。会话开始于两端已经建立认证连接之后。[SSB EBT 指南](https://ssbc.github.io/scuttlebutt-protocol-guide/#ebt-replication)

会议材料适合了解设计动机；具体保证以规范、源码和实际版本为准。不能把演讲中的“会自动同步”“找最快的节点”“不依赖服务器”等概括直接变成产品承诺。

## 2. 对当前讨论最重要的结论

**已有很多可以学习和评估复用的组件，暂时没有理由从零设计一套广播树、日志格式或密码握手。** 但需要分别评估连接、在线广播、持久补齐和授权，不能用一个组件的名字替代完整保证。

| 我们的问题 | 最相关的现有参考 | 尚不能由它包办的部分 |
| --- | --- | --- |
| 按身份联系节点、穿透与路径切换 | iroh | 大陆实际网络表现、我们的平台与部署边界 |
| 在线节点之间快速传播消息 | iroh-gossip / Plumtree | 永久保存、长时间离线后的补齐、消息作者授权 |
| 已保存记录经第三方传播、断线后补齐 | SSB EBT、p2panda 的日志与同步模块 | 操作的权限含义、冲突处理和历史保留策略 |
| 邀请由指定节点处理 | ssb-invite、SSB Rooms 2 HTTP invite | 我们的网络 ACL、多渠道证明和重试结果语义 |
| 去中心化成员授权 | p2panda-auth；Willow/Meadowcap 作为不同模型的对照 | sudo 式创世验证、用户裁决冲突等具体约定 |

这张表是能力映射，不是建议同时引入全部项目。各行依据见后文。

## 3. EBT 到底怎样帮助我们

### 3.1 数据模型先让同步变简单

经典 SSB feed 是单作者追加写入的签名日志，带连续序号和前一条记录的哈希。因此一个节点可以说明“作者 A 的记录我已经连续收到第 42 条”，另一个副本提供第 43 条以后的内容。[Feeds and Messages 规范](https://spec.scuttlebutt.nz/feed/messages.html)

记录的来源可以在转交后验证，不要求每次都联系原作者。映射到我们的场景：A 产生一份有效撤销记录，B 保存后 A 离线；C 以后只联系 B，也有机会取得并验证这份记录。前提是 B 实际保存了完整的必要记录，且 C 与 B 能够建立通信。这是适用性推论，不是网络无条件可达保证。

### 3.2 广播树与持久日志复制要区分

原始 Plumtree 用 eager 链路传正文、lazy 链路传简短提示，减少冗余并在漏消息时修复传播树。SSB 的 EBT 只是受其启发的适配，进一步处理 feed 进度、历史补齐和重连摘要。[Plumtree 作者论文](https://asc.di.fct.unl.pt/~jleitao/pdf/srds07-leitao.pdf)、[EBT 与 Plumtree 的区别](https://github.com/ssbc/epidemic-broadcast-trees#comparison-to-plumtree)

在 SSB EBT 中，接收来源按 feed 选择。不同 feed 可以从不同邻居获取；来源没有及时推进时可以换源。它不要求整台设备只连接唯一的“最快节点”，也不等于 QUIC 会话换路。[Vector Clock Partitioning](https://ssbc.github.io/scuttlebutt-protocol-guide/#vector-clock-partitioning)、[固定提交的来源选择逻辑](https://github.com/ssbc/epidemic-broadcast-trees/blob/942db6ebab378ba538a87b293f0aab7ffd33695f/events.js)

### 3.3 EBT 不替我们定义权限

EBT 中的 sequence 表示复制进度，不是 ACL 版本、全网操作顺序或投票结果。`request`、`block` 控制 feed 的复制范围，其调用策略仍由宿主决定。[ssb-ebt 使用说明](https://github.com/ssbc/ssb-ebt#usage)

推论：它可以搬运已经形成的授权和撤销依据，不能自动判断我们的 sudo 操作是否有效，也不能靠接收顺序裁决管理员之间的冲突。precode 的次数仍由已约定的指定节点核销，复制结果不会把扣次权分给其他副本。

## 4. 多端管理必须注意的适配边界

经典 SSB 不支持多个独立写入设备同时向同一条 feed 追加。私钥相同也不能保证两台设备生成相同的下一条记录。[消息规范](https://spec.scuttlebutt.nz/feed/messages.html)

签名证明消息由相应密钥签出，不保证持钥者从未签出另一份历史。两个副本都有“第 42 条”，内容仍可能位于不同分支；仅比较长度不能检测全部分叉。[作者关于分叉的说明](https://github.com/ssbc/ssb-db/issues/157#issuecomment-282100060)

**对我们最重要的推论：操作的创世授权，与保存记录的 writer/sequence 不是一回事。** 如果选择单写者日志，不能让不同管理员把所有 sudo 操作同时追加到一条共用根密钥的 feed。可以研究由各独立写入者记录操作、操作内另带授权依据的映射；这不要求改变用户已经确定的权限模型，也没有新增超级管理员角色。

核心 JS EBT 通过 `getAt`、`append`、`getClock`、`setClock` 等接口依赖宿主存储。数据库与本地进度恢复仍需集成。它的 `createStream(remoteId)` 会替换同一 remote ID 的旧 stream，因此不天然等于 MQTT、WebRTC、直连同时无序投递的统一会话。[核心库固定源码](https://github.com/ssbc/epidemic-broadcast-trees/blob/942db6ebab378ba538a87b293f0aab7ffd33695f/index.js)

这与“新增渠道不改变信任规则”相容，但还需要处理渠道适配。直接采用成熟状态机，与自行用其思想重写一个协议，是不同的工程投入。

## 5. iroh 已经有广播树，但广播不等于补齐历史

`iroh-gossip 0.101.0` 明确基于 HyParView 与 Plumtree。其 `proto` 是无 I/O 的状态机，`net` 是运行在 iroh 连接上的适配层。这个分离本身就是值得学习的现成设计。[固定版本 API](https://docs.rs/iroh-gossip/0.101.0/iroh_gossip/)

核查提交 `f128b987` 的 Plumtree 实现：正文缓存默认保留 30 秒，消息 ID 默认保留 90 秒；`delivered_from` 是最后一跳邻居，载荷 hash 校验不是原作者签名验证。它没有因此提供长期日志与作者级授权。[固定提交源码](https://github.com/n0-computer/iroh-gossip/blob/f128b98705f5ad3a28a92f1d2b483686348b6ae8/src/proto/plumtree.rs)

API 还明确存在消费者落后而错过消息的 `Lagged` 事件。[Event API](https://docs.rs/iroh-gossip/0.101.0/iroh_gossip/api/enum.Event.html)

推论：不能仅靠一次 gossip 广播兑现“撤销记录不会丢失、离线节点日后一定补齐”。我们应评估现成持久同步组件，让广播服务及时性，让可验证存储和补齐服务长期正确性。无需因此再自写一套广播树。

`iroh-docs 0.101.0` 是另一个候选：提供持久存储、签名条目和 range-based set reconciliation，并组合 blobs 与 gossip。它的条目由 namespace 写权限密钥与 author 密钥签名。[iroh-docs API](https://docs.rs/iroh-docs/latest/iroh_docs/)

但其记录排序含时间戳和内容哈希，namespace 写能力也不自动等于我们的管理员 ACL。准确地说，条目由 namespace、author、key 共同标识：同 author、同 key 的新记录可以替代旧记录，不同管理员使用不同 author 时，同 key 并不会因此自动互相覆盖。跨作者如何选出有效策略，以及怎样保留需要用户裁决的冲突，仍是应用职责。是否用独立事件条目承载授权记录，需要评估额外复杂度。[固定提交 sync.rs](https://github.com/n0-computer/iroh-docs/blob/8cfeacb087b4b195b1930683aa4448e990da0659/src/sync.rs)

## 6. precode 已有很接近的先例

### 6.1 传统 ssb-invite

它让指定 pub 生成邀请码，申请者使用邀请临时凭据联系该 pub，再提交自己的长期身份。pub 本地记录 `total/used`，成功后发布 follow 消息。[ssb-invite 固定版本 README](https://github.com/ssbc/ssb-invite/tree/cc29449b601e178f9cf0a89baf44b05ad0f1f2b9)

这验证了我们讨论的职责划分有现成先例：临时入门凭据、指定节点核销、长期节点身份、成功后的独立结果。

但核查源码明确保留并发扣次竞争的注释；计数写入与 follow 发布也不是一个完整的授权结果事务。不能把这份旧实现直接当成我们的严格次数与重试保证。[核销源码](https://github.com/ssbc/ssb-invite/blob/cc29449b601e178f9cf0a89baf44b05ad0f1f2b9/index.js#L153)

### 6.2 peer-invites 与 Room

`ssb-peer-invites` 将邀请、申请者接受、接待者确认分开，展示了可验证消息之间的关联。但源码明确记录多个接待者可能接受不同首次兑换结果的问题，不能用它替代我们已选定的单核销者。[验证逻辑](https://github.com/ssbc/ssb-peer-invites/blob/491c777152c37eb60e9933209b92b3a4421e7369/valid.js)、[已知问题](https://github.com/ssbc/ssb-peer-invites/blob/491c777152c37eb60e9933209b92b3a4421e7369/index.js#L57)

Go Room 的 HTTP invite 在一个数据库事务中加入成员并使 token 失效，提供了本地一次性核销的参考。但已消费 token 重试会失败，而不是返回同一授权结果；也没有由此获得可跨任意渠道传播的全网成员证书。[事务源码](https://github.com/ssbc/go-ssb-room/blob/892b77139af8e2cf1a6482908f3f288339680aa9/roomdb/sqlite/invites.go#L113)、[重试测试](https://github.com/ssbc/go-ssb-room/blob/892b77139af8e2cf1a6482908f3f288339680aa9/roomdb/sqlite/invites_test.go)

SSB HTTP invite 的网页本身参与 HTTPS 入网服务，和我们“网页服务器仅分发客户端”的角色不同；SSB 成功入网后的 follow 关系也不等于 miopunch ACL。[HTTP invite 规范](https://ssbc.github.io/ssb-http-invite-spec/)

结论：借鉴证据结构和事务处理，继续保留我们的产品语义。复用不意味着必须接受社交图、公网 pub 或旧实现中的边界缺口。

## 7. 浏览器与节点辅助职责已有先例

`ssb-browser-core` 将身份、日志和索引放在浏览器，通过 WebSocket 接入 pub/room。这是“浏览器拥有自己的身份、连接能力不同”的直接参考；它还处理多个 tab 并发写入的限制。该仓库已于 2026-05-13 归档，不能只因有旧 demo 就视为当前低维护成本依赖。[browser-core](https://github.com/arj03/ssb-browser-core)

Rooms 2 在转接的双向流内建立两端自己的加密连接。它不是 WebRTC 穿透协议。Pub 保存并提供 feed，Room 帮助在线节点建立隧道，Room 自己仍有成员与邀请数据库。[Rooms 2](https://ssbc.github.io/rooms2/#tunneled-connection)

推论：消息递送、密文流量转发、持久保存权限记录，可以是不同职责。让用户节点承担其中一些，不等于需要一个同时包办一切的固定服务器。

iroh 官方目前仍说明浏览器连接经 relay，WebRTC/WebTransport 是可能扩展方向。[iroh 浏览器说明](https://docs.iroh.computer/languages/wasm-browser)

因此浏览器对等语义有先例，但“在任意 NAT 条件下、没有任何可达辅助节点、打开网页就能联系所有成员”没有因此获得保证。WebRTC 继续作为候选，具体可达性仍需实验。

## 8. 更值得继续评估的现成栈：p2panda

p2panda 提供分模块的签名操作、存储、同步、网络和授权能力。其 0.7.1 已发布于 2026-08-21，发布记录包括规范化编码和 iroh 依赖升级；高层 Node 的完整 group/space 集成及部分新的 authoriser/sync 接口仍列在 Unreleased 下。[Changelog](https://github.com/p2panda/p2panda/blob/main/CHANGELOG.md)

最有价值的调查方向，是评估已有 core/store/sync 能否承载我们的小量管理记录，而不是立刻引入整个产品框架。

| 模块 | 已有能力 | 对我们的意义 |
| --- | --- | --- |
| core | 带签名的单写者操作日志，header/body 分离 | 评估复用现成记录格式和验证实现 |
| store | SQLite 存储、日志/游标/组状态、事务接口 | 评估复用持久化，避免自行拼出另一套存储协议 |
| sync | 双边 topic/log 补齐，以及协议与调度接口 | 评估复用断线追补，不等于已有 MQTT 适配 |
| net | iroh、发现、地址簿、gossip 与同步 | 已组合在线传播和历史补齐，但不是任意节点自动提供任意转发 |
| auth | 成员、权限条件、依赖关系、冲突处理 | 有现成授权引擎，但默认行为需要单独审视 |

模块依据：[core](https://docs.rs/p2panda-core/latest/p2panda_core/)、[store](https://docs.rs/p2panda-store/latest/p2panda_store/)、[sync](https://docs.rs/p2panda-sync/latest/p2panda_sync/)、[net](https://docs.rs/p2panda-net/latest/p2panda_net/)、[auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)。

高层 API 将交付与处理分开，说明了至少一次交付、确认和崩溃后重放。采用它仍需让本地业务处理幂等，不能因有存储库就宣称任意操作严格只执行一次。[高层 API](https://docs.rs/p2panda/latest/p2panda/)

其作者直接讨论了我们已接受的取舍：节点获知撤权后拒绝未来操作，是很多 P2P 应用可以采用的方式；若还要求各节点对撤权发生期间的应用历史完全一致，就需要额外追溯处理。[作者关于访问控制的分析](https://p2panda.org/2025/08/27/notes-convergent-access-control-crdt.html)

这个区分很重要：可靠传播权限事实，与自动回滚一切上层历史，是不同目标。我们的“收到即生效、离线窗口接受”不需要因为采用某个库就扩展成后者。但成员授权本身发生矛盾时，仍需明确识别与用户解决规则。

### 8.1 默认授权策略不直接对应本轮共识

`p2panda-auth` 默认 strong-removal 会使部分与作者撤权/降权并发的操作失效，并连带处理依赖。它支持自定义 Resolver，但扩展点存在不代表已经实现用户裁决流程。[auth API](https://docs.rs/p2panda-auth/latest/p2panda_auth/)

作者说明同时互相移除的 manager 可能让组失去所有管理者，并明确其方案不使用 invitation token。这和我们的创世提权恢复入口、precode 指定节点核销有实际差异。[访问控制设计](https://p2panda.org/2025/07/28/access-control.html)

推论：值得首先评估其日志、存储、补齐；是否使用完整 auth 模块需要另做语义对照。“不做拜占庭共识”并不自动排斥采用 CRDT，但我们也不因复用 CRDT 就接受未讨论的冲突裁决规则。

### 8.2 版本、平台与多渠道的现实边界

项目 README 提醒 API 尚未稳定到适合生产依赖的程度。[项目说明](https://github.com/p2panda/p2panda)

2026-09-16 官方描述的 Dash Router、更多网络上下文下的同步，仍在原型与后续验证阶段。文章还说明，路由、网络和同步需要交换信息，过早隔离会妨碍调优。这支持我们的渠道与信任解耦原则，同时提醒不能想当然地用一个通用适配接口抹平所有渠道差异。[官方进展](https://p2panda.org/2026/09/16/dash-router-sync-api.html)

本轮未证实新栈已提供完整、受支持的浏览器对等节点。旧 shirokuma 的 GraphQL 客户端不能当作新栈的浏览器实现证据。另有实验性 Go/Python/Node.js FFI，可作为集成线索，不能视为已验证的全平台支持。[旧浏览器 SDK](https://github.com/p2panda/shirokuma)、[实验性 FFI](https://github.com/p2panda/p2panda-ffi)

### 8.3 Willow 作为次级对照

Willow/Meadowcap 对独立可验证的读写能力、授权委托和传输解耦有学习价值。但它的时间范围主要约束条目的 timestamp 与数据范围，不能直接当作我们“由管理员按当前时钟核销邀请码”的有效期。其撤销讨论包含限期授权不续期与较新数据覆盖，与明确的成员撤销不是同一套语义。[Meadowcap 规范](https://willowprotocol.org/specs/meadowcap/index.html)、[时间与撤销讨论](https://willowprotocol.org/more/timestamps_really/index.html)

当次核查的 `willow25 0.7.9` 已有持久存储和 Drop Format；但新版 Confidential Sync 规范仍标为 Proposal，旧 JS 实现的兼容性也不能从当前规范推出。`iroh-willow` 已发布的 0.0.1 页面仍说明工作尚未发布完成。[willow25](https://docs.rs/willow25/latest/willow25/)、[同步规范](https://willowprotocol.org/specs/confidential-sync/index.html)、[willow-js](https://github.com/earthstar-project/willow-js)、[iroh-willow](https://docs.rs/iroh-willow/latest/iroh_willow/)

因此，Willow 暂列为数据与授权模型的对照，不把它排在 p2panda 的管理记录复用调查之前。也不建议为使用它而把我们的 ACL 改造成文件数据空间的读写能力。

## 9. 维护状况影响直接复用的选择

以下是调查日读取的默认分支或发布状态，不等于对所有分支、安全性或运行质量作保证。

| 项目 | 核查到的版本/状态 | 对选型的含义 |
| --- | --- | --- |
| epidemic-broadcast-trees | 9.0.4；默认分支最新提交 2023-02-24 | 独立 JS 核心，有测试；需评估长期维护与宿主适配 |
| ssb-ebt | 9.1.2；默认分支最新提交 2022-10-18 | SSB 数据库/muxrpc adapter，不是任意程序的独立同步服务 |
| go-ssb | 默认分支最新提交 2023-03-08；项目 alpha、EBT beta | EBT handler 仍有多连接分摊请求的 TODO；不能假设与 JS 完全等价 |
| ssb-invite | 3.0.3；默认分支最新提交 2024-02-12 | 单核销者先例；实际计数逻辑需要审视 |
| ssb-peer-invites | 2.1.0；默认分支最新提交 2020-06-28 | 消息证据关系可学，不能继承其重复兑换缺口 |
| go-ssb-room | 默认分支最新提交 2023-07-10；latest release v2.0.6 为 2021-06-03 | 隧道和本地事务参考，不是新项目即插即用的全栈 |
| ssb-browser-core | 2026-05-13 归档 | 浏览器架构先例，不直接认定为长期依赖 |
| iroh-gossip | 0.101.0；核查提交 f128b987，2026-07-30 | 现成在线广播，需另行提供持久补齐 |
| iroh-docs | 0.101.0；核查提交 8cfeacb0，2026-08-19 | 现成持久同步，但数据与权限语义需匹配 |

SSB 前三项仓库未标记归档；go-ssb 的其他仓库活动比默认分支新，不能据上表笼统宣称整个项目完全停更。

版本依据：[EBT package.json](https://github.com/ssbc/epidemic-broadcast-trees/blob/942db6ebab378ba538a87b293f0aab7ffd33695f/package.json)、[EBT commit](https://github.com/ssbc/epidemic-broadcast-trees/commit/942db6ebab378ba538a87b293f0aab7ffd33695f)、[ssb-ebt commit](https://github.com/ssbc/ssb-ebt/commit/546932f36c08e050ed772f2c39a8a4463968a512)、[Go EBT handler](https://github.com/ssbc/go-ssb/blob/d6db27d1852d5edff9c7e07d2a3419fe6b11a8db/plugins/ebt/handler.go)、[ssb-invite commit](https://github.com/ssbc/ssb-invite/commit/cc29449b601e178f9cf0a89baf44b05ad0f1f2b9)、[peer-invites commit](https://github.com/ssbc/ssb-peer-invites/commit/491c777152c37eb60e9933209b92b3a4421e7369)、[room commit](https://github.com/ssbc/go-ssb-room/commit/892b77139af8e2cf1a6482908f3f288339680aa9)、[room release](https://github.com/ssbc/go-ssb-room/releases/tag/v2.0.6)、[gossip commit](https://github.com/n0-computer/iroh-gossip/commit/f128b98705f5ad3a28a92f1d2b483686348b6ae8)、[docs commit](https://github.com/n0-computer/iroh-docs/commit/8cfeacb087b4b195b1930683aa4448e990da0659)。

## 10. 演讲中应避免直接转成规格的概括

- TLS 并不必然要求依赖公开 CA；标准已有 raw public key 等认证方式。不能为了去中心化就认定必须自行发明握手。[RFC 7250](https://www.rfc-editor.org/rfc/rfc7250.html)
- 有签名的日志仍可能由持钥者分叉；“不可篡改”应理解为修改会被验证发现，不是作者无法签出冲突历史。
- EBT 的复制来源切换与 iroh 的传输路径切换是不同层次，不能用前者解决 UDP 限速或 NAT 可达性。
- Pub 的持久复制与 Room 的流量转发不同；公开 feed 的存储节点和只转发端到端密文的节点，对数据的可见性也不同。
- 没有全网共识，不等于无需验证身份、重放、依赖关系或冲突。

后三类判断依据见前述规范和源码。这里是校正技术边界，不否定演讲的启发价值。

## 11. 当前建议：先确定复用位置

**建议暂停自行细化新的广播/复制 wire format，先比较现成组件与我们的数据、权限语义是否相容。**

优先考虑直接复用的工作：成熟连接栈、消息签名与编码实现、持久存储与事务、缺失记录补齐、在线广播状态机。每一项是否真的省工作，要连同版本、平台、语言边界和维护一起评估。

必须由我们明确但未必要从零实现的规则：网络 ACL、sudo 验证绑定何种操作、撤销的生效范围、冲突如何交用户处理、precode 单核销者与重试语义。这些是产品契约，不应被某个库的默认行为悄悄替换。

原生 Go、Rust 绑定或独立进程均未选定。保留 Go 不应自动意味着把成熟 Rust 网络/同步组件重写一遍；采用 Rust 组件也不自动意味着全项目重写。

## 12. 最小验证场景，尚未执行

1. A 发出有效撤销记录，B 确认保存后 A 离线；C 日后只联系 B，能够补齐并验证。
2. B 重启后仍能提供记录；旧状态、重复消息和乱序到达不会恢复已撤销权限。
3. 两个管理员各自产生合法操作，传播层完整保留；业务冲突能交用户解决，存储层不会暗中按时间戳覆盖掉其中一个。
4. 指定节点核销 precode 后回复丢失，再次申请取得同一结果，不重复扣次。
5. 同一个 peer 换连接渠道，身份和权限含义不变；不预先承诺跨所有承载无缝迁移。
6. 浏览器具有独立身份并参与相同授权规则，确认候选组件实际的存储、连接和生命周期支持。

这些场景用于比较复用价值，不是新增主线测试要求。小网络里 EBT 的流量优化未必最重要；能否少写一套记录、恢复和验证逻辑，才是这轮复用调查首先要回答的问题。

## 13. 当前比较场景：老成员返回并补齐服务 ACL

本节根据后续讨论收窄当前评估范围。上节其他场景保留为后续候选，不同时要求本轮全部解决。

### 13.1 场景边界

- 小型网络，大部分节点在线。
- 假定参与通信的节点已有可用的认证双向连接；不比较发现、打洞、QUIC/WebRTC、路径切换或网关故障。
- 手机 P 有权作出本次 ACL 修改；基线不同时引入管理员任免或 sudo 凭证恢复。
- 服务器 S 是服务提供者与规则保存者，不要求它也是管理员。
- 平板 T 已有有效成员身份，离线数日；客户端停止和设备关机对本场景等价。
- ACL 控制成员访问目标的本地服务，不控制 miopunch 自身通信。
- 当前采用“目标开放范围与成员授权范围都允许才放行”的简单解释。
- 不选择 L2/L3/L4 组网实现。协议与端口仅用于精确说明访问结果。

### 13.2 一次正常变化

为了只比较成员 ACL 的同步，暂假设 S 的本地开放范围已涵盖本例的 80 与 8080；这只是比较用的条件，不是产品默认开放所有端口，也不代表这两个端口上一定都持续运行服务。目标自己调整开放范围可另加场景。

| 时点 | 发生的事情 | 期望 |
| --- | --- | --- |
| 初始 | T 可以经 miopunch 访问 S 的 TCP 80 | 有效成员身份和旧 ACL 已保存 |
| T 离线 | T 停客户端或关机数日 | S、P 照常参与网络 |
| P 修改 | 对 T 取消 TCP 80、允许 TCP 8080 | 可表达为一次完整策略变更；格式未定 |
| S 收到 | 验证 P 的身份与改策略的权限，保存并应用变更 | S 后续拒绝 T:80，允许符合条件的 T:8080 |
| P 离线 | 本次修改者不再在线 | S 仍拥有可供验证和转交的有效依据 |
| T 返回 | T 以原身份联系 S，补齐必要信息 | 不要求 P 在线，不要求重新加入网络 |
| T 访问 | 请求 80 或 8080 | S 按新规则判断，即使 T 曾持有旧规则也不能要求 S 放行旧权限 |

本例的“允许”是 ACL 判定，不保证目标服务在监听或业务请求一定成功。

### 13.3 衡量组件的具体问题

1. 是否能保存 P 作出的变更及验证所需依据，并由 S 在重启后继续提供？
2. S 不是修改者，也不是管理员时，能否转交原始授权记录而不冒充批准者？
3. T 缺席数日后，能否取回足以获得当前有效规则的数据？不预设必须永久保存、回放全部历史。
4. 重复送达、重试和旧记录晚到，会不会让 80 重新开放？
5. 取消 80 与允许 8080 能否作为完整策略修改处理，避免只应用半份而误报完成？
6. 同步记录的权限，与实际连接本地服务的权限，是否被错误混为一谈？

“保存并可供补齐”需要包含策略正文和验证依据，只有签名头或内容哈希并不等于正文已经在磁盘上。特别是 iroh-docs 的条目引用独立内容，不能把 metadata 同步完成当作整个策略已持久保存。

主场景暂时只有一个有效修改者、没有并发冲突。它检验正常工作流程，不证明多管理员冲突、撤权并发或历史裁剪已经正确。最小扩展只先加入 S 重启、重复消息与旧状态重连。

### 13.4 基于文档和源码的比较，尚非实测

| 候选 | 在本场景能提供的已有部分 | 我们仍需做的适配或确认 | 当前判断 |
| --- | --- | --- | --- |
| SSB 日志 + EBT + 存储 | 签名记录、逐作者进度、第三方复制和缺失追补 | 将管理身份映射到 writer；ACL 内容与授权验证；本地服务访问检查；宿主存储与较旧生态维护 | 好的完整流程参考，不能把 EBT 单独视为成品 |
| iroh-gossip 单独使用 | 在线广播、减少重复传输 | 没有长期历史补齐；需另建持久层、记录认证和 ACL | 单独不足以满足 T 离线数日后的恢复，可作及时通知 |
| iroh-docs + 内容存储 | 持久条目、作者签名、集合补齐、从持有副本的 S 同步 | namespace 写能力与管理权限的映射；策略条目表达；跨作者更新规则；业务访问检查 | 值得与日志方案对照，主场景不必先排除状态同步 |
| p2panda core/store/sync | 签名操作、SQLite 存储与事务、日志与游标、双边补齐 | P 是否有权修改这份策略；操作如何变成当前 ACL；访问判定；发布版接口适配 | 优先做下一轮具体集成评估的候选 |
| p2panda-auth | 可验证的组管理、条件化权限、依赖与冲突处理 | 其组/数据权限不是现成 TCP/UDP 端口 ACL；默认强删除策略与用户裁决需要比较 | 暂不作为此基线的必选依赖 |
| Willow/Meadowcap | 数据同步和可验证能力模型 | 数据空间权限与服务 ACL 映射，更新/撤销语义，现成实现成熟度 | 次级对照，当前没有更省工作的证据 |

组件事实来源：[SSB EBT](https://ssbc.github.io/scuttlebutt-protocol-guide/#ebt-replication)、[gossip 实现](https://github.com/n0-computer/iroh-gossip/blob/f128b98705f5ad3a28a92f1d2b483686348b6ae8/src/proto/plumtree.rs)、[iroh-docs](https://docs.rs/iroh-docs/latest/iroh_docs/)、[p2panda-core](https://docs.rs/p2panda-core/latest/p2panda_core/)、[store](https://docs.rs/p2panda-store/latest/p2panda_store/)、[sync](https://docs.rs/p2panda-sync/latest/p2panda_sync/)、[auth](https://docs.rs/p2panda-auth/latest/p2panda_auth/)。版本与 Willow 边界见前文。

这些比较评价是基于公开接口和实现的推论，不是已经完成的通过/失败测试。没有一个候选仅靠接入同步模块就自动执行 miopunch 对本地服务的端口限制。

### 13.5 对下一步研究的影响

把待解决的问题暂时分为：规则数据及其证据怎样可靠到达，以及目标 S 怎样据此判断一次服务访问。前者优先复用已存在的存储/同步机制；后者使用清楚、可测试的产品规则，并评估成熟策略工具，不立即自写复杂权限框架。

用一条完整策略记录表达本次修改，是比较用的简单候选，不决定 wire format、全网版本号或永远只能同步日志。若现有状态同步组件更简洁地满足场景，可以重新讨论早先偏向日志的设计。

若采用日志，作者的日志前一项也不一定是这份 ACL 的前一版本。日志连续性和策略替代关系需要分别理解，尤其不能把不同策略、不同管理员的写入混成一个全局序号。

当前建议优先对照 p2panda 的日志基础与 iroh-docs 的状态条目模型，SSB/EBT 提供设计参考。选型依据是完成本场景需要多少额外逻辑、哪些保证已有、哪些需要自己承担；不以“必须叫 EBT”或“必须保留早先设计”为前提。
