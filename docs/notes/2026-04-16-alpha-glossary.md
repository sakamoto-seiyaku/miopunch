# Alpha/POC 术语词典（草案）

> 状态：Alpha/POC 草案（非正式规范）。  
> 目标：在 Alpha/POC 阶段，为文档 / CLI / UI / 日志 / 配置提供**统一术语口径**；进入正式文档前，需要二次核实与术语收敛（例如 P4 阶段）。  
> 原则：引用 RFC/规范/上游项目文档仅用于溯源；本词典对 Miopunch 的“使用口径”可能与标准术语存在**有意的简化**。

## 约定

- **单一事实来源（SSoT）**：Alpha/POC 范围内，出现术语冲突时，以本文件为准。
- **术语键（term_id）**：用于日志 / reason_code / 配置键 / 文档引用的稳定标识；建议 `snake_case`。
- **稳定字段（freeze）**：`reason_code`/`stage`/`term_id`/`exit_code` 在 Alpha/POC 内视为稳定；不重命名只新增；必须改名时保留旧码为 alias/deprecated（保持兼容）。
- **别名（alias）**：用于兼容历史说法（例如 `xtcp` 系列旧名），不推荐对外展示。
- **来源（refs）**：尽量优先 RFC / 标准规范；其次是业界事实标准项目文档；不使用“二手转述”作为定义来源。

---

## 1) 网络与 NAT 基础

### NAT（`nat`）

- 定义：一种将一个地址域的 IP 地址映射到另一个地址域的方法，用于在尽量透明的前提下完成路由（常见：私网地址域 ↔ 公网地址域）。  
- Miopunch/POC：我们主要关心 **UDP 场景**下 NAT 的映射/过滤/回环行为对 P2P 直连的影响。  
- refs：RFC 2663（NAT Terminology and Considerations）https://www.rfc-editor.org/rfc/rfc2663

### NAT 映射（`nat_mapping`）

- 定义：NAT 为内部 `IP:port` 到外部 `IP:port` 建立并复用映射的行为（mapping behavior）。  
- Miopunch/POC：用于推断“对端是否可能预测端口/复用映射”，影响候选排序与重试策略。  
- refs：RFC 4787 §4 https://www.rfc-editor.org/rfc/rfc4787

### 端点无关映射（Endpoint-Independent Mapping，`eim`）

- 定义：同一内部 `IP:port` 发往任意外部 `IP:port` 时，复用同一个外部映射端口。  
- Miopunch/POC：倾向认为更“好打洞”，更可能形成稳定直连。  
- refs：RFC 4787 §4 https://www.rfc-editor.org/rfc/rfc4787

### 地址相关映射（Address-Dependent Mapping，`adm`）

- 定义：同一内部 `IP:port` 发往同一外部 **IP**（外部端口可不同）时复用映射；外部 IP 不同则可能得到不同映射。  
- refs：RFC 4787 §4 https://www.rfc-editor.org/rfc/rfc4787

### 地址+端口相关映射（Address-and-Port-Dependent Mapping，`apdm`）

- 定义：同一内部 `IP:port` 仅在发往同一外部 **IP:port** 时复用映射；外部端点变化可能导致映射变化。  
- refs：RFC 4787 §4 https://www.rfc-editor.org/rfc/rfc4787

### NAT 过滤（`nat_filtering`）

- 定义：NAT 的 filtering 行为：内部端点先出站建立状态后，NAT 决定“允许哪些外部源端点的回包”被转发回该内部端点。  
- Miopunch/POC：用于解释“为什么必须双方先出站、为什么入站被限制”。  
- refs：RFC 4787 §5 https://www.rfc-editor.org/rfc/rfc4787

### 端点无关过滤（Endpoint-Independent Filtering，`eif`）

- 定义：只要包的目的地是内部 `IP:port`，不关心外部源 `IP:port`，都允许入站。  
- refs：RFC 4787 §5 https://www.rfc-editor.org/rfc/rfc4787

### 地址相关过滤（Address-Dependent Filtering，`adf`）

- 定义：内部端点需要先向某外部 **IP** 出站，才允许来自该外部 IP（任意端口）的入站回包。  
- refs：RFC 4787 §5 https://www.rfc-editor.org/rfc/rfc4787

### 地址+端口相关过滤（Address-and-Port-Dependent Filtering，`apdf`）

- 定义：内部端点需要先向某外部 **IP:port** 出站，才允许来自该外部 IP:port 的入站回包。  
- refs：RFC 4787 §5 https://www.rfc-editor.org/rfc/rfc4787

### Hairpinning（回环/发夹，`hairpinning`）

- 定义：两个同 NAT 内侧的端点，仅使用彼此的外部映射 `IP:port` 也能互通。  
- Miopunch/POC：用于解释“同一局域网/同一路由器后面”的直连/打洞现象（可能走 hairpin）。  
- refs：RFC 4787 §6 https://www.rfc-editor.org/rfc/rfc4787

