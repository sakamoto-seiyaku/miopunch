# Signaling backend 候选信道专项调研（Door 3，Discovery/Mailbox/Exchange）

## 背景与目标

本调研服务于 Door 3（Signaling backend 插件化）的下一步收口：我们需要回答“哪些信道 **可以** 被抽象成 external backend”，以及“第一批 backend 组合（V1）为什么建议从 `MQTT + NATS` 开始（先证明可插拔），并为后续 DHT/去中心化入口路线留下清晰框架”。

已确认的前提与口径见：

- 纲领：`docs/decisions/door-3-signaling-backend-charter.md`
- 讨论纪要：`docs/notes/2026-04-23-signaling-backend-discussion-summary.md`

本调研只覆盖 **可行性、profile 与风险**，不下钻到 wire 字段/协议细节。

## 评估模板（统一口径）

对每个候选 backend，用同一套问题快速评估（KISS）：

- **是否自建（self-hostable）**：能否在用户/团队自有基础设施中部署；是否依赖“不可控第三方平台”。
- **Discovery**：新节点如何“找到入口/下一步提示”（允许慢）。
- **Mailbox（fallback）**：网内不可达时，是否能投递/收取控制消息（允许慢、可重复、可乱序）。
- **Exchange**：至少支持一种：
  - `realtime`：低延迟交互式交换（更适合 pubsub/push）
  - `scheduled`：基于 start window 的约定式交换（慢速/状态型也可做）
  - `store-and-poll`：写入状态 + 轮询观察（实现风格，不等于“只做 discovery”）
- **BackendProfile 关键约束**：`max_payload_bytes`、`push_or_poll`、`latency_class`、`min_lead_time`。
- **不可信建模与风险**：ToS/封号风险、可观测元数据、可被封锁/被墙、滥用/Spam 面、实现复杂度。

## 能力矩阵（速览）

> 目的：一眼回答“这玩意能不能当 external backend”，以及它更像 `push` 还是 `poll`、更像 `realtime` 还是 `scheduled`。

| Backend | Self-hostable | Public usable | Discovery | Mailbox（fallback） | Exchange | Notes（profile/硬约束） |
|---|---:|---:|---:|---|---|---|
| MQTT | Y | Y | Y | push（可离线队列\*） | realtime / scheduled | broker 配置决定队列/ACL/大小上限 |
| Cloudflare Tunnel（HTTP/WS to self-hosted origin） | Y | Y | Y | push/poll（取决于自建服务） | realtime / scheduled | published app 支持 HTTP/HTTPS；非 HTTP 需要 client-side cloudflared |
| Cloudflare Tunnel（Quick Tunnels / `trycloudflare.com`） | Y | Y | Y | push/poll（取决于自建服务） | realtime / scheduled | 无需账号；随机域名；并发/功能有限（更偏 dev/临时兜底） |
| Cloudflare Workers（Durable Objects + WebSocket relay） | N | Y | Y | push + history（可选） | realtime | serverless；可基于模板/示例快速部署（但仍需要一段 Worker 代码） |
| Cloudflare Pub/Sub（managed MQTT） | N | ? | Y | push（MQTT） | realtime / scheduled | 需要 CF 账号；产品可用性/计划与配额需再确认 |
| Matrix room | Y | Y | Y | push + history | realtime | event ≤ 64KiB（协议上限） |
| IRC channel | Y | Y | Y | push（弱 history） | realtime | 单行 512B 上限 |
| XMPP（MUC + PubSub） | Y | Y | Y | push（MUC）/pubsub | realtime | server feature 差异大（XEP 支持不一） |
| NATS（core） | Y | Y | Y | push（ephemeral） | realtime | 不持久化（除非用 JetStream） |
| NATS（JetStream） | Y | ? | Y | push + history | realtime | 有 durable stream/consumer 语义 |
| WAMP（router） | Y | ? | Y | push（ephemeral） | realtime | 必须有 router；可视为“WebSocket 消息总线” |
| Nostr relay | Y | Y | Y | push（relay 语义） | realtime | 多 relay 的“多写入”是否要视为镜像写需另定口径 |
| libp2p（kad-dht + pubsub/stream） | Y | Y | Y | push（pubsub） | realtime / scheduled | durable mailbox 需额外层；需要 bootstrap peers |
| IPFS（pubsub + IPNS/DAG） | Y | Y | Y | push（pubsub）/poll（IPNS） | realtime / scheduled | pubsub 偏 ephemeral；IPNS 更像 store-and-poll |
| BT DHT（BEP44） | Y | Y | Y | poll（KV） | scheduled / store-and-poll | `v` 实务常见 ≤ 1000B（节点可拒存更大值） |
| BT DHT（BEP50） | Y | ? | Y | push（pubsub） | realtime | 成熟度/实现覆盖待验证 |
| Git private repo | Y | Y | Y | poll（pull） | scheduled / store-and-poll | 历史膨胀、冲突策略与权限模型是核心约束 |
| Email（SMTP + IMAP/POP3） | Y | Y | Y | poll（收件箱） | scheduled / store-and-poll | 延迟不可控/易受反垃圾影响；消息大小/频率受服务商限制 |
| DNS TXT | ? | ? | seed-only | N | N | `<character-string>` 255B/段；TTL/缓存/截断影响大 |
| Webhook reflector（request bin） | ? | Y | Y | poll（API） | scheduled / store-and-poll | 平台 ToS/滥用面；自建同类服务可变为 Y |
| CoAP（自建 server） | Y | N | Y | poll/observe | scheduled / store-and-poll | 协议型“自建 mailbox”；实质依赖自建服务 |
| WebTorrent tracker | ? | Y | seed-only | N | seed-only | 更像 SDP/offer-answer rendezvous，不是通用 mailbox |

