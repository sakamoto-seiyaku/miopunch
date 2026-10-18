# 独立受限端点与准入接口：MIOPUNCH-92

本项落实 [M92](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/b8004144-fd68-4025-acbc-198804561b95) 的新增接口切片，依据 [M91 整体规格](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6d5cd846-3c1b-4c21-977e-fdafc208f876) 及工作区已有的 ADR 0010／0011／0012。新增模式能够实际建连；Native／Browser 全面迁移和旧契约恢复分别属于 M93／M94／M95。

库提供通用的身份、控制和连接交付机制。成员资格、准入秘密、审批及协调协议由应用实现 `EndpointAdmission`；接口没有内置 miopunch 状态机，也没有产品名开关。两种端点共用原来的 socket、TLS、QUIC、路径 actor 和连接实现。

## 公开接口与交付顺序

在原 `Builder` 上配置密钥、传输、ALPN 和旧 hooks，然后调用 `restricted(policy)`，得到只能绑定 `RestrictedEndpoint` 的 `RestrictedBuilder`。新端点没有转成／借用普通 `Endpoint` 的公开入口。新 Incoming、Accepting、Connecting 及控制用途的对应类型都没有 early-data 方法；TLS 的 early data 也在此模式关闭。

| 应用操作 | 可获得的能力 |
| --- | --- |
| `EndpointAdmission::admit` | 已完成 TLS 身份认证的 `ControlConnection` 引用；可进行有界控制消息、查看连接／路径信息和启动原会话的 UDP 路径验证。 |
| `AdmissionOutcome::Pending`／`Reject` | 不交付业务；缺失决策也不默认为接受。 |
| `AdmissionOutcome::ControlOnly` | 只有 `connect_control`／`accept_control` 等控制入口可以接收控制句柄；没有转成普通连接的方法。 |
| `AdmissionOutcome::Accept(DataPathPermit)` | 继续检查原会话的数据路径、当前授权和旧 hook；成功后交付原 `Connection`。 |
| `ControlConnection::weak_handle` | 可观察／关闭原会话的弱句柄，不保活，也不能升级成普通连接；用于接入应用的撤销事件。 |

`DataPathPermit` 只能通过 `qualify_data_path` 创建，绑定原连接、PathId、地址及 Noq fresh probe。候选必须已经建立、符合当前资源／载体资格，并收到探测水位以后的认证 ACK；最终交付会复查路径 generation 和实际存活状态。另一条连接的证明不能用于放行，即使端点身份和地址相同。

数据用途依次执行：TLS 身份认证 → 控制协议及应用准入 → fresh 数据路径证明及当前授权 → 旧 `after_handshake` → 再次复查 → 双边同用途 Ready → 再次复查 → 原 actor 安装数据许可 → 退休控制通道 → 最后复查 → 原 `Connection`。旧 hook 的 Reject 仍关闭连接；旧 hook 中发生的撤销在发送 Ready 前被发现。

控制用途没有普通数据路径交付，也不会向旧业务 hook 传入完整 `Connection`。控制句柄自身不提供通用 stream 或 datagram。应用必须验证其控制协议内容；库不声称能从任意 payload 推断它是控制还是业务。应用在业务交付后仍须将成员撤销接入弱句柄关闭或现有的动态路径 guards，库不会自行获知应用之外的授权状态变化。

## 控制线协议

显式受限模式使用首条 client-initiated bidirectional stream（编号 0），双方先交换 17 字节 `iroh/admission/1\0` 前缀。随后使用 `kind:u8 + length:u32be + body` 帧：kind 0 是最多 65,536 字节的应用控制消息；kind 1 是库的 Ready，body 为单字节用途（0：业务，1：控制）。错误前缀、未知 kind、过长消息和不同交付用途均拒绝。长度在读取 body 前检查。

应用只获得完整消息收发方法，不能取得底层 SendStream／RecvStream。Ready 与应用控制消息分开处理，数据交付前关闭控制 stream。此处增加的是显式新模式的应用控制协议；QUIC／TLS 线格式、密码和拥塞算法没有改变。普通端点不交换这个前缀。

## 路由、期限与所有权