---

## 2) NAT 穿越与候选（STUN / TURN / ICE / Punch）

### STUN（`stun`）

- 定义：一套用于 NAT 穿越的工具协议，可用于端点获知 NAT 为其分配的外部 `IP:port`、做端点连通性检查、维持 NAT 绑定；**STUN 本身不是完整穿越方案**。  
- Miopunch/POC：用于自发现公网映射（`v4/v6 ip:port`）、推断 hint、以及候选信息的一部分。  
- refs：RFC 8489 Abstract https://www.rfc-editor.org/rfc/rfc8489

### STUN RTT（`stun_rtt`）

- 定义：一次 STUN transaction 的请求-响应往返时间（RTT）。  
- Miopunch/POC：作为内置 STUN view 仲裁/打平时的最小指标（阈值 `30ms` 视为打平）；不额外对映射 `ip:port` 发 ping。  
- refs：实现口径（POC 约定）

### TURN（`turn`）

- 定义：在无法直连时，通过“中继节点”转发；协议允许客户端控制中继并经由中继与对端交换包。  
- Miopunch/POC：POC 阶段主要做直连/打洞；TURN 代表“数据面中继”能力（后置）。  
- refs：RFC 8656 Abstract https://www.rfc-editor.org/rfc/rfc8656

### ICE（`ice`）

- 定义：面向 UDP 通信的 NAT 穿越协议，使用 STUN/TURN 来收集候选并做连通性检查/选路。  
- Miopunch/POC：我们借用 ICE 的“候选/端点对/选路”概念，但不承诺实现完整 ICE。  
- refs：RFC 8445 Abstract + §2.1 https://www.rfc-editor.org/rfc/rfc8445

### 候选传输地址（candidate transport address，`candidate_transport_address`）

- 定义：用于通信的候选“传输地址”，是某传输协议下的 `IP address + port` 组合（ICE 规范中以 UDP 为主）。  
- Miopunch/POC：候选可以来自本机接口（host）或 STUN 映射（server-reflexive）；最终只会选出一条用于打洞/直连。  
- refs：RFC 8445 §2.1 https://www.rfc-editor.org/rfc/rfc8445

### Host Candidate（本机候选，`candidate_host`）

- 定义：直接来自本机网络接口的地址候选。  
- refs：RFC 8445 §2.1 https://www.rfc-editor.org/rfc/rfc8445

### Server-Reflexive Address（服务端反射地址，`server_reflexive_address`）

- 定义：通过 STUN/TURN 在 NAT 公网侧观察到的“翻译后地址”（对端看见的公网映射）。  
- refs：RFC 8445 §2.1（示例列举）https://www.rfc-editor.org/rfc/rfc8445

### Relayed Address（中继地址，`relayed_address`）

- 定义：从 TURN 服务器分配得到、用于经由中继转发的地址。  
- refs：RFC 8445 §2.1；RFC 8656 Abstract https://www.rfc-editor.org/rfc/rfc8656

### 打洞（UDP Hole Punching，`hole_punching`）

- 定义：P2P NAT 穿越常用技术族：双方配合出站，利用 NAT 映射/过滤特性建立可互通的 UDP 路径；在部分 NAT 上可结合端口预测技巧。  
- Miopunch/POC：对应 `path=punch`；与 `path=direct`（直连）并列。  
- refs：RFC 5128（P2P across NATs）https://www.rfc-editor.org/rfc/rfc5128

---

## 3) Miopunch 连接语义（POC 口径）

### 地址族（`ip_family`）

- 定义：`v4` / `v6`；POC 支持 `auto`、`-4`、`-6` 约束。  
- Miopunch/POC：`-4/-6` 只约束 P2P 打洞与数据面建立；不扩展到其他无关功能。  
- refs：实现口径（POC 约定）

### 路径类型（`path`）

- 定义：`direct`（直连）/ `punch`（打洞）。  
- Miopunch/POC：连通成功后，CLI/UI 默认展示“实际使用的路径类型”。  
- refs：实现口径（POC 约定）

### 端点对（endpoint pair，`endpoint_pair`）

- 定义：一次连接最终实际使用的 `local ip:port → remote ip:port`。  
- Miopunch/POC：点对点详情默认展示端点对；全局总览不展开候选全集。  
- refs：实现口径（POC 约定）；候选概念来源 RFC 8445

### 端点漂移（Endpoint drift，`endpoint_drift`）

- 定义：同一 peer 的“可达远端端点（`ip:port`）”在运行过程中发生变化（换网/NAT 重绑定/地址前缀变化等），导致既有候选或 active endpoint 失效。  
- Miopunch/POC：若收到一条“可验真”的 dataplane 包，其 UDP 源 `ip:port` 与当前 active remote endpoint 不一致，则认为发生 drift；POC 允许自动切换 active endpoint 并记录解释事件。  
- refs：实现口径（POC 约定）；现象背景可参考 RFC 5128（P2P across NATs）与 RFC 4787（NAT 行为术语）