\* “可离线队列”指 backend 具备某种“订阅者离线时可恢复”的语义；具体能否成立取决于实现与配置（MQTT/NATS/Matrix/XMPP 都存在差异）。

## 结论速览（先给分型，避免发散）

### 第一批（V1 落地）组合：`MQTT + NATS(core)`

- `MQTT`：基线 external backend（push/pubsub，realtime 能力强），继续作为默认选项合理。
- `NATS(core)`：作为“最像 MQTT 的第二 backend”与参考实现（可用公共 demo 做 smoke/开发验证，但不把公共 demo 作为生产依赖）；若需要离线可恢复/可重放语义，应进入 JetStream 或改用其它 backend。

### 去中心化入口方向：`MQTT + DHT`（后续专项验证）

- 目标不是“把 MQTT 换成 DHT”，而是让系统具备“去中心化入口/overlay”这一类 backend 的可插拔落地路径。
- DHT 这条路线会显著牵涉 `scheduled/store-and-poll` 的 profile 与工程约束，因此更适合作为后续专项验证。

### `V2` 候选：慢速/状态型 backend（Git/Email 等）

- `Git private repo` 与 `Email（SMTP + IMAP/POP3）` 是典型的 `store-and-poll / store-and-forward` carrier：更慢，但覆盖面广、部署形态多（含“完全自控”的可能）。
- 这类 backend 更适合承载：`bootstrap + fallback mailbox + scheduled/store-and-poll exchange`，不应假设 `realtime exchange`。

### “DHT backend”并不等价于“只用 DHT 存取”

从工程落地角度，把 “DHT/去中心化入口” 相关实现分成几条典型路线更清晰：

- **路线 A（更像“完整 backend”）**：`libp2p(kad-dht + pubsub/stream)`  
  用 DHT 做 discovery/rendezvous，用 pubsub/stream 做 mailbox + exchange（realtime 或 scheduled 都可）。
- **路线 B（更像“状态型 backend”）**：BitTorrent DHT 的 `BEP44`（store-and-poll）  
  能做 discovery + scheduled/store-and-poll exchange；但把它扩展成“高频 mailbox”会明显变复杂（容量/多写者/可枚举性）。
- **路线 C（更像“尝试把 DHT 变成 pubsub”）**：BitTorrent DHT 的 `BEP50`（DHT PubSub）  
  目标是提供 push/pubsub 语义，但成熟度与实现覆盖需要专项验证。

因此：**libp2p 不是“DHT 的必备条件”**；但在“必须满足 Discovery+Mailbox+Exchange 三件套”的约束下，路线 A 更像一种“天然完整 backend”的形态，而路线 B 更像“慢速/状态型 backend”的代表样本（用于校准 `BackendProfile` 是否足够表达差异）。

## 候选 backend 清单（按类型分组）