`RestrictedRouter` 复用原 Router 的 accept loop、task owner 和 shutdown。普通 `ProtocolHandler` 的 `accept(Connection)` 与 shutdown 可以复用；其拥有原 `Accepting` 的 `on_accepting` 不在新模式调用。需要自定义 TLS 接入时，通过新 `RestrictedIncoming::accept_with` 手动接入。控制路由使用独立的 `ControlProtocolHandler`。Router 和 Builder 的 endpoint getter 返回受限类型。

默认受限建连预算为 60 秒，可在新 builder 配置。出站从建连开始计时；入站从原 receipt 时刻计时，包含 ALPN 分类。调用者更早的绝对 deadline 始终优先，后续阶段不重新起算。取消、对端关闭或丢弃未完成的 future 会结束准入检查并关闭原连接，包括控制流仍持有 Noq 引用的情况。成功交付后的会话寿命不再由 setup deadline 决定。

关闭与原端点保持一致：`close()` 排空并关闭连接，底层资源还遵守端点对象的所有权。需要等待 MQTT 驱动退出时，应先关闭并释放连接和全部端点 owner，再等待驱动／socket 退出；保留端点引用并等待其析构会形成错误的测试等待顺序。独立控制句柄拥有其控制会话，drop 会关闭它；弱控制句柄没有这种保活作用。

## 仍在使用的过渡接线

本项没有迁移以下既有调用者，也没有把它们的旧枚举分支改成空实现。它们继续经过原 shared registration 的旧分支；新增受限端点走另一条类型化准入分支。

| 调用者 | 实际使用方式 |
| --- | --- |
| `src/admission_gate.rs` | `configure` 安装旧 `Builder::admission_gate`，旧 hook 返回 AcceptData／AcceptRestricted。 |
| `src/connection_coordinator_native.rs`、`src/native_network.rs`、`src/resources.rs` | Native SDK 安装原 AdmissionGate／CoordinatorHook，并在旧 before_data_release 协调建路。 |
| `src/message_bootstrap.rs`、`src/bin/unified_native_member.rs` | 载体帮助入口和探针的旧 before_data_release 接线。 |
| `browser-probe/src/network.rs` | BrowserGate 的旧准入结果、before_data_release 与 builder 接线。 |
| `tests/admission_gate.rs`、`member_attempt.rs`、`mqtt_bootstrap.rs`、`irc_bootstrap.rs`、`negotiated_accept_context.rs`、`unified_connection.rs` | 旧门禁、申请、取消、期限、路由和协调器夹具。 |
| `tests/mio74_rtc_offer_ownership.rs`、`path_maintenance.rs`、`rtc_control_release.rs`、`rtc_invalidation_barrier.rs`、`rtc_path_qualification.rs`、`rtc_release_validation.rs`、`tcp_punch.rs` | 原会话建路、RTC 资格及原生路径的旧 hook 夹具。 |

直接声明／安装点的完整位置保存在[过渡调用清单](../evidence/restricted-endpoint-2026-10-18/transitional-callers.json)。Native／Browser 的入口迁移由 M93／M94 交付；M95 再恢复原入站 `into_0rtt`、旧 hook 枚举及移除过渡接线。M92 不宣称旧消费者源码兼容已经恢复。

## 验收依据

测试穿过公开的端点、路由、控制消息和最终业务连接入口；没有新增生产 mock 接口。新增案例包括真实双端控制／普通 stream、缺决策、拒绝、无合格路径、旧 hook 否决、撤销、取消、deadline、drop、accept_with、控制帧拒绝、跨连接证明，以及实际 MQTT 控制到同一 QUIC 会话 UDP 业务的切换。另以普通端点验证现有 early-data 和真正 0-RTT 恢复继续工作。

原版消费者在首次生产代码修改前固定，并在官方 iroh 1.2.0 源码及其原锁上编译通过。相同消费者保存在[证据目录](../evidence/restricted-endpoint-2026-10-18/README.md)，供 M95 原样重用。类型绕过检查先编译正常消费者，再逐个要求目标编译诊断；不能把依赖缺失或零测试当成成功。最终构建、实际运行、旧 SDK 故事及审查结果均以该目录回执为准。