### NAT Hint（`v4_hint` / `v6_hint`）

- 定义：Miopunch 的**启发式分类**，用于排序/解释，不代表 RFC 的正式分类。  
- Miopunch/POC：
  - `easy`：更可能直连/更可能一次打洞成功
  - `hard1`：更难；可能需要更多尝试/端口预测；（v4 下常与“端口可预测递增”相关）
  - `hard2`：最难；端口不可预测/行为更不稳定，直连概率更低
  - `unknown`：数据不足（不阻断，只降低置信度）
  - v6：若“入站受限/需先出站建表”，按 `hard1` 处理；POC 暂不区分更细 v6 模式  
- refs：RFC 4787（mapping/filtering/hairpin 的底座术语）；RFC 5128（端口分配常可预测的事实背景）

---

## 4) 控制平面（Control Plane）与 MQTT

### 控制平面（`control_plane`）

- 定义：用于交换“连接建立所需控制消息”的逻辑层（入网、在线、候选交换、打洞协调、会话锁等）。  
- Miopunch/POC：mesh 优先（网内转发），MQTT 兜底（无邻居/失联时）。  
- refs：实现口径（POC 约定）

### 数据平面（`data_plane` / `dataplane`）

- 定义：承载实际 payload 的传输层（例如 `kcp` / `quic`），以及其上的能力层（POC：shell）。  
- refs：实现口径（POC 约定）

### MQTT（`mqtt`）

- 定义：基于发布/订阅模型的消息协议。  
- Miopunch/POC：用于控制面“兜底投递”；topic 的可枚举性与元数据泄露风险需要被考虑（因此使用派生 topic + E2E 加密）。  
- refs：OASIS MQTT v5.0 https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.pdf

### MQTT broker（`mqtt_broker`）

- 定义：提供 MQTT Publish/Subscribe 的服务端端点；Miopunch 口径为 `host:port`（POC 默认语义为 `tcp://`）。  
- Miopunch/POC：broker **不可信**，只负责 mailbox；所有控制面消息必须端到端加密 + 可验真。  
- POC：不在 join code 中携带 broker 的用户名/密码/证书等材料（鉴权/TLS 后置再做）。  
- refs：OASIS MQTT v5.0 https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.pdf

### broker_profile（`broker_profile`）

- 定义：Miopunch 的内置 broker “优先级视图”（例如 `global` / `cn`）。  
- Miopunch/POC：仅在未显式配置 `control_plane.brokers` 时生效；用于决定默认连接顺序，不改变控制面 E2E 安全语义。  
- refs：实现口径（POC 约定）

### 最终生效 brokers（`brokers_effective`）

- 定义：本机最终会使用的 broker 列表：若配置了显式 `control_plane.brokers` 则以其为准；否则由 `broker_profile` 选择内置默认。  
- Miopunch/POC：常驻 `up` 只会同时连接前 2 个；invite 生成 join code 时会把 `brokers_effective` 的前 1–2 个写入 code。  
- refs：实现口径（POC 约定）

### invite_brokers（`invite_brokers`）

- 定义：join code 内携带的 1–2 个 broker 端点；joiner 与 approver 在 invite/join 阶段只使用这组 broker 做投递/回包。  
- Miopunch/POC：用于避免 joiner/approver 因本地配置或 DNS/geo 分流导致“落到不同 broker 实例而互相看不见”。  
  - POC：invite 会尽力将 broker 端点固定为确定性的 `ip:port`；若无法解析则保留 hostname 并输出强警告。  
- refs：实现口径（POC 约定）

### msg_id（`msg_id`）

- 定义：控制面消息的高熵随机 ID，用于幂等去重。  
- Miopunch/POC：当同一条消息需要对多个 broker 重复投递时，msg_id 必须保持不变；接收端在去重窗口内只处理一次。  
- Miopunch/POC（格式，敲定）：本体 `16B` 随机；RFC4648 base32(raw, no-pad)，规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）。  
- Miopunch/POC（输入容错，敲定）：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符。  
- refs：实现口径（POC 约定）

### proto_version（`proto_version`）

- 定义：协议版本号。  
- Miopunch/POC：使用 `int`；POC v0 固定为 `0`（后续演进为 `1/2/...`）。  
- refs：实现口径（POC 约定）

### kind（`kind`）

- 定义：控制面消息的“种类名”，用于路由到具体处理逻辑。  
- Miopunch/POC：命名约定为 `snake_case`；未知 `kind` 必须安全忽略（不阻断主流程）。  
- refs：实现口径（POC 约定）

### capabilities（`capabilities`）

- 定义：`hello` 消息中携带的能力摘要（只描述“我支持什么”，不携带实例化的 targets/sessions 细节）。  
- Miopunch/POC：建议形态 `{ cmd:["ping","sh"], data_proto:["kcp","quic"], quic_cc:["bbr","brutal"], connectors:["wsl","ssh"] }`。  
- refs：实现口径（POC 约定）