> 注意：这里只列“可被抽象成 external backend 的候选 carrier”。其中一些是完整 backend，另一些更像 `seed-only` 或“仅适用于特定 profile”。

### A) Push/PubSub 型（更适合 realtime exchange）

#### MQTT（baseline）

- Self-hostable：是（Mosquitto/EMQX 等）。
- Discovery：join code 提供 broker/topic prefix 即可。
- Mailbox：天然（topic inbox）。
- Exchange：天然（realtime），也可降级 scheduled。
- 约束：payload/retain/ACL 由 broker 决定（应通过 BackendProfile 暴露上限）。

#### Matrix room（自建 homeserver）

- Self-hostable：是（Synapse 等），也可用公共 homeserver（取决于运维/策略风险与可用性）。
- Discovery：room id / alias。
- Mailbox：room 消息（push + 历史可追溯）。
- Exchange：realtime（但时延与限流较强，偏 “slow realtime”）。
- 关键约束：事件大小上限为 **65536 bytes**（协议层上限，具体实现可能更严）。见 Matrix spec “Size limits”。  
  这对 `max_payload_bytes` 非常友好，但仍需考虑 rate limit 与风控。

#### IRC channel（自建 IRCd）

- Self-hostable：是（inspircd/ergo 等）。
- Discovery：公开 channel / invite-only channel。
- Mailbox：弱（历史/离线依赖 server 配置与 bouncer；更像“在线 pubsub”）。
- Exchange：realtime（但 payload 极小）。
- 关键约束：单条消息最大 **512 字节**（含 CRLF）。这几乎强制要求“chunk/ref”策略，不适合承载太多 exchange 数据。

#### XMPP（MUC + PubSub）

- Self-hostable：是（Prosody/ejabberd 等），也可用公共 server（取决于策略与可用性）。
- Discovery：server domain + room（MUC）/节点名（PubSub）。
- Mailbox：
  - MUC message：偏 push（history/offline 取决于服务端配置）。
  - PubSub（XEP-0060）：更像“可持久化的主题/节点”，可表达 poll 或 push+history（依实现）。
- Exchange：realtime（但更像 “slow realtime”；时延/限流/反垃圾影响大）。
- 关键点：XMPP 能力高度依赖 server 的扩展支持与配置（不要假设“所有 XMPP 都一样”）。

#### NATS（core / JetStream；含公共 server 形态）

- Self-hostable：是（单机/集群均可）；也存在公共 server（作为一种部署形态，不改变抽象）。
- Discovery：server URL + subject prefix。
- Mailbox：
  - core NATS：偏 push（ephemeral）。
  - JetStream：可做到 push+history（durable stream/consumer）。
- Exchange：realtime（core）或 realtime+durable（JetStream），scheduled 也可表达（只要 profile 满足）。
- 公共 server 形态（可用于 smoke/开发验证）：`demo.nats.io:4222`（注意：这是 demo，不应作为生产依赖）。
- 关键点：一旦需要“离线可恢复/可重放”，基本就进入 JetStream 语义。

#### WAMP（router 型 pubsub + RPC）

- Self-hostable：是（需要 router，例如 Crossbar.io）。
- Discovery：router URL + realm + topic prefix。
- Mailbox：topic pubsub（偏 push；history/persistence 取决于 router 能力与配置）。
- Exchange：realtime。
- 关键点：它更像“WebSocket 消息总线”；作为 backend 时要把“必须有 router”写进 profile/运维前提。

#### Nostr relay（弱中心、可多 relay）

- Self-hostable：是（自建 relay），也可使用多个公共 relay（取决于策略与可用性）。
- Discovery：relay 列表 + filter。
- Mailbox：事件发布/订阅（push）。
- Exchange：realtime（更像“弱中心 pubsub”）。
- 风险：生态在快速演进；不同 relay 的限流/事件上限差异大；需要明确“多 relay 写入=镜像写”是否违背 Door 3 的 KISS（默认我们不做镜像写）。

### B) DHT / P2P overlay 型（去中心化入口方向）

#### 路线 A：libp2p（kad-dht + gossipsub/stream）

参考样本：`edgevpn`（本地 clone：`/tmp/miopunch-research/edgevpn`）。

