# 普通浏览器 WebRTC 载体的库复用与接入成本

2026-10-03。只读源码评估；本次未实现 RTC、未编译候选库、未运行 RTC/NAT 或性能实验，未创建任务或作出产品选型决议。范围是普通浏览器通过 MQTT-WSS / IRC-WSS 建立受保护控制交换，再将 WebRTC DataChannel 接入现有 iroh/noq 会话；不包含 IWA。

**研究建议：保留现有 Endpoint/Connection、准入和路径门控，新增共享报文桥、浏览器 `web-sys` RTC adapter、原生 RTC adapter，以及受限控制流内的协商 coordinator。** 当前发布的 `iroh-webrtc-transport` 可提供底层源码参考，但不能直接 `cargo add` 后获得与本仓库兼容的同一连接升级。仅以 Browser↔native 为目标，原生端优先评估 `webrtc` 0.21 的完整异步封装；确实需要掌握原生物理 socket 和驱动循环时再评估 `str0m` 0.24。这是按集成边界作出的推论，不是已验证的成功率、性能或维护成本排名。[发布依赖][a-cargo]、[原生 facade][a-facade]、[webrtc driver][w-driver]、[str0m I/O 边界][s-io]支持这些判断。

## 本次固定的证据

官方 docs.rs 的 latest 页面在检查时分别指向 `iroh-webrtc-transport` **0.1.0-alpha.2**、`webrtc` **0.21.0**、`str0m` **0.24.0**、`datachannel` **0.16.1**。以下源码判断采用固定发布包而非随时变化的 main；alpha.2 的发布仓库是 **SuddenlyHazel/iroh-webrtc-transport**，不与另一仓库的早期 skeleton 混用。[alpha.2 发布信息](https://docs.rs/crate/iroh-webrtc-transport/latest)、[webrtc 发布信息](https://docs.rs/crate/webrtc/latest)、[str0m 发布信息](https://docs.rs/crate/str0m/latest)、[datachannel 发布信息](https://docs.rs/crate/datachannel/latest)。

| 发布包 | 包内 `.cargo_vcs_info.json` 提交 | 本地发布包 SHA-256 |
| --- | --- | --- |
| iroh-webrtc-transport 0.1.0-alpha.2 | `fdb511da1650600dba5b97e04880b1b9aa2738a6`，仓库路径 `crates/iroh-webrtc-transport` | `acdc23ee2c0c67cdff4d5a1544eb472c96460c0948ca436dcb41b9cadc973aac` |
| webrtc 0.21.0 | `4be2e3d6f99787ac438d28f9f704d343b1aa1498` | `b3cc7db94f14a20c9401202f7300f38071dcac4446177b40e12dc2df34b2256b` |
| str0m 0.24.0 | `95e5038c6f45de6646d8e3f4c2bca6207a5bedde` | `018e4267976f45bf568c146b559bbabd2e823a9dc8c10b5b86e83118b597b89c` |

上述已有包位于 `/tmp/miopunch-webrtc-source-20261003/`；本次读取包内容并计算校验值，没有再次下载。提交来自各包的 `.cargo_vcs_info.json` L1–6。仓库本次读取的 HEAD 是 `c4974a0e4f2be3a7ff4b2474cb503279ad5b0253`，同时存在用户 WIP；本地引用描述实际读取的工作树，不冒充干净提交快照。只新增本报告。

现有设计依据是 [CONTEXT](../../CONTEXT.md)、[ADR 0005](../adr/0005-side-channel-control-bootstrap.md) 和 [同一连接路径维护记录](2026-10-02-connection-path-maintenance.md)：旁信道递送受限 iroh 报文；端点身份、成员准入与业务承载资格分开；目标是保留原端到端会话，原连接终止后明确失败。已完成的浏览器公共 relay 资格是另一条路径的证据，不能替代 RTC 资格。[浏览器公共 relay 记录](2026-10-03-browser-official-public-relay.md)。

本次沿用已有资源入口证据，不重复运行：正常证书校验下，EMQX `8084/mqtt` 和 HiveMQ `8884/mqtt` 的浏览器 MQTT subscribe/publish，以及 browser-WSS↔native-TCP1883 双向报文通过；Ergo testnet `webirc` 只有 WSS 101、`text.ircv3.net` 与 IRC 001 注册证据，没有 PRIVMSG、mio QUIC 或 RTC 资格。native MQTT1883 观察使用明文 TCP，不能误称 native 侧 TLS 已验证。[MQTT 入口证据](../evidence/browser-wss-resources-2026-10-03/mqtt-browser-native.json)、[IRC 入口证据](../evidence/browser-wss-resources-2026-10-03/irc-browser-entry.json)。

## 浏览器和原生端实际复用的是不同东西

浏览器 RTC adapter 应调用浏览器已有 `RTCPeerConnection` / `RTCDataChannel`，经 `wasm-bindgen` / `web-sys` 传递 SDP、ICE 事件和二进制报文。原生端需要真正实现 ICE、DTLS、SCTP 的库来与它互通。两边可以共享信令消息、报文格式、队列和 session 状态，不能因此把浏览器 API 和原生 RTC 实现当作同一套运行后端。alpha.2 本身也采用这一分工：native 模块依赖 Rust `webrtc`，browser 模块调用 `web_sys`，没有把 native 库塞入浏览器来取得 UDP socket。[目标条件与依赖][a-cargo]、[浏览器 API 边界][a-browser-rtc]。

W3C 的普通浏览器模型提供由浏览器实现的 RTC 连接和消息接口，信令通道由应用提供；本次因此不把“原生 Rust 库可编译成 WASM”推导成“普通页面可操作原生 UDP/raw socket”。MQTT-WSS / IRC-WSS 也只是可替换旁信道入口；DataChannel 到达 open 和收到消息，仍不等于现有 QUIC 路径已验证或业务资格已释放。[W3C WebRTC §4.1、§6](https://www.w3.org/TR/2025/REC-webrtc-20250313/)、[当前准入释放代码][m-release]。

候选比较仅针对原生 RTC adapter：

| 候选 | 能复用的职责 | 引入的集成工作 | 本范围的判断 |
| --- | --- | --- | --- |
| `webrtc` 0.21.0 / webrtc-rs | `PeerConnectionBuilder`、SDP/ICE、host/STUN gathering、socket/timer 驱动、DataChannel 消息/关闭/背压 | 包装当前 async API，驱动 channel 事件；设置有限发送预算；把 RTC 生命周期映射到本仓库 session/资格 | Browser↔native 的首个 adapter 较省集成步骤；不需要先维护自己的 ICE I/O loop。**推论，未实测。** |
| `str0m` 0.24.0 | Sans-I/O RTC、ICE candidate pair 检查、SDP、DTLS/SCTP、channel 数据与状态 | 自己管理 NIC 枚举、socket、STUN/TURN gathering、时间/收发循环、报文 demux、调度与背压 | 更适合需要掌握物理 socket 的原生端；更多控制来自应用承担更多 I/O 工作，不能算免费能力。 |
| `iroh-webrtc-transport` alpha.2 | 已有 QUIC↔DataChannel frame/queue/pump 思路和两端后端示例 | 升级旧 iroh/noq API；拆掉不合适的 facade；移植 browser private helpers；接本仓库资格/资源策略 | 最有价值的是抽取底层报文桥和事件处理，整库 fork 的复用范围反而需要缩小。 |
| `datachannel` 0.16.1 / datachannel-rs | libdatachannel 的 native PeerConnection/DataChannel wrapper | C/C++ FFI、CMake/clang/OpenSSL 构建和跨平台链接、回调生命周期；同样需本仓库 adapter | 可以保留为备选；引入不同构建体系，不因只做 DataChannel 就视为零成本。 |

事实依据：webrtc 0.21 在 `rtc` Sans-I/O core 上提供 async driver；`with_udp_addrs` 可枚举当前接口，driver 实际 bind socket 并驱动 gatherer。[w-peer]、[w-driver]、[w-gather]。str0m 明确将候选地址获取、socket I/O 和 timeout 驱动留给调用方；它在收到候选后才承担 ICE 检查。[s-io]、[s-loop]。datachannel 的官方发布说明确认 C++ wrapper 和 CMake/vendored 构建选项，底层 libdatachannel 为 MPL-2.0；Rust 项目的许可与底层库许可需分别保留。[datachannel 0.16.1 官方说明](https://docs.rs/crate/datachannel/latest)、[libdatachannel v0.23.0 LICENSE](https://github.com/paullouisageneau/libdatachannel/blob/v0.23.0/LICENSE)。

`webrtc` 的 Runtime 抽象并不自动复用现有 iroh 的 UDP socket：该固定版本 `driver.rs` L449–465 先调用 `std::net::UdpSocket::bind`，再交给 runtime 包装。只需独立 RTC UDP socket 时这不是障碍；若要求与现有 QUIC/打洞绑定共用物理 socket，则必须另评估接入 seam，不能只更换 TokioRuntime。[w-driver]。str0m 的调用方 I/O 模型更容易表达这种共享，但仍需实现 STUN/DTLS/SCTP 与原 QUIC 的安全 demux 和生命周期；本次没有证明现有 mio 打洞 socket 可以直接复用。[s-loop]。

“Rust 库”也不保证没有 native 构建依赖。固定 webrtc 默认 features 是 `runtime-tokio` 和 `crypto-ring`，可选择 AWS-LC；str0m 默认包含 AWS-LC 和 examples，另有 RustCrypto/OpenSSL/平台 crypto provider。应针对目标选择必要 feature 与 crypto backend，并检查实际依赖树；本次未构建、未比较产物，不能声称其中任何候选默认完全避免 C/汇编工具链。[w-cargo]、[s-cargo]、[str0m crypto 文档 L337–354][s-crypto]。

## alpha.2 为什么不能直接加入当前工程

发布包声明 `iroh ^0.98.2`、`iroh-base ^0.98.0`、`noq-udp ^0.10`、`n0-watcher ^0.6`，native 后端 `webrtc ^0.17.1`，浏览器固定 `wasm-bindgen =0.2.118`。本仓库使用 iroh/iroh-base `=1.2.0`、noq-udp `=1.3.0`、n0-watcher `1.0.0`，并 patch 到带有资格和资源策略的 vendor；browser-probe 固定 wasm-bindgen `=0.2.129`。这不是一个依赖 resolver 能消除的接口版本差异。[a-cargo]、[本仓库 Cargo L54–58、L111–115][m-cargo]、[browser Cargo L12–28][m-browser-cargo]。

即使 Cargo 允许某些旧版本同时存在，旧 `Builder`、`CustomAddr`、`RecvMeta`、watcher、trait 实现不会变成新版本的同名类型；在实际版本/links 约束下 resolver 也可能先报冲突。本次没有运行 resolver，所以不指定它会首先报哪一个错误。将 manifest 改成 1.2/1.3 还不够，下面是源码里已能定位的真实接口差异。

| 接口 | alpha.2 | 当前工作树 | 移植要求 |
| --- | --- | --- | --- |
| `CustomEndpoint::poll_recv` | 最后输出参数为 `&mut [Addr]` | `&mut [RecvInfo]` | 每项生成 `RecvInfo::new(remote, local)`；适配新 noq `RecvMeta`，保留 datagram stride。 |
| `CustomSender::poll_send` | `cx, dst, transmit` | `cx, dst, src: Option<&CustomAddr>, transmit` | 接收本地 route 选择；校验或正确映射 source，避免把不同 session 的双向路径混在一起。 |
| watcher / 地址 / transport factory | 基于旧依赖类型 | 新版本类型，并有 resource policy wrapper | 重新绑定全部跨 crate 类型；不要桥接一套旧 iroh Endpoint 进现有 Node。 |
| owned-resource cleanup | `WebRtcEndpoint` 没实现本仓库 shutdown seam | optional `CustomEndpointShutdown`，Endpoint 限定关闭时间 | 注册 RTC 关闭/任务取消、解绑事件、唤醒等待者，遵循 QUIC drain 后的清理顺序。 |
| 外部资源 | 预置 Google STUN；配置只接受 STUN/STUNS，拒绝 TURN | 本仓库按使用类型、来源、custom 地址检查资源 | STUN gathering、WSS 信令和可选 TURN 的权限必须在访问前生效；不能只靠 sender 的 custom 地址门控。 |

对应源码是 [alpha endpoint L53–77、L138–185][a-endpoint]、[alpha sender L75–80][a-sender]、[当前 custom traits L125–204][m-custom]、[RecvInfo L656–710][m-recv]、[alpha STUN config L18–67][a-config]。本仓库 `CustomTransport::resource` 描述单个设施，用当前 wrapper 检查 bind/send；一个 RTC session 可能同时接触多个 STUN origin，而 native `PeerConnection` 会在报文桥之外自行访问它们，故不能假定声明一个 resource 即覆盖所有 RTC 外部访问。[当前 custom traits L24–45、L94–117][m-custom]、[原生 RTC config L102–145][a-native]。这是接线缺口的推论，不要求本次决定新增配置模型。

浏览器端还有公开 API 边界：`transport`、`native` 对外公开，`core` 与 `browser_runtime` 是 private；`browser/rtc.rs` 的 helpers 为 `pub(super)`。公开浏览器入口主要是库自己的 `BrowserWebRtcNode` facade，不能从现有 `BrowserNetwork` 通过一个公开 attach 调用装上 channel。实际选择是扩展上游低层 API，或者抽取相应 MIT 源码并做本地 adapter，保留归属信息。[a-lib]、[a-browser]、[a-browser-rtc]。

## 哪些底层代码确实有复用价值

### 报文桥、GSO 和 MTU

发送端将 noq `Transmit` 连同 `segment_size` 装成一个 binary frame，接收端把 frame 还原为 datagrams，并以 `RecvMeta.stride` 表达批量段；接收缓冲不够容纳整批时，保留 pending frame offset，逐段交付。这个逻辑已经覆盖“一个 DataChannel message 不一定只有一个 QUIC datagram”的接线细节，可以直接借鉴并移植新 trait 类型。[a-sender]、[a-endpoint]。

固定发布 frame v4 是 30 字节头（magic/version/flags/session ID/segment size/payload length）加 QUIC payload。**源码默认最大 payload 是 65,535 字节，默认 max_transmit_segments 是 10**；README 的“默认 1200”说明与该发布代码不一致，报告以包内 `frame.rs` L12 和 `transport.rs` L164–170 为准。65,535 是 frame payload 上限，不是验证过的网络 MTU、浏览器可用 message size 或推荐发送大小。[a-frame]、[a-transport]、[README 的不同默认描述](https://docs.rs/crate/iroh-webrtc-transport/0.1.0-alpha.2)。

因此适配时需要把 frame/header 长度、QUIC 单段 MTU、GSO 批大小和协商的 SCTP message 上限分开处理。不能让 MTU 探测/GSO 批量把 frame 推过接收端限制；保守地先禁用或降低 GSO 是可评估的实现取舍，不是本次作出的配置决议。较大 GSO 批可以减少消息/回调次数，但同一 frame 的 SCTP 分片若无法完整交付，也会使其中多个 QUIC datagram 一起丢失；这项取舍未实测。webrtc 0.21 channel 文档仍提示普通 OnMessage 大小限制，应结合实际双方后端行为核查，不能把某个库的 64KiB frame 默认值当作跨浏览器保证。[a-sender]、[w-data]、[W3C 固定版本 §6.1.1 maxMessageSize](https://www.w3.org/TR/2025/REC-webrtc-20250313/#rtcsctptransport-interface)。

### 队列、waker、背压与关闭

`SessionHub` 有一个 bounded 接收队列，以及每个 session 的 bounded mpsc 发送队列。接收满时丢弃并报错；`poll_send` 在 outbound capacity 不足时返回 Pending，PollSender 与显式保存的 waker 支持恢复/注销唤醒。hub 关闭会唤醒 recv waiter 并移除 outbound routes。可复用的价值在于将 async/browser 事件世界转成 iroh 要求的 poll 接口。[a-hub]。

默认 packet queue 容量是 1024；DataChannel low/high 阈值为 **4MiB / 16MiB**。这些是不同层的预算：前者主要按消息数量计，后者按 channel bytes 计，多 session 总内存还会累计。不能复制默认值后声称内存/排队延迟已有合适上界。共享 bridge 应同时限制消息大小、session 数和累计 bytes，并明确定义队列满/关闭的丢弃与错误语义。[a-config]、[a-hub]、[a-transport]。

native 0.17 后端已有一个 outbound pump、open 去重、binary/wrong-session 检查，以及 high-water 等待 `on_buffered_amount_low` 或 close 通知；浏览器对应代码将 `ArrayBuffer` 转为 bytes，使用 `bufferedamountlow` / close / error 事件，并在安装监听器后再检查状态以避免错过边沿。这些比“调用 send()”多出的异步细节值得复用。[a-native-pump]、[a-browser-events]、[a-browser-pump]。

照搬还会留下 lifecycle 缺口：alpha `is_valid_send_addr` 只检查地址是否为语法正确的 session 地址，不查询 route 是否存在或 channel 已关闭。本仓库退休失效 custom 路径时依赖该方法返回 false；只 unregister hub 的 outbound route，会让该 predicate 继续认为地址可用。移植必须使 route validity 反映当前 session 状态，关闭时撤销 advertisement/route、唤醒 poll 与背压等待者；单个 channel 关闭不能顺手 `hub.close()` 杀死其他 peer 的 RTC paths。[a-sender]、[a-hub]、[当前失效路径退休 L1113–1147][m-retire]。

浏览器 `RtcPeerConnection` / `RtcDataChannel`、`Closure` 与 `Rc<RefCell<...>>` 由本地主线程 owner 保有，poll-facing bridge 只持有可满足 `Send + Sync` 的 queues/state；库自身的这条分界可以保留。不能给 JS 对象补 unsafe Send/Sync 来满足 custom trait。需要在 close/cancel/page 生命周期内解绑并释放回调，保留 promise/event 的 local 调度。这是沿用现有源码结构的推论。[a-registry]、[a-browser-pump]、[当前 custom trait bounds][m-custom]。

另一个需要移植时收紧的失败边界：alpha browser backpressure helper 无法安装监听器时返回 `Low`，open helper 类似地继续；在现有“明确失败且有界”的语义下，应把这类无法继续可靠等待的状态纳入取消/错误处理，不能把 helper 当作已证明的无丢唤醒实现。本次只是定位该分支，未运行故障实验。[a-browser-pump] L510–577、L615–688。

### Native 后端不能只把 webrtc 0.17 改成 0.21

alpha.2 使用 `APIBuilder`、`RTCDataChannel` 具体类型和分散的 `on_open/on_message/on_close/on_buffered_amount_low` callback；webrtc 0.21 公开的是 `PeerConnectionBuilder`、`PeerConnectionEventHandler`、`Arc<dyn DataChannel>`、`dc.poll()` 的 `DataChannelEvent`，binary send 参数是 `BytesMut`。因此可以复用 adapter 的职责和状态逻辑，需要重写具体 API 接线。[a-native]、[w-peer]、[w-data]。

0.21 已有 `with_data_channel_send_buffer_limit`、`outstanding_bytes`、`writable`、`try_send`：有限预算下发送等待 capacity，关闭会唤醒 blocked sender；默认发送预算仍是无界。新 native adapter 可以直接采用这些公共能力而少维护旧的临时 callback waiter，但仍须设置实际预算，理解多 sender 的软上界，并把 session 关闭传播到 bridge。它不是原有代码自动升级，也不是“库有背压，应用不需要预算”。[w-peer] L308–333、[w-data] L121–228。

## 同一 Connection 能接到哪一层

alpha.2 native facade 明确创建两个 Endpoint：一个正常 bootstrap Endpoint，另一个使用相同 secret key 的 Minimal/custom-only application Endpoint。bootstrap 交换 RTC offer/answer/candidates 后，再在 application Endpoint 对 session custom 地址调用 `connect`；WebRtcPreferred 的 fallback 也属于 dial 阶段重新选择。相同 EndpointId 不代表同一个 QUIC Connection，更不是活跃业务 stream 在路径间迁移。[a-facade] L254–320、L529–542、L725–737。

**低层 transport 自身不必另建 Endpoint。** `configure_endpoint` 只是 `builder.add_custom_transport(...)`；因此把移植后的 bridge 预先注册到现有 native/browser builder，再由现有 coordinator 对目标 Connection 开路径，在接口层是可表达的。必须区分两个时机：[a-transport] L143–162、[当前路径 API][m-path-api]。

1. **首次旁信道升级，业务尚未释放。** 在 `before_data_release` 的受限、已认证控制阶段交换 SDP/ICE；RTC DataChannel 真正 ready 后，原 QUIC dialing side 用 `open_control_path(original_connection, rtc_remote_addr)`，验证原 QUIC path。不能用需要 Ordinary/已准入业务状态的 `open_data_transport_path` 替代。当前 native 的这个 hook 已承接 MQTT/IRC/TCP/relay/UDP 升级，是新增 RTC 分支的现有接入层。[当前 bootstrap L184–255][m-bootstrap]、[open_control_path L1215–1224][m-control-api]。
2. **已经 Ordinary 的成员业务连接增加/恢复路径。** 可复用 `open_data_transport_path` 和现有路径 ACK/资格逻辑，但要将 RTC data ID 加入明确允许的候选集合；当前 ResourceContact 只认可 TCP 和 member data custom ID，未知 custom carrier 不会自动成为业务恢复候选。[当前资源候选 L625–653][m-resources]、[现有维护记录](2026-10-02-connection-path-maintenance.md)。

路径偏好也需要明确接线：当前 `PreferDirect` 只识别 IP 与 TCP data ID，`PreferRelay` 识别 native relay 与 member data ID；未知 custom RTC route 不会自动拿到直连优先级。默认 Quality 可以比较合格路径的 QUIC RTT，但 direct RTC 与潜在 TURN 路径属于不同物理承载，不能仅以一个 custom ID 无条件全部分类为直连。[当前路径排序 L46–58][m-selection]。这属于后续资源/策略接线，未在本报告冻结策略。

首次释放还存在一个必须修改的窄 fork seam：`ControlSession::commit_data_path` 委托 fork commit，后者只找 established IP path、安装 IP-only packet gate，并关闭**全部** custom paths；这会关闭新 RTC path，单纯添加 CustomTransport 无法完成首次升级。需要泛化为“仅保留已验证、明确获准的数据路径，退休信令路径”的释放操作，或者将同等语义安全地接入已有 finalize/authorize 流程；不能简单删除所有 custom 限制。[ControlSession L268–320][m-control-session]、[fork commit L762–816][m-commit]。

已有 `AfterHandshakeOutcome::AcceptData { allowed_data_custom_ids }` 与 `authorize_candidates` 可以按 transport ID 保留获准 custom route，并阻止不获准/已退休 route 被重新选择。native gate 有注册的 allowlist，browser gate 当前返回空 allowlist。RTC 必须穿过双方的 allowlist、before-release 成功条件、最终 packet gate 和资源候选过滤；仅 native 添加一个 ID 仍会被 browser 拒绝。受限临时连接、普通成员连接和其它 peer 的 route 不应借用彼此资格。[native gate L644–656][m-native-gate]、[browser gate L294–300][m-browser-gate]、[fork authorize L819–865][m-authorize]。

coordinator 的职责是把 offer/answer/trickle/end、连接身份、协商 generation、session route 和取消绑定到当前受保护控制流；处理候选先于 remote SDP 到达、双方同时发起、超时、成员资格变化、ICE 失败/重启与 route 退休。alpha.2 `DialCoordinator` 已有 from/to、dial ID、generation 匹配逻辑可参考，但它的 dial/facade 状态不是本仓库 original Connection 的资格证明，也不能代替现有 SCRAM/成员授权。[alpha coordinator L12–156][a-coordinator]、[ADR 0005](../adr/0005-side-channel-control-bootstrap.md)。

需要验证的最终成功条件是原两端 Connection stable ID 保持、原 stream 继续、原 QUIC 路径有 fresh authenticated ACK，且 MQTT/IRC 不再获准业务报文。DataChannel open 只能说明 RTC carrier ready。原 Connection 终止后，只能报告失败或让调用方另建新连接；ICE restart/新 channel 并不能复活已终止 QUIC session。这沿用现有路径维护语义，不是声称 RTC 已实现这些结果。[现有维护和故障资格](2026-10-02-connection-path-maintenance.md)、[当前 open_data_transport_path 文档][m-path-api]。

## 引入工作分层与收益边界

| 层次 | 最小新增/适配 | 可以继续复用 | 是否需要改 transport 内核 |
| --- | --- | --- | --- |
| 旁信道 browser adapter | MQTT-WSS / IRC-WSS 的报文递送、分段/预算/取消 | 已有受限 QUIC 控制流、身份和准入语义 | WSS 自身不要求新 QUIC 内核；本报告未资格化这段集成。 |
| RTC packet bridge | frame、地址/session、bounded queues、poll/waker、活性与 shutdown | iroh 的 CustomTransport trait、noq packet/ACK/stream 处理 | 不需新增 QUIC wire format 或密码协议；要 port 新 trait API。 |
| Browser RTC adapter | web-sys API、SDP/ICE、local event owner、binary send/recv、缓冲/关闭 | 浏览器原生 RTC 网络栈、现有 BrowserNetwork 身份/存储 | 不需要在 WASM 引入原生 RTC 内核。 |
| Native RTC adapter | webrtc 0.21 event/channel facade，或 str0m 外部 I/O driver | 候选库已有 RTC 协议实现 | 不需要自行实现 ICE/DTLS/SCTP 协议；str0m 仍需外部 gathering/I/O。 |
| 协商/资格 coordinator | 接 restricted hook、same-Connection path open、双方 ID allowlist、资源权限、失败/退休 | 本仓库会员授权、候选/路径维护、QUIC validation | **需要窄的资格释放 seam 适配**，首次 commit 当前 IP-only。 |
| 更深 socket 共用/CC 改造 | 只有实际要求时另评估 | 不能把 Sans-I/O 本身视为已经接好 | 不属于完成 Browser↔native 首个 RTC carrier 的必然前提。 |

这意味着合理的“复用收益”是省掉 RTC 协议栈实现、利用现有 custom-packet 接口、复用准入和业务 session；**不是只增加一个 Cargo 依赖**。整库 fork 会额外背负双 Endpoint facade、自己的 browser Node/stream/protocol registry、bootstrap/fallback 策略，以及旧依赖 API，许多职责已有现成 mio 实现，仍需拆除或改写。只抽取小 bridge 和两端事件处理，接口和升级面更符合当前需求；这是按已读代码作出的建议。[a-lib]、[a-facade]、[a-transport]、[a-browser]。

与重新在 DataChannel 上定义业务协议相比，QUIC-over-DataChannel 保留现有 stream、身份、应用路由和 same-Connection 路径模型，减少重做准入/同步的范围。但它同时保留 QUIC 加密并增加 DTLS 加密、SCTP/frame 处理、内存复制和新的 handshake/lifecycle；不因为 transport adapter 小就宣称开销小。[a-sender]、[a-frame]、[RFC 8831 §5–6](https://www.rfc-editor.org/rfc/rfc8831.html#section-5)。

alpha native 创建 `ordered=false, max_retransmits=0` 的 channel，可避免把 QUIC 重传叠到完整可靠有序的 DataChannel 上；然而 SCTP congestion control 仍存在，不能被描述成裸 UDP。双层 QUIC/SCTP 控制、较大 SCTP 分片和 browser/native send queue 会影响 RTT/吞吐观测，可能出现队列滞后；影响幅度必须测量，源码不足以证明特定性能劣化或特定 CC 需要替换。[a-native] L65–77、[RFC 8831 §6.1、§5](https://www.rfc-editor.org/rfc/rfc8831.html#section-6.1)。

## 尚未得到的结论

本次可以确认存在可移植的 packet bridge 和原生候选 API，也能定位 same-Connection 首次释放的真实修改点。以下仍是后续资格问题，不能从库存在或 WSS 可达推定通过：

- 普通浏览器 RTC↔固定 native 后端的真实互通、candidate/mDNS/IPv6 行为；受限会话上的首次升级与原 stream 保留。
- 两端 RTC path admission、信令 path 禁业务、成员移除与资源禁用下的 close/queued-packet race；session A close 不污染 session B。
- GSO、frame 上限与双方最大 message 的组合；队列满、listener 失败、低水位/关闭同时发生、ICE restart 后旧 route 不复活。
- 公网 NAT 成功率、STUN 可达性、实际 fallback 与取消；是否需要 TURN 是资源和产品决议，alpha.2 不支持它不是产品结论。
- native 安装包和 WASM/JS 下载增量、crypto build 兼容性、峰值内存、CPU/能耗、协商耗时、首包时间、吞吐和断路重建。这些没有本仓库 RTC build/运行数据，不能给出工时、百分比或无依据的 MB 数。

如果只选择下一步评估入口，建议从 **webrtc 0.21 native + browser 原生 RTC + 小共享 bridge** 开始；将 alpha.2 作为已定位细节的源码素材，优先看 packet/pump，不使用其双 Endpoint facade。如果后续明确要求 RTC 跟当前 NAT 调度共用原生 socket，再把 **str0m + 自管 gathering/I/O** 纳入针对该要求的对照。真正的当前阻塞是两端资格释放和 adapter 生命周期尚未接线/资格化，不是缺少一个可用 RTC crate。

## 固定源码引用

下面的上游引用均固定到发布包记录的提交；L 范围在正文说明。本地链接定位本次读取的工作树行号。

[a-cargo]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/Cargo.toml#L16-L74
[a-lib]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/lib.rs#L45-L69
[a-facade]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/facade/native.rs#L254-L320
[a-transport]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/transport.rs#L143-L171
[a-endpoint]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/core/endpoint.rs#L53-L207
[a-sender]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/core/sender.rs#L36-L176
[a-frame]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/core/frame.rs#L3-L75
[a-hub]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/core/hub.rs#L754-L1009
[a-config]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/config.rs#L9-L67
[a-native]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/native.rs#L57-L153
[a-native-pump]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/native.rs#L250-L491
[a-browser]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/browser.rs#L18-L123
[a-browser-rtc]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/browser/rtc.rs#L15-L105
[a-browser-pump]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/browser_runtime/direct/data_channel.rs#L315-L488
[a-browser-events]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/browser_runtime/direct/data_channel.rs#L40-L168
[a-registry]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/browser/registry.rs#L1-L21
[a-coordinator]: https://github.com/SuddenlyHazel/iroh-webrtc-transport/blob/fdb511da1650600dba5b97e04880b1b9aa2738a6/crates/iroh-webrtc-transport/src/core/coordinator.rs#L12-L156
[w-cargo]: https://github.com/webrtc-rs/webrtc/blob/4be2e3d6f99787ac438d28f9f704d343b1aa1498/Cargo.toml#L14-L33
[w-peer]: https://github.com/webrtc-rs/webrtc/blob/4be2e3d6f99787ac438d28f9f704d343b1aa1498/src/peer_connection/mod.rs#L259-L342
[w-driver]: https://github.com/webrtc-rs/webrtc/blob/4be2e3d6f99787ac438d28f9f704d343b1aa1498/src/peer_connection/driver.rs#L434-L505
[w-gather]: https://github.com/webrtc-rs/webrtc/blob/4be2e3d6f99787ac438d28f9f704d343b1aa1498/src/peer_connection/transport/stun_gatherer.rs#L1-L69
[w-data]: https://github.com/webrtc-rs/webrtc/blob/4be2e3d6f99787ac438d28f9f704d343b1aa1498/src/data_channel/mod.rs#L76-L279
[s-cargo]: https://github.com/algesten/str0m/blob/95e5038c6f45de6646d8e3f4c2bca6207a5bedde/Cargo.toml#L100-L128
[s-io]: https://github.com/algesten/str0m/blob/95e5038c6f45de6646d8e3f4c2bca6207a5bedde/src/lib.rs#L464-L482
[s-loop]: https://github.com/algesten/str0m/blob/95e5038c6f45de6646d8e3f4c2bca6207a5bedde/src/lib.rs#L128-L228
[s-crypto]: https://github.com/algesten/str0m/blob/95e5038c6f45de6646d8e3f4c2bca6207a5bedde/src/lib.rs#L337-L354
[m-cargo]: /home/js/Git/miopunch/Cargo.toml:54
[m-browser-cargo]: /home/js/Git/miopunch/browser-probe/Cargo.toml:12
[m-custom]: /home/js/Git/miopunch/vendor/iroh/src/socket/transports/custom.rs:24
[m-recv]: /home/js/Git/miopunch/vendor/iroh/src/socket/transports.rs:656
[m-path-api]: /home/js/Git/miopunch/vendor/iroh/src/endpoint.rs:1133
[m-control-api]: /home/js/Git/miopunch/vendor/iroh/src/endpoint.rs:1215
[m-bootstrap]: /home/js/Git/miopunch/src/message_bootstrap.rs:184
[m-control-session]: /home/js/Git/miopunch/src/control_session.rs:268
[m-commit]: /home/js/Git/miopunch/vendor/iroh/src/socket/remote_map/remote_state.rs:762
[m-authorize]: /home/js/Git/miopunch/vendor/iroh/src/socket/remote_map/remote_state.rs:819
[m-retire]: /home/js/Git/miopunch/vendor/iroh/src/socket/remote_map/remote_state.rs:1113
[m-release]: /home/js/Git/miopunch/vendor/iroh/src/endpoint/hooks.rs:208
[m-resources]: /home/js/Git/miopunch/src/resources.rs:625
[m-selection]: /home/js/Git/miopunch/src/path_selection.rs:46
[m-native-gate]: /home/js/Git/miopunch/src/admission_gate.rs:644
[m-browser-gate]: /home/js/Git/miopunch/browser-probe/src/network.rs:294