### state_head（`state_head`）

- 定义：长期态视图的摘要头；用于快速判断“我是否落后/需要 state_pull”。  
- Miopunch/POC：形态 `{ governance_head_b64, decls_head_b64 }`：  
  - `governance_head_b64`：治理快照链头 `snapshot_body` 的 `hash_b64`（base64url no-pad；32B）  
  - `decls_head_b64`：声明集合 head（`set_head_b64`；base64url no-pad；32B）  
- refs：实现口径（POC 约定）

### governance_head_b64 / decls_head_b64（`governance_head_b64` / `decls_head_b64`）

- 定义：治理快照链头 / 声明集合头的摘要 hash。  
- Miopunch/POC：  
  - `governance_head_b64`：`hash_b64 = base64url(no-pad, sha256(canonical_json(snapshot_body)))`（32B）  
  - `decls_head_b64`：`set_head_b64 = base64url(no-pad, sha256(concat(sorted sha256(canonical_json(decl)))))`（32B）  
  - 仅用于“是否需要 pull/是否变化”的快速判断。  
- refs：实现口径（POC 约定）

### decl（`decl`）

- 定义：声明集合（`decls`）的元素；长期态的可验真对象（用于 membership/peers 收敛）。  
- Miopunch/POC（最小结构，敲定）：`{ msg_id, created_at_unix_ms, issuer_peer_id, kind, body, sig_b64 }`；不包含路由字段（`dst_peer_id/hop_limit/...`）。  
- Miopunch/POC（kind，POC 最小）：`approve_member` / `revoke_member`。  
- refs：实现口径（POC 约定）

### 控制面线格式（`control_plane_wire_format`）

- 定义：控制面消息“解密后的明文编码格式”。  
- Miopunch/POC：POC v0 统一使用 UTF-8 JSON；`proto_version` 为 `int`；`kind` 命名为 `snake_case`；最小字段集合：`proto_version`、`route{dst_peer_id,msg_id,hop_limit,created_at_unix_ms,expires_at_unix_ms?}`、`signed{sender_peer_id,kind,in_reply_to?,body,sig_b64}`。  
  - `hop_limit`：POC 固定 `H=3`；取值 `0..H`；`hop_limit>H` 直接丢弃。  
  - 签名覆盖：覆盖 `dst_peer_id`；不覆盖 `hop_limit`（允许转发递减）；签名为 `Ed25519.Sign(priv, sha256(transcript_json))`。  
  - transcript 字段（POC，敲定）：`dst_peer_id + msg_id + created_at_unix_ms + expires_at_unix_ms? + sender_peer_id + kind + in_reply_to? + body`。  
  - `*_b64` 统一为 `base64url(no-pad)`。  
- refs：实现口径（POC 约定）

### 控制面密文 framing（`control_plane_ciphertext_framing`）

- 定义：控制面消息在 MQTT/mesh 上承载的“外层密文 bytes”的打包格式（解密前）。  
- Miopunch/POC（敲定）：`v(1B) || nonce(12B) || ct`；`v=0` 表示 AES-256-GCM；`ct` 含认证 tag。  
- refs：实现口径（POC 约定）

### 去重窗口（`dedup_window`）

- 定义：接收端用于丢弃重复控制面消息的缓存窗口（LRU + TTL）。  
- Miopunch/POC：POC v0 采用 `seen`（容量 `8192`，TTL `10m`）+ `handled_requests`（容量 `1024`，TTL ≥ request 有效期且最小 `10m`）；并丢弃 `abs(now-created_at)>10m` 的消息。  
  - 约束：`invite/approve/join` 的 handled 记录与 `uses` 扣减需最小持久化（覆盖 invite 有效期）；其它 handled 记录允许重启清空（KISS）。  
- refs：实现口径（POC 约定）

### Topic Name / Topic Filter（`mqtt_topic_name` / `mqtt_topic_filter`）

- 定义：Topic Name 用于发布消息的主题名；Topic Filter 用于订阅匹配（含通配符）。  
- Miopunch/POC：我们使用“派生 topic”承载 inbox/presence 等控制面通道。  
- refs：OASIS MQTT v5.0（PUBLISH/ SUBSCRIBE 相关章节）https://docs.oasis-open.org/mqtt/mqtt/v5.0/mqtt-v5.0.pdf

### Inbox（`inbox`）

- 定义：Miopunch 语义：某 peer 的控制面“收件箱 topic”（接收加密控制消息）。  
- 同义词：Mailbox（`mailbox`）与 Inbox（`inbox`）在本文档中等价。  
- refs：实现口径（POC 约定）

### Mailbox（`mailbox`）

- 定义：同 Inbox（`inbox`）。  
- refs：实现口径（POC 约定）

### reply_topic（`reply_topic`）