- Self-hostable：是（纯 P2P；但需要 bootstrap peers 列表作为入口 hint）。
- Discovery：
  - `kad-dht` 可用于 rendezvous/peer discovery（advertise/find）。
  - 也存在 “Rendezvous server” 协议（注意它是 server 形态，偏中心化；若采用需谨慎口径）。
- Mailbox：用 pubsub topic/stream 做 per-peer inbox 或 per-network mailbox。
- Exchange：更适合 realtime；也可实现 scheduled。
- 工程成本：依赖与运行复杂度较高（libp2p 组件较多；需要 profile/故障口径）。

调研用参考（协议族/组件导航）：

- libp2p protocols 列表（含 DHT、PubSub、Rendezvous 等）：`https://docs.libp2p.io/concepts/protocols/`。

#### IPFS（libp2p 生态的一种“打包形态”）

- Self-hostable：是（跑自己的 IPFS 节点；也可做 private swarm）。
- Discovery：可用 `IPNS`（name → 最新 CID）或其它基于 DHT 的 name 机制表达入口/提示。
- Mailbox：
  - `IPFS PubSub`：更像 push（偏 ephemeral，不擅长“离线后补收”）。
  - `IPNS + DAG`：更像 store-and-poll（把“最新状态/索引”挂在 IPNS 上，轮询观察变化）。
- Exchange：realtime（pubsub）或 scheduled/store-and-poll（IPNS）。
- 关键点：如果把 “Mailbox=离线可恢复” 当作强要求，则单靠 pubsub 往往不够，通常需要额外的“可持久化状态层”。

#### 路线 B：BitTorrent DHT（BEP44 store-and-poll）

- Self-hostable：是（可用现成 BT DHT 网络，也可做私有 DHT/受控 bootstrap）。
- Public usable：是（mainline DHT 是公共网络；加入时需要一组 bootstrap 节点/路由器作为入口 hint，例如常见的 `router.utorrent.com:6881` / `router.bittorrent.com:6881` / `dht.transmissionbt.com:6881`）。
- Discovery：适配容易（key → peer hint）。
- Mailbox：理论可做，但会很快遇到“多写者 + 多消息队列”困难（容量/可枚举性/垃圾回收）。
- Exchange：更适合 scheduled/store-and-poll（“写入状态 + 轮询观察 + start window”）。
- 关键约束：
  - BEP44 明确指出节点 **可以拒绝**存储 `v > 1000 bytes` 的值（实现中通常会更严格）。
  - item 在未 reannounce 的情况下默认过期时间为 **2 小时**；建议 **每小时** reannounce 一次以避免过期（对我们来说也意味着：它天然更适合“低频小状态”的 store-and-poll，而不是高频 mailbox 队列）。
  - 因此它天然适合“极小状态/索引”，不适合承载可变长消息队列。
- 对 Door 3 三件套的结论（把话说死，避免误解）：
  - `Discovery`：✅（前提：join code 或 network config 能给出“要查的 key/盐”；DHT 本身不可枚举/不可 list）。
  - `Exchange`：✅（以 `store-and-poll`/`scheduled` 为主；需要更大的 `min_lead_time` 与更长 window 来覆盖 DHT lookup 抖动）。
  - `Mailbox`：⚠️ **能做最小 mailbox，但不适合“通用队列”**：
    - 推荐建模为“每个 sender 一个 outbox”（单写者，避免 multi-writer 冲突），receiver 轮询已知成员的 outbox 集合并以 `msg_id` 去重；
    - 一旦需要并发多消息/大 payload，就不可避免引入：chunk/ref（immutable chunks + mutable manifest）、或非常保守的 ring buffer（都受 1000B 上限约束）。
    - 所以它更适合作为 backup backend（兜底），而不是把它当成“类 MQTT 的 mailbox”。
- 工程上对我们特别有用的 BEP44 原语（会影响 profile 与 API 设计）：
  - `get(seq=...)`：可以“只在对方 seq 变大时才返回 value”，这非常适合轮询场景（减少无效流量）。
  - `salt`：允许用同一 keypair 衍生多个逻辑 slot（例如 `outbox(S→R)`、`network bootstrap`、`round state` 等），同时仍可被读端验签。

#### 路线 C：BitTorrent DHT PubSub（BEP50，成熟度待评估）

