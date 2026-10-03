# 普通浏览器旁信道 + WebRTC 的收益与复杂度评估

日期：2026-10-03。状态：只读研究与下一阶段建议；不是产品决策、实施计划、实测结果或 Plane 任务。仅新增本文，不修改既有实现。本报告按 `research` 技能核对官方标准、官方浏览器资料和项目当前源码；采用 `codebase-design` 的 Module / Interface / Seam / Adapter 词汇。IWA 不在研究范围内。

## 判断

**值得一个小而真实的原型；现有证据还不足以立即正式引入，更不足以移除现有数据 relay 回退。** 新增价值是普通浏览器与 native 之间真正的 UDP 端点直连，直连成立时不必让业务经过数据 relay。它不新增身份、成员资格、入网材料持久化或 LogSync：现有普通浏览器已通过这些行为。延迟、吞吐、relay 流量与费用可能改善，但改善多少、在哪些网络可达、额外 CPU/内存/电量是否合算，都需要本项目实测。

如果目标只是“普通浏览器能入网、同步和访问成员”，MIOPUNCH-65 已提供真实可用基线，RTC 的优先级低于这些行为本身。如果目标包含“浏览器在直连可行的网络中无需数据 relay”，RTC 正好补上当前浏览器的能力空缺。若目标变成“长期后台常驻”或“任意 NAT 下都强保可达”，RTC 不能单独交付该目标：普通浏览器生命周期限制依然存在，直连失败也仍需要另一个获准数据路径，或明确暂不可达。[现有浏览器公共 relay 资格](2026-10-03-browser-official-public-relay.md)、[Chrome Page Lifecycle](https://developer.chrome.com/docs/web-platform/page-lifecycle-api)、[ICE RFC 8445](https://www.rfc-editor.org/rfc/rfc8445)

## 已有事实、推断与缺口

| 性质 | 当前能支持的结论 | 不能扩展成的结论 |
| --- | --- | --- |
| 项目端到端证据 | MIO65：普通 Chromium、正常公共证书验证、独立身份、一次 native 手工批准、授权与 secret 保存、成员业务、原始签名记录及 body 的真实 LogSync、刷新恢复；禁用唯一 relay 后明确无业务且无绕过访问 | 另一浏览器、移动后台、RTC、任意 NAT 或公共 relay SLA 已完成 |
| 今日入口观测 | Chrome 153 下 EMQX `8084/mqtt`、HiveMQ `8884/mqtt` 的真实 MQTT 3.1.1 Pub/Sub 及浏览器 WSS ↔ native TCP 1883 双向通过；Mosquitto `8081/mqtt` 本次超时 | 当前 miopunch 的 MIO1/32 字节 route、QUIC/TLS、member proof、路径提交、LogSync 经这些公共 broker 已通过 |
| 今日 IRC 观测 | Ergo 官方 testnet 的 WSS HTTP 101、`text.ircv3.net`、001 注册通过；没有 JOIN、PRIVMSG | MIR1 分片、公共限流下完整 QUIC 握手或 miopunch 成员通信已通过 |
| 官方标准事实 | DataChannel 使用 SCTP/DTLS/ICE/UDP，可提供不排序、零重传的 UDP-like 消息载体 | 浏览器暴露原生 UDP socket，或能直接接现有 native QUIC UDP listener |
| 设计推断 | 在原 QUIC/TLS 下增加 RTC 自定义载体，有机会保留原 Connection、成员证明和上层 stream | 只添加一个库就完成源码和语义兼容；WebRTC 的 PeerConnection 成功等于项目业务成功 |
| 必须实测 | 独立 NAT 下无 TURN/数据 relay 的直连、原 Connection 连续性、实际省去 relay 字节数、成本与失败收敛 | 未测量的成功率、毫秒、MB、性能百分比 |

项目事实来源：[MIO65](2026-10-03-browser-official-public-relay.md)、[MIO50](2026-10-02-browser-manual-admission.md)、[MIO51](2026-10-02-browser-control-replica.md)。今日入口的原始观测来自父研究已有记录，已保存为 [MQTT 浏览器/native 回执](../evidence/browser-wss-resources-2026-10-03/mqtt-browser-native.json)与 [IRC 浏览器入口回执](../evidence/browser-wss-resources-2026-10-03/irc-browser-entry.json)；本报告没有重跑。[DataChannel 标准 RFC 8831](https://www.rfc-editor.org/rfc/rfc8831)

README/CONTEXT 中部分“尚无 runtime”描述仍是早期设计文本；当前源码和后续资格笔记明确存在实际 Rust/native/WASM 实现。本研究以接受的产品规则作为约束、以当前源码和具名资格作为实现证据，不把早期未来时措辞当成代码不存在。

## 接入位置：新的路径能力，不是第二套网络

可保持的流程如下；首次邀请分支和已有成员分支有不同准入资格，不能混成统一“先持 secret 才能开始”的过程。

```text
加载原身份、材料、资源选择
  → 浏览器接获准 MQTT-WSS / IRC-WSS；native 接同一逻辑服务
  → 现有 route/envelope 递送受限的真实 QUIC/TLS，核对目标身份
  → 首次邀请：申请 Connection A 仅 AdmissionRequest scope，批准与材料递送/保存
    材料齐备后另建 Member Connection M；已有成员直接建立 M
  → M 上的当前网络 scope + TLS exporter 绑定的成员 proof
  → 原受保护 M 内有界交换 RTC SDP / ICE 参数 / 尝试绑定
  → browser RTC + native ICE/DTLS/SCTP 建立候选载体
  → 原 Connection 在该载体上做真实 QUIC 路径验证
  → 双方重新核查身份、成员、secret generation、资源资格并提交
  → 普通成员 Connection / 原 stream / LogSync；继续既有选路与失败语义
```

RTC 对端 native 必须实现可与浏览器互通的 WebRTC 协议。这里的“UDP 直连”指 RTC 的物理端点间 UDP 路径；业务仍是 `QUIC → DataChannel → SCTP → DTLS → ICE/UDP`。它与当前 native↔native 的直接 QUIC/UDP 路径不同，也不能把 RTC candidate 直接交给 native NAT engine 就认为互通。浏览器对的内建 ICE 可以局部使用，不意味着替换 native↔native 已有 UDP/TCP/NAT engine，更不意味着项目要自写全套 ICE。[RFC 8831](https://www.rfc-editor.org/rfc/rfc8831)、[既有 native message bootstrap](../../src/message_bootstrap.rs)

合适的 Seam 仍是现有端点自定义载体及数据资格释放：RTC Adapter 满足 QUIC datagram 的 Interface，上层 Module 继续只使用 Endpoint / Connection / stream。这样复杂度集中在载体、私密协商和路径资格处，获得 Locality。让业务 SDK 在 relay Connection 与另一条 RTC Connection 间自行切换，会把握手、身份、授权、关闭及恢复复杂度泄漏给每个调用方，且不能证明原 stream 延续。当前 `BrowserMemberConnection` 本身已有原 Connection handle，适合从此处观测连续性。[浏览器成员 Interface](../../browser-probe/src/member_connection.rs)、[自定义载体 Interface](../../vendor/iroh/src/socket/transports/custom.rs)

当前源码确实需要改的地方：

1. **入口与配置。** `BrowserNetwork` 目前仅配置 relay，空获准列表即启动失败，拨号只循环 relay URL；`BrowserGate::AcceptData` 的自定义数据 ID 列表为空。必须加入浏览器 WSS 载体、资源用途与实际拨号入口。native 原 MQTT/IRC origin 与浏览器 WSS URL 可能表示同一 broker/IRCd，但字符串不相等；公共 service tag、32 字节 route、topic/channel 与封套必须保持协议相容。TLS/WSS 的端点别名不能绕过用户 disable，也不能一改 URL 就误称支持原生加密 IRC。[browser network](../../browser-probe/src/network.rs)、[MQTT 集成](2026-10-02-mqtt-bootstrap-validation.md)、[IRC 集成](2026-10-02-irc-bootstrap-validation.md)
2. **有界私密协商。** 当前 `message_bootstrap` capabilities v2 只有 TCP/UDP/relay；RTC 需要明确能力协商、SDP/candidate 上限、期限、并发尝试与版本行为。ICE credentials、DTLS fingerprint、candidate 和尝试绑定应在预期双方的原安全会话内，不能裸发公开 channel/topic；RTC 标识不成为新节点身份。过期消息、另一持邀请者、旧尝试或终止 Connection 的消息不能提交当前路径。WSS 只保护到服务器，不能替代这个端点私密性。[message bootstrap 源码](../../src/message_bootstrap.rs)、[ADR 0005](../adr/0005-side-channel-control-bootstrap.md)、[WebRTC 安全标准](https://www.rfc-editor.org/rfc/rfc8827)
3. **首条路径释放与修复必须区分。** 首次 Member 数据升级在 restricted `before_data_release` 内；额外 RTC 自定义路径应走该阶段的 `open_control_path`，由原 dialing side 开路。`open_data_transport_path` 是已 Ordinary 的修复入口。当前 `ControlSession::commit_data_path` 最终进入 IP-only commit，要求已建立 UDP IP path 并关闭全部 custom path，直接复用会连 RTC 一起关闭。需要通过严格数据资格释放保留显式注册 RTC ID，退休 MQTT/IRC 信令路径，并对排队报文、远端重新开路和取消同样生效。`AcceptData` 与最终 authorize 不能把“custom”整体视为业务许可。[control session](../../src/control_session.rs)、[endpoint hooks](../../vendor/iroh/src/endpoint/hooks.rs)、[实际 commit/authorize](../../vendor/iroh/src/socket/remote_map/remote_state.rs)
4. **路径政策与运行寿命。** RTC 是 direct、relay 还是不可用，须由实际选中 candidate 和获准用途判断，不能仅因 CustomAddr 就取得 `PreferDirect`。候选删除、实际地址变化、关闭、取消、成员移除、secret rotation 必须传到原 gate 与载体。RTC 对象、native UDP 端口、DTLS/SCTP 状态、timer、队列与 WASM/JS 事件都要归同一个拥有者关闭；原生维护预算不能未经验证直接等同于 browser 可常驻维护。[既有路径维护](2026-10-02-connection-path-maintenance.md)、[节点维护](2026-10-02-node-maintenance.md)、[资源配置](../../src/resources.rs)

预算接入是另一个真实成本：当前 native maintenance 的 conservative/active 都有 `contact_timeout_ms=2000`，message bootstrap 总 operation 上限为 45s，browser 的 `TIMEOUT` 为 12s；这些不同层的上限不表示调用一定可等待全部 45s。RTC 新增 SDP/ICE/DTLS/SCTP 阶段可能先被外层 contact 或 proof/stage 期限取消。下一阶段须观测各阶段耗时与剩余预算，沿同一 coordinator 传递取消并复用既有退避；不能全局延长超时、建立第二个 RTC recovery engine 或吞掉外层取消来得到漂亮成功率。上述数值是当前源码事实，不是新的默认值或 SLA。[native maintenance](../../src/network_maintenance.rs)、[message bootstrap](../../src/message_bootstrap.rs)、[browser TIMEOUT](../../browser-probe/src/network.rs)

## 实际收益与不可免除的成本

| 项目 | 可能取得的收益 | 新增成本 / 约束 |
| --- | --- | --- |
| 数据路线 | 成功直连时业务绕过数据 relay，降低对 relay 带宽与在线状态的依赖 | 引导 broker/IRCd 仍是控制资源；不能称完全无外部设施 |
| 延迟、吞吐、费用 | 少一个 relay 绕路，可能降低 RTT、减少 relay 转发字节及部署成本 | RTC 封装、额外握手及路径质量可能抵消收益；当前没有数字结论 |
| 普通浏览器能力 | 不安装 native、不用 IWA，即可获得浏览器支持的端点 UDP 通信 | native 增加 ICE/DTLS/SCTP 栈；浏览器只能通过 RTC Interface，不能移植全部 native socket 手段 |
| 安全与上层复用 | 可保留现有 QUIC 身份、TLS exporter proof、成员验证、LogSync 与原 stream | 外层 DTLS 与内层 QUIC 加密并存；RTC fingerprint 必须绑定安全协商；DTLS 安全不是成员授权 |
| 传输与调度 | 选 UDP-like 不排序零重传模式，避免外层可靠有序传输再复制内层重传 | SCTP 拥塞控制仍存在，内层 QUIC 也有拥塞控制；没有由此消除双层 CC，损失/背压交互需实测 |
| 生命周期 | 前台有活跃页面时可使用，刷新恢复原身份和材料 | 页面 freeze/discard、移动切后台、事件延迟/迟到与取消需要处理；RTC 不提供长期后台保证 |

DataChannel 的 buffer 指标、低水位事件、message size 与异步 error/close 是浏览器标准 Interface 的一部分；发送被接受不代表对端已收到。Adapter 应限定 datagram 大小、队列字节、并发 RTC 对象和握手数量，处理 send 背压及 native segmentation/GSO，不能用库默认的大消息/大缓冲掩盖项目边界。这些是需要设计和测量的成本；本报告不报未经项目验证的库体积或性能数字。[W3C RTCDataChannel](https://www.w3.org/TR/webrtc/#rtcdatachannel)

Chrome 官方说明 frozen 状态会暂停可冻结任务，discard 可无回调发生，移动端终止事件不可靠；它对 frozen 的建议包括关闭 WebRTC/WebSocket、IndexedDB 与释放 Web Locks。WebRTC 活跃可能影响部分回收策略，但不是承诺进程永远存活。因此必须保存 durable material、正确失效旧 Connection，再重开新的会话，不能把页面关闭记成 Leave。[Chrome 生命周期说明](https://developer.chrome.com/docs/web-platform/page-lifecycle-api)

## 三种产品行为比较：这里只列选择，不作决定

| 方案 | 用户行为与成功条件 | 失败表现与取舍 |
| --- | --- | --- |
| 现有 browser WSS data relay | 已有独立身份、邀请、批准、保存、同步；获准 relay 上成员数据成功即可使用 | relay 不可用/全部禁用时明确暂不可达；这是当前实测基线 |
| MQTT-WSS/IRC-WSS + RTC，保留获准 data relay | 保持同一入网规则；可经信令引导 RTC，也可已有 relay 先使用并尝试改善路径 | 直连失败可使用获准数据回退；不需要新增 mandatory TURN。增加路线能力与实现复杂度，最大程度保留现有可用行为 |
| 从开始彻底移除 data relay fallback | 仅当 RTC direct 数据路径真的成立才获得正常成员数据；材料可已保存，业务仍不可用 | 控制入口能通但 UDP 被阻/候选不可达时失败；去掉数据回退是可达性与交付行为变化，不能以“RTC已接入”为由顺带决定 |

TURN 是数据 relay，不是仅地址观察；如配置 TURN，仍须 DataRelay 资格，若禁用则不能生成/采用 relay candidate。不配置 TURN 是合法的直连资格验证选择，并非 ICE 无法运行；代价是某些拓扑下没有获准数据路径。已有 iroh data relay 可承担项目的获准回退，无需为了 RTC 强加一个 TURN 依赖，但 MQTT-first 到它的同一 Connection 路径仍需对应场景资格。[ICE](https://www.rfc-editor.org/rfc/rfc8445)、[资源与路径政策](2026-10-01-resource-policy-validation.md)

首次加入可保留现有 AdmissionRequest：信任来自邀请指定 contact 与原 QUIC 身份，不要求尚未获准者预先持有 network admission secret。批准后在受限会话中递送并分别验证保存两份材料；只有正常 Member/LogSync/business 阶段才要求 current secret proof 与当前成员资格。**材料已存而 RTC 数据失败，应报告入网完成但暂不可达，不追加批准、不生成替代身份，也不宣称业务成功。**新邀请若增加浏览器可用 route，只属于联系信息与编码相容性，不得顺带包含长效 admission secret。[BrowserGate 原规则](../../browser-probe/src/network.rs)、[入网成功定义](../../CONTEXT.md)、[材料完成规则](2026-10-01-admission-completion-validation.md)

## 真实失败边界

- WSS 连接/注册/订阅成功只说明入口可用。今日 MQTT 的应用自 echo 只说明 broker Pub/Sub；RTC `open` 只说明载体状态。正常成员接受必须是目标认证、当前 proof、原 QUIC 路径提交，以及另一真实成员的业务回复和验证保存的 LogSync。
- 控制 live、RTC data loss 时不得把业务流回灌 MQTT/IRC；fallback 只走获准 DataRelay/direct。原 Connection 实际关闭后旧 stream 失败，新的 RTC/QUIC 会话不是复活；源范围历史仍可之后重新同步。[已有故障隔离证据](2026-10-02-connection-path-maintenance.md)
- RTC 不是 QAD/观察资源，不能仅交换 native 旧观察端口假装浏览器 ICE 成功。物理端口和 candidate 由实际 RTC 端点拥有；capture 与浏览器 selected candidate-pair 共同证明 direct。
- 当前 IRC 本地资格是一片 128 bytes、最多 16 片、每 10ms 一片、queue expiry 500ms、assembly 3s；实际五命令 burst/2 commands/s fakelag 曾使完整 bootstrap 失败。增加 RTC SDP/candidate 更需要有界承载和速率适配，公共注册成功没有免除这个成本。[IRC 实测边界](2026-10-02-irc-bootstrap-validation.md)
- Ergo testnet 官方仅邀请非滥用测试并记录全部流量，本次不能将其指定为生产默认。Libera 禁止未获明确许可的 bot 间基础设施转发，不应选成默认，也不在本研究中联系 staff。OFTC 官方说明浏览器通过 Socket.IO 协商其 HTTPS↔IRC gateway；推断它不能被当作原始 IRC-WSS 的即插即用 endpoint。[Ergo testnet](https://testnet.ergo.chat/)、[Libera 基础设施政策](https://libera.chat/policies/#use-of-liberachat-as-infrastructure)、[OFTC WebChat/WebIRC](https://www.oftc.net/WebChat/)

## 下一阶段最小验证预算与停止条件

下面是最多四个有依赖的竖切建议，**本报告没有运行它们**。先采用可控 MQTT-WSS，IRC 作为后续独立资格，避免同时扩宽载体、库、NAT 与平台。每个切片一个正常故事、一个关键故障；只回归受影响的 proof/gate/resource/connection/WASM，不继承旧 Go/OpenSpec 全矩阵，也不为每次更动重跑不涉及的慢 NAT。运行 deadline、总候选数/字节、队列字节、对象数量和清理检查先写入证据要求；具体数值在实现前与现有协议尺寸核对，不把尚未测量的预算写成 SLA。

| 切片 | 最小证明 | 失败后怎么决定 |
| --- | --- | --- |
| 1. MQTT-WSS 的真实受限 bootstrap | 真实浏览器↔native、MIO route/envelope、QUIC/TLS 目标身份；同邀请另一身份不能读/冒充私密协商；首次请求在没有 admission secret 时仍可批准并保存原材料 | 若只 Pub/Sub 能通，或必须新造前 QUIC 密码/身份层才可接，就先停 RTC 扩展，报告不相容点 |
| 2. 真正分离 NAT 的 RTC direct | 独立持久身份、双方无公网 peer listener、TURN 与 data relay 禁用；按各条成员/replica Connection 分别证明原 stable ID、原 stream 和新鲜 ACK，允许沿用不同 ALPN 分别建连；真实 member PONG（带新挑战）及非空签名 body 的 LogSync 验证保存；capture + selected ICE pair 指明双方端点 UDP | 若必须新 Connection/facade 才能完成同一条成员连接的换路，或只有 loopback、TCP/TURN candidate，不能通过“原连接直连”门槛；停止扩大平台/网络矩阵 |
| 3. data-loss / control-live 与获准回退 | 物理 RTC UDP DROP、MQTT-WSS 保留；旧 stream 在实际 lifetime 内明确失败或切换到已获准且已验证的数据路径；禁用 relay 不得访问。另在资格释放附近取消/移除成员，迟到 RTC 回调不复活旧路 | 信令开始承载业务、禁用被绕过、旧 stream 被新会话假装恢复，均暂停正式引入；在现有路径/gate修正后仅重验此切片 |
| 4. 必要的前台成本和生命周期基线 | 对比现有 WSS relay 与 direct RTC 的相同应用负载、同一物理网络与 native peer；测 setup/成员首回复、RTT/goodput、broker/relay/peer bytes、native与browser进程 CPU/RSS、队列峰值和关闭后对象/端口。若 mobile 是下一交付对象，再补实际 Android native↔普通手机浏览器前台及一次 freeze/reopen | 若更快/省 relay 的路径仍使资源开销不符合目标设备已约定预算，保留原型而不推广；若只有前台成立，则只记录前台资格，不升级为后台保证 |

原 stable ID 是本地保留 handle 生命周期的观察值，不是 wire identity；两端都要观测各自原 handle，且验证原 stream 内容与 fresh QUIC ACK，不能用相同整数代替安全绑定。跨 NAT 一次成功不产生 NAT 成功率；性能重复仅用于同一场景的测量稳定性，直到新的实际问题需要更广样本才扩大范围。[同一 Connection 的现有资格方法](2026-10-02-connection-path-maintenance.md)

正式引入前的 YES 门槛是：上述前三个切片真实通过，第四个给出目标场景中可解释的收益与有界开销；库与 browser/native 版本被固定，资源禁用和取消无绕过，且维护者接受新增 RTC 栈与生命周期维护成本。NO/延期门槛是：只能重建会话、直接路径在目标场景不成立、成本超过目标设备预算，或实际目标需要后台/强保可达而仍未有对应承载。先完成小原型取得这些证据，比现在承诺“全面换 RTC”更能快速回答是否值得；native↔native 原 UDP/TCP 不需要陪同替换。