- 定义：入网（invite/join）阶段的临时回包 topic；approver 把 `membership_bundle` 发回该 topic。  
- Miopunch/POC：必须具备 ≥128bit 有效熵且不包含 `peer_id/name`；建议形态 `reply_topic=base32(raw,no-pad, random16B)` 并写入 topic 时用小写（不要带固定前缀/路径段）。  
- refs：实现口径（POC 约定）

### Presence / last_seen（`presence` / `last_seen`）

- 定义：在线宣告与最近可验真可达时间戳。  
- Miopunch/POC：presence/last_seen 属于控制面消息；可用于 UI 总览的“在线/离线/最近可见”。presence 必须携带 `state_head`（摘要 hash），用于传播视图变化并触发对端 best-effort `state_pull`。  
- refs：实现口径（POC 约定）

---

## 5) 网络身份与成员（Membership）

### peer_id（`peer_id`）

- 定义：节点/设备的稳定标识（用于索引、路由与展示；非密钥）。  
- Miopunch/POC（格式，敲定）：`peer_id = base32(raw,no-pad, sha256(ed25519_sign_pubkey)[:16])`；规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）。  
- Miopunch/POC（输入容错，敲定）：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符。  
- refs：实现口径（POC 约定）

### 身份密钥（`identity_key`）

- 定义：节点的长期身份密钥材料。  
- Miopunch/POC（敲定）：包含 Ed25519（签名）+ X25519（静态 ECDH）；重置 identity 视为新节点（=新 `peer_id`，需重新入网）。  
- refs：实现口径（POC 约定）

### 网络标识（`net_id`）

- 定义：网络命名空间标识；用于派生控制面 topic 命名空间等。  
- Miopunch/POC（格式，敲定）：`net_id = base32(raw,no-pad, sha256(net_secret)[:16])`；规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）。  
- Miopunch/POC（输入容错，敲定）：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符。  
- refs：实现口径（POC 约定）

### 网络密钥（`net_secret`）

- 定义：用于派生 topic / AEAD key 等的根密钥材料（POC：不轮换）。  
- Miopunch/POC：建议为高熵随机（例如 `32B`）；不可直接复用为子密钥，必须经 HKDF 域分离派生。  
- 风险：泄露意味着攻击者可持续订阅/投递到该网络命名空间；POC 阶段的干净恢复通常是“重建网络”。  
- refs：实现口径（POC 约定）

### 入网材料 / 邀请（`invite_secret`）

- 定义：用于 join/approve 的临时入口材料（可设有效期）；用于 E2E 交付 membership bundle。  
- Miopunch/POC：建议为高熵随机（例如 `32B`）；不可直接复用为子密钥，必须经 HKDF 域分离派生。  
- refs：实现口径（POC 约定）

### invite_topic（`invite_topic`）

- 定义：入网（invite/join）阶段的临时入口 topic；joiner 把 `join_request` 发到该 topic。  
- Miopunch/POC：必须具备 ≥128bit 有效熵且不包含 `peer_id/name`；建议形态 `invite_topic=base32(raw,no-pad, random16B)` 并写入 topic 时用小写（不要带固定前缀/路径段）。  
- refs：实现口径（POC 约定）

### Invite ID（`invite_id`）

- 定义：用于索引/文件名的 invite 短标识（不用于安全语义；不是 secret）。  
- Miopunch/POC（建议）：`invite_id = base32(raw,no-pad, sha256(invite_topic)[:16])`（26 字符）。  
- refs：实现口径（POC 约定）

### 治理快照链（`governance_snapshot_chain`）

- 定义：owner 签名的治理快照（owners/admins/recovery/config 等），并以 `prev_hash_b64` 串成链；全网以链头为准。  
- Miopunch/POC（格式，敲定）：  
  - snapshot 由 `snapshot_body + owner_sig_b64` 构成；hash 只对 `snapshot_body` 计算。  
  - `hash_b64 = base64url(no-pad, sha256(canonical_json(snapshot_body)))`。  
  - `owner_sig_b64 = base64url(no-pad, Ed25519.Sign(owner_priv, sha256(canonical_json(snapshot_body))))`。  
- refs：实现口径（POC 约定）

### 治理签名请求（`governance_proposal`）