- 目标：在 DHT overlay 上提供 push/pubsub 语义。
- 风险：
  - BEP50 是 **Draft**，并且它对 DHT 协议行为有额外约束（这意味着：想“直接用公网”需要足够多节点实现同一套行为；否则就只能自带节点/变相自建）。
  - 成熟度、实现覆盖、互通性需要专项验证；当前更像“未来路线”。

### 参考项目快速解读（用来校准“DHT 到底覆盖了什么”）

#### edgevpn（libp2p + DHT + PubSub）

- 直观观察：它虽然“用了 DHT”，但 **DHT 主要承担 discovery/rendezvous**；消息投递更像是 **通过 PubSub 完成**，而不是把“消息队列”硬塞进 DHT put/get。
- 对 Door 3 的启发：如果我们把目标定为“一个 backend 必须支持 Discovery+Mailbox+Exchange”，那么：
  - “DHT-only（KV put/get）”路径往往会迫使我们在 DHT 上实现队列/索引/垃圾回收；
  - 而“DHT + PubSub/Stream”路径更像是一个天然完整 backend。

#### wa-tunnel（第三方聊天软件承载数据）

- 价值：它是一个“极端慢/高限制 carrier 也能承载数据”的例子，证明我们抽象 backend 时不必只盯着传统 pubsub。
- 代价：第三方平台 ToS/风控/登录形态决定了它更像“插件候选”，而不是产品默认依赖。

### C) 状态型 / Store-and-poll（慢速但隐蔽；更适合 bootstrap + scheduled）

#### Git private repo（push/pull 作为 carrier）

- Self-hostable：是（Gitea/GitLab/self-host git + deploy key），也可用公共平台私库（存在平台风控/审计风险）。
- Discovery：repo URL + branch/path 作为入口。
- Mailbox：可行（按 peer_id 分文件/目录；pull 轮询）；但会受 repo 历史膨胀与冲突策略影响。
- Exchange：更偏 scheduled/store-and-poll（realtime 很难）。
- 优点：隐蔽性强、权限模型成熟（尤其 deploy key），非常适合作为“兜底兜底”的最慢后备。

#### Email（SMTP + IMAP/POP3；用邮箱当 “store-and-forward mailbox”）

- Self-hostable：可以（自建邮件系统/自有域名 + 托管服务）；也可直接使用现成邮箱服务（但受风控/反垃圾影响更大）。
- Discovery：邮箱地址（或 per-network 的地址/别名）+ 约定的 subject/tag 规则（属于 backend config）。
- Mailbox：天然存在（收件箱）；实现形态更偏 `poll`（IMAP/POP3 拉取），因此延迟与频率预算必须写进 profile。
- Exchange：更适合 `scheduled/store-and-poll`；不适合假设 `realtime`（投递时延与丢信不可控）。
- 风险/约束：
  - 反垃圾/风控：高频小包、加密 blob、批量收发都可能触发限流或投递失败；
  - 可观测元数据：发件人/收件人/时间戳/大小等会暴露；
  - “自建邮件服务器”运维成本通常 **高于** 自建 MQTT/NATS（这点需要在 V2 预期里写清楚）。

#### DNS TXT（极限脑洞：只当 discovery/极小状态）

- Self-hostable：取决于是否有可写 DNS（通常不是所有人都有）。
- Discovery：可行（`_miopunch.<domain>`）。
- Mailbox/Exchange：基本不现实（除非“极小状态 + 强轮询 + 严格限频”）。
- 关键约束：
  - 单个 `<character-string>` 上限 **255 bytes**（RFC1035）。
  - 即使通过多段字符串拼接、或理论上有更高的总上限，实践中仍受 UDP 截断、缓存与 TTL 影响。

#### Webhook reflector（request bin / 调试反射器）

- Self-hostable：视具体实现而定（可以自建；也可用第三方服务如 webhook.site）。
- Discovery：一段可写入/可查询的 URL（通常是“先生成再分发”）。
- Mailbox：天然偏 poll（服务端存储请求/事件，客户端轮询 API 拉取）。
- Exchange：更偏 scheduled/store-and-poll（用“轮询观察变化 + start window”来跑 exchange）。
- 风险/约束：ToS/滥用面、保留时长、请求大小上限、访问控制；若自建则主要变为“运维与审计”问题。

#### CoAP（协议型：自建 mailbox/server）