- 定义：用于 owner 离线签名的“签名请求文件/文本”（proposal）。proposal 本身不需要签名；最终只签 `snapshot_body`。  
- Miopunch/POC（最小字段，敲定）：  
  - `proposal_version`（int；v0 固定 `0`）  
  - `proposal_id`（string；`16B` 随机 → base32(raw,no-pad)；26 字符）  
  - `created_at_unix_ms`（int64）  
  - `net_id`（string，建议；防跨网误签）  
  - `initiator_peer_id`（string）  
  - `initiator_name`（string，可选；仅用于展示）  
  - `snapshot_body`（object；见 `governance_snapshot_chain`）  
  - `prev_snapshot_body`（object，可选但推荐；用于签名端做 diff 展示；需校验其 hash 匹配 `prev_hash_b64`）  
  - `summary`（object，建议；用于展示/解释；非权威，签名端必须复算/校验）  
  - `summary_text`（string，可选；纯展示文本；非权威，签名端必须复算/校验）  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`（治理章节）

### Owner / Admin（`owner` / `admin`）

- 定义：网络治理角色：owner 为超级权限签名 key；admin 为可管理权限 key。  
- Miopunch/POC：任意 admin 节点在通过有效验证后，可执行管理员动作；关键动作必须二次确认。  
- refs：实现口径（POC 约定）

### Revoke / Demote（`revoke` / `demote`）

- 定义：revoke 拉黑某身份公钥；demote 降级角色（例如移除 admin）。  
- Miopunch/POC：  
  - CLI：`miopunch revoke <peer>`（核按钮，二次确认）→ LocalAPI task `revoke_member` → 产生 `revoke_member` decl（永久 tombstone）。  
  - 约束（POC）：只允许撤销普通 member；admin/owner 相关变更走 owner-signed snapshot 链（后置）。  
  - 回滚不作为 POC 目标；误操作通过“二次确认 + owner/admin 验证流程”降低概率；需要回来则换新身份 key 再 join。  
- refs：实现口径（POC 约定）

### Recovery Code（`recovery_code`）

- 定义：一次性恢复 owner key 的高熵材料（等价于 root 私钥种子）；消耗后从 allowlist 移除。  
- refs：实现口径（POC 约定）

---

## 6) 传输与拥塞控制（Dataplane）

### KCP（`kcp`）

- 定义：快速可靠的 ARQ 传输协议实现（算法库），面向低时延；不自带加密。  
- Miopunch/POC：作为可选数据面承载；加密由上层负责（POC 统一走业界最佳实践）。  
- refs：KCP README https://github.com/skywind3000/kcp/blob/master/README.en.md

### QUIC（`quic`）

- 定义：基于 UDP 的安全传输协议，提供流、多路复用、低时延建连与路径迁移等能力。  
- refs：RFC 9000 Abstract https://www.rfc-editor.org/rfc/rfc9000

### 拥塞控制（Congestion Control，`congestion_control` / `cc`）

- 定义：控制发送速率/在途数据，以适应网络瓶颈并减少排队/丢包。  
- Miopunch/POC：在 `quic` 下对外暴露 `--quic-cc`（例如 `bbr` / `brutal`）。  
- refs：QUIC 相关 RFC + 算法文档（见下）

### BBR（`bbr`）

- 定义：模型驱动拥塞控制，以测得的带宽与往返时延为核心来控制发送速率与在途数据量。  
- refs：IETF draft-ietf-ccwg-bbr（版本可能更新）https://datatracker.ietf.org/doc/draft-ietf-ccwg-bbr/

### Brutal（HY2，`brutal`）

- 定义：Hysteria 的固定速率型拥塞控制：不因丢包/RTT 波动主动降速；达不到目标速率时按丢包率补偿提速（对带宽配置准确性敏感）。  
- refs：Hysteria 2 文档（server config / congestion control）https://v2.hysteria.network/docs/advanced/Full-Server-Config/

---

## 7) Shell / Session（POC 主能力）

### Target（`target`）

- 定义：可被接入的“终端目标”，例如 `wsl:<distro>`、`ssh:<name>` 等。  
- Miopunch/POC：用户可重命名 target；UI/CLI 默认展示用户命名而非实现细节。  
- refs：实现口径（POC 约定）

### Session（`session`）

- 定义：一次可重连、可恢复现场的交互会话。  
- Miopunch/POC：以 `tmux` 承载“现场”；断线/客户端重启可恢复；被控机系统重启则现场自然消失。  
- refs：实现口径（POC 约定）

### 单写者锁（Single-writer，`single_writer_lock`）

- 定义：同一 target 在同一时间只允许一个写者接入（避免并发输入破坏现场）。  
- refs：实现口径（POC 约定）

### PTY / ConPTY（`pty` / `conpty`）

- 定义：伪终端机制；Windows 的等价机制为 ConPTY。  
- Miopunch/POC：用于把“远端输入/输出”绑定到本地终端体验；Windows 侧需实现 ConPTY 路径。  
- refs：实现口径（POC 约定）；Windows 文档（后续补）

---

## 8) 可解释性（Explainability）相关术语

### Sidecar / Best-effort（`sidecar_best_effort`）

- 定义：可解释性相关的“回执/回报/统计/解释事件”都属于 sidecar，best-effort（可缺失、延迟、丢失、乱序）。  
- 硬规则：sidecar 缺失**不得改变**打洞/建链/握手/能力层流程与结果；只能降低解释置信度与可展示信息量。  
- refs：实现口径（POC 约定）

### 阶段机（Stage Machine，`stage_machine`）

- 定义：可解释性统一的“进度阶段”，用于 CLI/UI 同口径呈现（8 阶段）。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`（可解释性章节）

### 诊断树（Diagnosis Tree，`diagnosis_tree`）

- 定义：每个阶段独立的诊断决策树；失败时输出命中的路径与叶子结论。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`（可解释性章节）

### Reason Code（`reason_code`）

- 定义：诊断叶子结论的稳定代码（例如 `CP_DNS_FAIL`）；用于日志、UI 展示、测试断言与文档引用。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`（叶子结论词典）

---

## 9) 本机 HTTP 面板 / LocalAPI

### LocalAPI（`localapi`）

- 定义：本机 CLI/UI 与常驻 `miopunch up` 通信的本机 IPC API。  
- Miopunch/POC：承载“CLI 命令执行（除 up 外）”、“面板读路径（状态/阶段机/报告）”等；只允许本机访问（Linux unix socket / Windows named pipe）。  
- refs：实现口径（POC 约定）

### Task（`task`）

- 定义：可被创建/查询/观察进度的一次“长操作”执行实例。  
- Miopunch/POC：`join/ping/approve/revoke_member/sh_attach` 等统一以 task 承载；通过阶段机与诊断树对外解释；完成后可生成 Markdown 报告。  
- Miopunch/POC：`task.kind` 命名约定为 `snake_case`，与 CLI 命令语义对齐。  
- refs：实现口径（POC 约定）

### Shell Attach（`sh_attach`）

- 定义：进入/恢复对端 shell 的交互 task（以 tmux session 表示“现场”）。  
- Miopunch/POC：I/O 通过 WebSocket attach（`GET /api/v0/tasks/<task_id>/ws`）承载；阶段/诊断仍通过 SSE 事件流提供。  
- Miopunch/POC：WebSocket 帧约定（`Sec-WebSocket-Protocol: miopunch.sh.v0`）：
  - binary frame：PTY/ConPTY 原始字节流（stdin/stdout）  
  - text frame：控制 JSON（POC 最小：`winsize{cols,rows}`）  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`

### Shell List（`sh_ls`）

- 定义：列出对端 shell 的可用 targets / sessions 的只读 task（对应 CLI `miopunch sh ls`）。  
- Miopunch/POC：`miopunch sh ls` 通过 LocalAPI 创建 `sh_ls` task；HTTP 面板不开放该写操作，但 `sh_attach` 失败诊断可附带“可选 targets/sessions”。  
- Miopunch/POC：请求通道为控制面 RPC（mesh-first + MQTT 兜底），不要求先打洞/建链。  
- Miopunch/POC：结果允许缓存 TTL；失败时允许展示“上次缓存（可能过期）”。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`

### `miopunch.sh.v0`（WebSocket 子协议，`miopunch_sh_v0`）

- 定义：`sh_attach` 的 WebSocket 子协议标识（`Sec-WebSocket-Protocol`）。  
- Miopunch/POC：用于约束“binary=text frame 语义”，避免未来演进时互不兼容。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`

### Task ID（`task_id`）

- 定义：task 的稳定引用 ID。  
- Miopunch/POC：随机不透明 ID（高熵随机）；用于引用、下载报告与文件名；task 列表排序以 `created_at` 为准。  
- Miopunch/POC（格式，敲定）：`16B` 随机 → RFC4648 base32(raw,no-pad)；规范输出大写（解析大小写不敏感）；固定 `26` 字符（`[A-Z2-7]`）。  
- Miopunch/POC（输入容错，敲定）：解析时允许空格/短横线分组；输出一律规范化为大写无分隔符。  
- refs：实现口径（POC 约定）

### SSE（Server-Sent Events，`sse`）

- 定义：HTTP 服务端到浏览器的单向事件流（`text/event-stream`），用于推送实时状态而无需轮询。  
- Miopunch/POC：用于面板“状态/事件时间线/阶段机”的实时更新；POC 不做周期轮询，SSE 不可用则提示刷新/重试（轮询后置）。  
- refs：https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events

### Snapshot（`snapshot`）

- 定义：用于“断线重连后恢复视图”的快照事件。  
- Miopunch/POC：SSE 连接建立后必须先发 `snapshot`；POC 不支持基于 `Last-Event-ID` 的增量补发。  
- refs：实现口径（POC 约定）

### CSRF Token（`csrf_token`）

- 定义：用于防御跨站请求伪造（CSRF）的随机 token；通常与 cookie/session 绑定并要求同源携带。  
- Miopunch/POC：面板写操作仅允许 `invite/join/sh_attach` 三类；POC 不强制引入 CSRF token（后置），通过 Host/Origin/Referer 同源校验 + 写操作白名单避免跨站/误触发。  
- refs：https://owasp.org/www-community/attacks/csrf

### UI Token（`ui_token`）

- 定义：HTTP 面板对外监听时，用于保护写接口与 WebSocket 的共享 token（避免本机以外访问/误触发）。  
- Miopunch/POC：后置；仅当未来显式开启“非 loopback 监听”时才需要。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`

### Bech32m（`bech32m`）