- Self-hostable：是（本质依赖自建 CoAP server 或网关）。
- Discovery：coap(s) URI + resource path。
- Mailbox：可以用 GET/POST/observe（Observe 更像 push），但实际语义取决于你如何设计 server 的资源与存储。
- Exchange：更偏 scheduled/store-and-poll（也可用 observe 做 slow realtime）。
- 关键点：它更像“用 CoAP 协议做一套最小自建 backend”，并不自带“去中心化入口”属性。

### D) 第三方平台（可行，但需把 ToS/风控当作 profile）

这些平台常常满足 Discovery/Mailbox（channel/group）与一定程度 Exchange（realtime 或 slow realtime），但在工程上要把下面这些“平台约束”当作 backend profile 的一部分（客观记录即可）：

- ToS/封号/风控（自动化、频率、加密内容、文件传输）
- API/客户端耦合（bot token、扫码登录、设备绑定）
- 地域可用性与网络封锁

典型例子（仅作为“可抽象”证明）：

- Telegram Bot API：`sendMessage` 文本长度上限 **4096** 字符（适合作慢速信令/短消息，不适合大交换）。
- Slack：`chat.postMessage` 存在消息大小上限（且有截断/展示差异）；更适合作“企业内兜底”而非公网默认。
- WhatsApp：`wa-tunnel` 展示了“用聊天承载数据”的可行性，但工程与风控成本高（参考：`/tmp/miopunch-research/wa-tunnel`）。

### E) Reverse-tunnel / Serverless（非 VPS 形态）

这类方案的共同点是：**不要求公网 VPS/固定公网入口**，但可能会引入某个“反向隧道/托管平台”的依赖。

#### Cloudflare Tunnel（published hostname）

- 目的：在内网/NAT 后自建 backend，同时对公网暴露一个 HTTPS 入口（反向隧道）。
- 注意：对 “published applications”，Cloudflare 支持的服务类型以 **HTTP/HTTPS 与少数 TCP 类协议**为主，且 **非 HTTP 服务需要在客户端安装 cloudflared**。因此对我们的节点来说，更现实的落地形态是：把自建 backend 做成 **HTTP/HTTPS/WebSocket**，而不是指望“任意 TCP/UDP 都能直接公开暴露”。
- UDP：Tunnel 的“隧道连接”本身可以走 TCP/UDP（HTTP/2/QUIC），但这不等价于“把任意 UDP 服务发布到公网”。在 Door 3 语义上，更建议把它当成 “HTTP/WS carrier 的一种部署方式”。

#### Cloudflare Tunnel Quick Tunnels（`trycloudflare.com`）

- 特点：无需 Cloudflare 账号即可获得随机 `trycloudflare.com` 域名；但定位更偏开发/临时用途，且存在并发与功能限制（例如：200 concurrent requests、无 SSE）。
- 对 Door 3 的定位：可作为“临时 bootstrap / 调试 / 演示”通道，不建议作为长期默认 backend。

#### Cloudflare Workers（Durable Objects + WebSocket relay）

- 形态：纯 serverless，“外部 backend 运行在 Cloudflare 上”，节点通过 HTTPS/WS 直连。
- 能力：Durable Objects 可以承载 WebSocket fanout 与少量状态/历史，非常接近我们对 `push mailbox + realtime exchange` 的目标能力。
- 交付：Cloudflare 有现成模板/示例可快速部署；但要成为 miopunch backend 仍需要我们提供一段最小 Worker（哪怕只是“加密 blob 透传 + per-network namespace”）。

#### Cloudflare Pub/Sub（managed MQTT）

- 形态：Cloudflare 托管 MQTT broker；对我们来说更像“换一个公共 MQTT 服务商”，而不是引入新协议。
- 需要：通常需要 Cloudflare 账号（以及开通对应产品/配额）；当前可用性（是否仍需申请/是否 GA）建议后续专门确认。
- 对 Door 3 的定位：如果可用，它会是“最接近 MQTT 体验”的 serverless 备选（无需自建 broker）。

### F) Tracker / SDP rendezvous（seed-only）

#### WebTorrent 公共 Tracker

- Self-hostable：可以自建 tracker（也存在公共 tracker 作为部署形态）。
- Discovery：更像“我把一组节点拉进同一个 rendezvous key/room 里”。
- Mailbox：不适合当通用 mailbox（tracker 的语义目标不是“存取任意控制消息”）。
- Exchange：更像“offer/answer 协调”这种特定形态的 rendezvous；因此在 Door 3 口径里更适合标注为 `seed-only`。