- 定义：一种带强校验的编码格式（带 checksum），适合人类输入/复制与二维码承载。  
- Miopunch/POC：用于 `invite/join` code 的编码；输出全小写；输入允许带分隔符（解析前去掉）；HRP 固定为 `miopunch`；code 携带 `code_type+version`；长度硬上限 `1024` 字符。  
- refs：https://github.com/bitcoin/bips/blob/master/bip-0350.mediawiki

### Issuer（`issuer`）

- 定义：某个短期/临时材料（如 invite code）的签发者/发码者标识（用于用户可解释性展示）。  
- Miopunch/POC：用于展示“我在连谁”；优先显示 `name`，否则显示 `peer_id` 前 8；不用于安全校验（安全以签名/密钥为准）。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`

### Ring Buffer（环形缓冲区，`ring_buffer`）

- 定义：固定容量的循环队列；写满后覆盖最旧数据。  
- Miopunch/POC：面板保留“最近 N 次 task 报告”采用 ring buffer 语义（N 可调）。  
- refs：实现口径（POC 约定）

---

## 10) 密钥派生与加密（POC 口径）

### HKDF（`hkdf`）

- 定义：基于 HMAC 的密钥派生函数（KDF），把一个输入密钥材料扩展/派生成多个用途各异的密钥。  
- Miopunch/POC：用于从 `net_secret`/`invite_secret` 派生 topic 命名材料与 AEAD key；不同用途通过 `info` 字段做域分离。  
- refs：RFC 5869 https://www.rfc-editor.org/rfc/rfc5869

### 域分离（Domain Separation，`domain_separation`）

- 定义：同一根密钥材料在不同用途下派生不同子密钥，避免“一把钥匙到处用”导致的联动风险。  
- Miopunch/POC：通过 HKDF 的 `info` 做域分离（固定前缀 + 用途名 + 版本），例如 `miopunch/v0/topic.inbox`、`miopunch/v0/aead.ctrl.group`。  
- refs：实现口径（POC 约定）

### base32（raw, no-pad，`base32_raw_no_pad`）

- 定义：RFC4648 的 base32 编码，使用 raw/no-pad 形式（不带 `=` padding）。  
- Miopunch/POC：`peer_id/net_id/msg_id/task_id` 等统一使用 base32(raw,no-pad)；规范输出大写（解析大小写不敏感）；解析允许空格/短横线分组；输出规范化为大写无分隔符。  
- refs：RFC 4648 https://www.rfc-editor.org/rfc/rfc4648

### canonical_json（`canonical_json`）

- 定义：用于签名/哈希输入的“确定性 JSON 编码”。  
- Miopunch/POC：`canonical_json(x) = json.Marshal(x)`（固定 struct 字段顺序；禁止 `map` 参与哈希输入）；集合语义必须先对元素 hash 排序，再计算 head。  
- refs：实现口径（POC 约定）

### base64url（no-pad，`base64url_no_pad`）

- 定义：RFC4648 的 URL-safe base64 变体（字符集 `A–Z a–z 0–9 - _`），并去掉 `=` padding。  
- Miopunch/POC：所有 `*_b64` 字段统一使用 `base64url(no-pad)` 规范输出；解析可容错接受 padding/standard base64。  
- refs：RFC 4648 https://www.rfc-editor.org/rfc/rfc4648

### AEAD（Authenticated Encryption with Associated Data，`aead`）

- 定义：带认证的加密模式；在解密时同时验证密文未被篡改；可带“关联数据（AAD）”参与认证但不加密。  
- Miopunch/POC：控制面消息统一使用 AEAD 做端到端机密性与完整性（并配合签名做身份绑定）。  
- Miopunch/POC：AES-GCM nonce 固定 `12B` 随机；若需要进入 JSON，使用 `nonce_b64=base64url(no-pad)`。  
- refs：概念术语（POC 约定）

### AES-256-GCM（`aes_256_gcm`）

- 定义：AEAD 算法（AES-GCM，256-bit key）。  
- Miopunch/POC：控制面 AEAD 固定使用 `AES-256-GCM`（不做算法协商）。  
- refs：NIST SP 800-38D（AES-GCM）

### 点对点私密（Pairwise，`pairwise_encryption`）

- 定义：只有目标收件人能解密的加密域（与“全网可读/成员可解密”相对）。  
- Miopunch/POC：短期敏感信息（STUN/候选/端点/端口等）必须使用 pairwise；POC 采用 X25519 ephemeral-static + HKDF 派生一次性 AEAD key。  
- refs：实现口径（POC 约定）

### 身份绑定（Identity Binding，`identity_binding`）

- 定义：把数据面（TLS/传输层）对端身份，绑定到控制面已验真的 peer identity（防 MITM）。  
- Miopunch/POC：数据面使用 TLS 1.3 双向认证；对端证书公钥必须匹配 membership 中该 peer 的 identity 公钥；证书首次生成落盘复用；reset=新 identity=新证书。  
- refs：`docs/notes/2026-04-15-alpha-product-discussion.md`