## 对 Door 3 的落地建议（高层，不下钻 wire）

### 1) 先把“backend 能力与 profile”抽象稳定，再讨论具体 carrier

避免陷入“先挑 carrier → 再被 carrier 反推协议边界”的局面。正确顺序应是：

1. 上层语义固定：Discovery + Mailbox + Exchange
2. profile 固定：`exchange_mode/latency_class/min_lead_time/max_payload_bytes/push_or_poll`
3. 每个 backend 实现“在其限制内满足语义”，达不到则明确“不满足该能力”

### 2) DHT/去中心化入口路线分型（不做优先级）

对 Door 3 来说，“DHT”更应该被理解为一类能力组合，而不是单一协议名：

- 想要“完整 backend（三件套）”更顺滑：通常会走 “DHT 做 discovery + 另一个消息原语做 mailbox/exchange”（例如 libp2p 的 pubsub/stream，或其它可订阅信道）。
- 想要“慢速/状态型 backend”样本：BEP44/IPNS/Git repo/webhook 这类 store-and-poll 更像同一类东西（关键是 profile：payload 上限、轮询间隔、min_lead_time）。
- 想要“把 DHT 变成 pubsub”：BEP50 属于这类路线，但成熟度/实现覆盖需要专项验证。

## 参考链接（只列本调研用到的关键材料）

- Door 3 charter：`docs/decisions/door-3-signaling-backend-charter.md`
- edgevpn：`https://github.com/mudler/edgevpn`
- wa-tunnel：`https://github.com/aleixrodriala/wa-tunnel`
- BEP44（BitTorrent DHT store）：`https://www.bittorrent.org/beps/bep_0044.html`
- BEP50（BitTorrent DHT pubsub）：`https://www.bittorrent.org/beps/bep_0050.html`
- IRC RFC1459：`https://www.rfc-editor.org/rfc/rfc1459`
- Matrix spec size limits：`https://spec.matrix.org/v1.14/client-server-api/#size-limits`
- DNS RFC1035：`https://www.rfc-editor.org/rfc/rfc1035`
- Telegram Bot API：`https://core.telegram.org/bots/api#sendmessage`
- Slack `chat.postMessage`：`https://api.slack.com/methods/chat.postMessage`
- libp2p protocols：`https://docs.libp2p.io/concepts/protocols/`
- Nostr NIP-01：`https://github.com/nostr-protocol/nips/blob/master/01.md`
- IPFS PubSub：`https://docs.ipfs.tech/concepts/pubsub/`
- IPNS：`https://docs.ipfs.tech/concepts/ipns/`
- XMPP XEP-0045（MUC）：`https://xmpp.org/extensions/xep-0045.html`
- XMPP XEP-0060（PubSub）：`https://xmpp.org/extensions/xep-0060.html`
- NATS docs：`https://docs.nats.io/`
- demo NATS server：`https://demo.nats.io/`
- CoAP RFC7252：`https://www.rfc-editor.org/rfc/rfc7252`
- CoAP Observe RFC7641：`https://www.rfc-editor.org/rfc/rfc7641`
- WAMP：`https://wamp-proto.org/`
- WebTorrent：`https://webtorrent.io/`
- bittorrent-tracker：`https://github.com/webtorrent/bittorrent-tracker`
- webhook.site：`https://webhook.site/`
- webhook.site API：`https://webhook.site/#!/api`
- Cloudflare Tunnel（published app protocols）：`https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/routing-to-tunnel/protocols/`
- Cloudflare Tunnel setup / Quick Tunnels：`https://developers.cloudflare.com/tunnel/setup/`
- Cloudflare Tunnel `trycloudflare.com`（Cloudflare One）：`https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/`
- Cloudflare Durable Objects WebSockets：`https://developers.cloudflare.com/durable-objects/best-practices/websockets/`
- Cloudflare templates：`https://github.com/cloudflare/templates`
- Cloudflare Pub/Sub announcement：`https://blog.cloudflare.com/announcing-pubsub-programmable-mqtt-messaging/`
- Cloudflare Pub/Sub（unofficial doc mirror, needs re-check）：`https://cloudflare-docs.justalittlebyte.ovh/pub-sub/platform/mqtt/`
