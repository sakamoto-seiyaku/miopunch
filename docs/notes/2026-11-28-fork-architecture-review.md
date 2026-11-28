# MioCube 网络 fork 与管理状态代码审查

2026-11-28。源码基线 `164f686d`，位置按本轮清理后的工作树标注。

审查时首选验证管理记录的提交与发布。同一份成员授权在磁盘和运行时有两个可观察的生效点，取消时序可能让节点持续使用旧视图。下文保留审查时的发现，实施决定和实际验收单列。

## 本次实施决定

用户选择**分批交付完整管理历史，并保留已经验证的进度**。普通成员同步、显式 Replica 和手工／预码入网各自保留原有的身份、权限和完成条件。每次联系使用调用者给出的总截止时间。断线后，接收端凭已持久保存的记录或同一入网请求的检查点继续。准入须收齐并验证授权历史后才能交付成员材料。

小历史一次交付。普通成员同步和准入达到 129 条或消息超过 1 MiB 时使用可续传的页，Replica 按最多 128 条、目标 4 MiB 分页。单条较大的签名记录完整发送。每页的硬上限仍为 4,096 条、16 MiB。分页保留原始签名头和正文。未解决因果依赖的暂存记录、服务策略和设备备份仍遵守各自的容量约束。

浏览器前台 Replica 的发送端和接收端各自在调用入口读取配置的联系期限，建连、已有页确认、数据与最终回执共用这一次截止时间。此前另加的 12 秒内部限制会在完整原件进入 IndexedDB 前反复结束传输，即使调用者仍有时间。显式短期限继续有效，每页可独立保留进度。原生 Replica 的单次操作上限仍为 20 秒。

在尚无发布用户的阶段，用户确认本次修复可以调整存储格式。原生 SQLite 在提交已应用顺序和计数的同一事务中保存按身份排序的当前成员和角色。重开时从完整签名记录重建因果图，并与这份成员视图核对。此前的开发库保存完整 Groups 状态，本次未提供该格式的迁移。

### 凭据恢复

后续决定将分页扩展到创始凭据恢复和管理凭据恢复。每次联系先完成原有的会话绑定凭据证明。小历史可一次交付，大历史按有序前缀续传，每页目标为 128 条或 1 MiB。单帧硬上限为 4,096 条和 16 MiB，较大的单条签名原件须连同封装放入一帧。条数控制单批验签工作量，字节数控制单帧内存和中断时需要重传的数据量。恢复结果仍依赖完整历史、有效授权和原有回执。

有节点目录的原生恢复使用 SQLite，浏览器使用 IndexedDB，两者重开后均可从已验证页继续。无存储参数的底层创始凭据 API 将检查点保留在调用者持有的凭据对象中，同一进程可重连接续。[创始凭据恢复](../../src/genesis_recovery.rs) · [原生管理凭据恢复](../../src/management_recovery.rs) · [浏览器管理凭据恢复](../../browser-probe/src/device_recovery.rs)

实测历史包含三条各 6 MiB 的签名原件。原生管理凭据恢复在第一页后断开，重开时仍无管理权限，再联系后完成恢复并读回原件。浏览器之间完成管理凭据恢复后，重开接收节点也能验证权限和三份正文。创始凭据的原生恢复定向测试六项通过。[原生管理恢复验收](../../tests/management_recovery.rs) · [创始恢复验收](../../tests/genesis_recovery.rs) · [浏览器恢复验收](../../browser-probe/harness/run-network.mjs)

### 第一档验收

- SQLite 工作线程提交成员新增或移除时取消等待，同一运行实例仍能看到新授权并继续写入。单条与批量提交均通过。[同一实例取消测试](../../src/management_store.rs)
- 原生迁移被拒后，保留拒绝结果并释放操作占用。节点重开后可再次迁移或启动设备替换。[拒绝后重开验收](../../tests/network_migration.rs)
- 原生 Replica 从已有两条开始，分 16 次正常联系接收 4,097 条新增记录。预码入网分 8 次联系收齐 4,098 条，部分和完整状态分别重开，消费次数为一。浏览器迁移中断时源网仍可通信，重开后切入目标网并读回三份各 6 MiB 的原始正文。[原生 Replica](../../tests/offline_replica.rs) · [预码入网](../../src/manual_admission.rs) · [浏览器验收](../../browser-probe/harness/run-network.mjs)
- 原生作者离线后，浏览器从另一台浏览器续传。显式短期限结束时保留已存原件，重新联系后取得完整范围的回执。接收节点重开后逐份读回三条 6 MiB 正文，普通成员同步也成功保存。[浏览器转发与独立读回断言](../../browser-probe/harness/run-network.mjs)
- 业务通道重接后继承握手实际通告的接收窗口。握手期间应用腾出空间后，发送方可继续传输。隔离网络中的真实 TCP 双向背压和重接场景通过。[窗口回归](../../src/business_channel.rs) · [真实 TCP 场景](../../tests/native_service_streams.rs)

### 旧网退出进度

用户选择让总体迁移阶段表示旧网退出是否已经结束。切换后，若签名历史已确认旧网成员资格结束，原生与浏览器都显示 `SourceExited`，详情保留 `MembershipAlreadyEnded`。只有实际收到退出回执时，详情才显示 `Delivered` 及收件方。待送达或仍需创始凭据确认时，总体阶段保持 `Switched`。[共用阶段定义](../../src/migration_progress.rs)

### 后续审查项的实施结果

| 原排名 | 修改与验收 |
| --- | --- |
| 5、13 | 普通入网与迁移入网共享材料交付和回执的网络步骤，本地提交完成后向当前页面发布。让 IndexedDB 的完成通知晚于网络截止，普通入网仍可在同页使用同一请求和授权重试；迁移期间源网仍能收发，页面自动切换后重开成功。[普通入网](../../browser-probe/src/network.rs) · [迁移入网](../../browser-probe/src/network_migration.rs) |
| 6 | mDNS 按当前快照增删 IPv4 接口并停止退役接收任务。IPv6 按实际入组结果选择发送接口，失败可以重试，内核报告已入组时仍能发送。被动监听也会刷新接口。七项定向测试通过，两个隔离节点分别在 IPv4 和纯链路本地 IPv6 上发现对端、选中正确的局域网路径并双向收发 PONG。[接口协调](../../vendor/swarm-discovery/src/guardian.rs) · [链路本地验收](../../scripts/check_native_lan.sh) |
| 7 | 普通保活只发 PING。短时资格检查允许新 ACK 从其他路径返回，并在检查期间为对端的备路探测提供同路 ACK。最后一个探测句柄释放后恢复正常选路。原生实测先完成单方探测，再暂停仍显示可用的 UDP，双方通过 RTC 在原期限内完成验证；恢复 UDP 后普通 ACK 回到首选路径。双浏览器并发维护也通过，等待超时、取消和复用之后原流仍可收发。[RTC 探测](../../src/rtc_transport.rs) · [原生回程验收](../../tests/rtc_release_validation.rs) · [双浏览器验收](../../browser-probe/harness/run-network.mjs) |
| 8、14 | 增强 UDP 在地址解析和物理绑定准备阶段响应取消。TCP 与显式增强 UDP 共用最终确认步骤，各自保留资源复核及回滚。真实 DNS 等待、发包后取消和两种载体的确认边界共四个用例通过。[增强 UDP 用例](../../tests/support/member_fusion_udp.rs) · [双载体验收](../../tests/support/member_path_commit.rs) |
| 9 | 文件建连按对端排队，最后一个调用完成或取消时回收该对端的队列。向离线 A 拨号期间，B 复用已有会话并保存 256 KiB。旧文件协议和断点续传回归也通过。[跨对端文件验收](../../tests/native_service_streams.rs) |
| 10 | p2panda-net 在分发时取得处理器的独立引用并释放注册表读锁。已开始的会话继续使用原处理器，动态注册的新协议可立即往返，受限端点仍执行撤权。[注册与会话测试](../../tests/p2panda_resolution.rs) |
| 11 | 待发流按可发送路径排队，组包时只比较当前路径和未绑定队列的队首。固定 CPU 交替测新旧版本各七次，1,024 条流、8 MiB、7,024 个包。优先级偏向另一条路径时，组包中位耗时从 115.01 降至 2.34 µs/包；均衡双路从 2.37 升至 2.44，单路从 2.55 降至 2.26。128 流均衡双路从 2.16 升至 2.39。数据取自本机未优化测试构建。38 项协议测试及真实端点分路通过。双 CLI 验收先让原终端在第一条直连上实际收发，再引入第二条直连。原终端和新 TCP 转发在两个方向均走各自的路径，逐帧匹配，26 次终端回显和每向 2,293,760 字节均核对通过。[发送调度](../../vendor/noq-proto/src/connection/streams/mod.rs) · [双路径验收](../../scripts/check_member_stream_paths.py) |
| 12 | 原生节点收到真实签名移除后，公开进度给出 `SourceExited` 和 `MembershipAlreadyEnded`。浏览器使用同一个阶段映射。[状态回归](../../src/native_migration.rs) |

## 审查范围

逐字节对比 11 个 fork 的 `src/**/*.rs` 和规范化 `Cargo.toml`，共 110 个 Rust 差异文件（含测试、测试钩子）与 7 个 manifest 差异。九个 fork 使用仓库内保存的固定原版，两个 mDNS fork 使用本机同版 Cargo 发布源。p2panda-auth 与 p2panda-net 的 Git 基线为 `083b4821`。下表数字表示差异文件数；语义审查沿所列调用链展开。全平台行为、真实性能及全部上游用例尚未重新验证。[上游材料](../evidence/vendor-distribution-2026-10-19/README.md) · [当前依赖入口](../../Cargo.toml#L136) · [领域术语](../../CONTEXT.md)

| fork | Rust | manifest 有差异 | 核对范围 |
| --- | ---: | --- | --- |
| iroh | 44 | 是 | 成员首拨、已准入连接、普通 QNT 与增强 UDP、TCP 暂存绑定及取消。检查预算、回执和资源交还；另核对资源许可、relay、DNS/pkarr、受限端点和 TLS 补丁入口。 |
| noq | 7 | 是 | 绑定身份、探测句柄与收发流接线。旧式持续 ACK 偏好仍有公开契约。 |
| noq-proto | 24 | 是 | 按流调度、路径标识耗尽、备用 ACK、重绑、旧路 STREAM 交回、迟到 ACK 和 FIN。重排发送元数据有专门验收。 |
| p2panda-auth | 2 | 否 | 因果前态策略检查和错误类型。原始 process 与 process_checked 共用核心算法，保留兼容测试。 |
| p2panda-net | 10 | 否 | 普通及受限端点、地址解析、发现退避、协议分发。保留普通无地址错误和显式解析入口。 |
| p2panda-sync | 0 | 是 | 原生与 WASM 的 store 依赖按 target 选择；src 下 Rust 文件与 0.7.1 基线一致。进一步追到应用的 SQLite、准入和 LogSync 消费者。 |
| iroh-relay | 7 | 是 | QAD、按地址族覆盖观察目的、拥塞控制、配置和诊断。BBRv3 与 QAD 行为依据 ADR 0019。 |
| netwatch | 7 | 是 | Linux link/address dump、地址族与接口身份、网络事件和 UDP 发送。确认失败可回到现有枚举入口。 |
| portmapper | 2 | 否 | NAT-PMP 取得租约后的取消、等待创建任务结束，已取得的 TCP、UDP 映射按原协议释放。 |
| iroh-mdns-address-lookup | 1 | 是 | 接口快照、链路本地地址和接收作用域、活动链路加入次序。 |
| swarm-discovery | 6 | 否 | IPv4 socket、接收任务及 IPv6 组播加入/撤回；普通使用方的默认接口选择另有兼容行为。 |

## 本轮发现

### 管理记录提交后，运行中的授权视图可能停在旧版

**Strong · 优先验证 · 取消时序**

原生 LogSync 的总超时包含逐条应用记录。SQLite 的 COMMIT 和内存 auth、applied_count 的更新分开；固定版本 SQLx 的工作线程允许提交成功后接收方已经消失。下次 apply_ready 只取未应用行，后续写入又用旧计数做条件更新。

**建议与取舍**　让持久管理记录 module 负责把已完成的提交发布到当前节点，或在下一次使用前按磁盘版本恢复。网络 adapter 继续拥有协议超时。 删除这段责任会让每个同步入口自行判断事务是否已生效。已有崩溃测试会重开数据库；当前进程的时序仍待验证。

**验收入口**　locality 集中在提交和视图的交接处。leverage 同时覆盖逐条同步与批量应用。阻断在提交已完成、通知未送达时的等待，在同一实例读成员状态并提交下一条记录，再与重开后的结果对照。

[src/offline_replica.rs:163](../../src/offline_replica.rs#L163) · [src/offline_replica.rs:182](../../src/offline_replica.rs#L182) · [src/management_store.rs:828](../../src/management_store.rs#L828) · [src/management_store.rs:879](../../src/management_store.rs#L879) · [src/management_store.rs:906](../../src/management_store.rs#L906) · [src/management_store.rs:973](../../src/management_store.rs#L973) · [tests/offline_replica.rs:296](../../tests/offline_replica.rs#L296)

### 原生可提交整份快照装不下的管理历史

**Strong · 协议容量 · 静态**

普通成员审批保存并应用新记录。该写入路径没有检查累计历史，当前复制发送却把全部已应用记录投影成一份快照，超过 4,096 条或 16 MiB 即失败。手工入网交付也有限额。原生批量应用和浏览器合并另有各自检查。

**建议与取舍**　在管理历史的写入和交换 seam 明确可交付承诺。保持既有小包上限的同时，需在设计时选择分批同步，或在承诺审批成功前明确容量限制及其产品代价。 只统一常量，原生仍能先承诺再交付失败。要区分记录数、内容字节与编码后报文字节。未做大历史实测。

**验收入口**　locality 让可提交与可交付范围能一起核对。leverage 覆盖原生审批、离线副本和浏览器接收。验收恰好在限额及超过限额时，新成员能看到真实结局。

[src/manual_admission.rs:624](../../src/manual_admission.rs#L624) · [src/manual_admission.rs:392](../../src/manual_admission.rs#L392) · [src/management_store.rs:817](../../src/management_store.rs#L817) · [src/management_store.rs:938](../../src/management_store.rs#L938) · [src/offline_replica.rs:96](../../src/offline_replica.rs#L96) · [src/replica_snapshot.rs:178](../../src/replica_snapshot.rs#L178) · [browser-probe/src/replica.rs:368](../../browser-probe/src/replica.rs#L368)

### RTC 短时探测留下持续的 Backup ACK 路由偏好

**Strong · 探测寿命 · 静态**

RTC 验证调用旧的 start_probe。它给当前物理路径设置持续的同路 ACK 标志，丢弃探测句柄不会释放。原生路径已有随句柄释放的返回路径探测。慢备路延迟影响本轮未测；标志的寿命差异可以直接从源码确定。

**建议与取舍**　让内部各 adapter 按目的选择已有的普通保活、新鲜 ACK 或带临时回程偏好的探测。旧的公开探测行为已有消费者契约，迁移内部调用方即可。 只从上层删除持有者不能复原旧布尔标志。现有按句柄释放的协议测试可作对照，需加上真实 RTC 调用方。

**验收入口**　locality 把 ACK 偏好与探测持有者放在一起。leverage 覆盖 RTC 验证和适用的后台检查。用原连接在成功和取消后观察普通 Backup ACK 的回程。

[src/rtc_transport.rs:901](../../src/rtc_transport.rs#L901) · [vendor/iroh/src/socket/remote_map/remote_state.rs:2152](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2152) · [vendor/iroh/src/socket/remote_map/remote_state.rs:2260](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2260) · [vendor/noq/src/path.rs:297](../../vendor/noq/src/path.rs#L297) · [vendor/noq-proto/src/connection/mod.rs:3191](../../vendor/noq-proto/src/connection/mod.rs#L3191) · [vendor/noq-proto/src/tests/multipath.rs:3146](../../vendor/noq-proto/src/tests/multipath.rs#L3146) · [docs/adr/0019-miopunch-application-on-p2panda-and-iroh.md:27](../../docs/adr/0019-miopunch-application-on-p2panda-and-iroh.md#L27)

### 增强 UDP 的取消晚于首次观察地址解析

**Strong · 取消次序 · 静态**

获得轮次绑定的准备工作先解析观察站并选择物理 socket，之后才检查调用方取消。解析阶段受绝对期限约束，但调用方已取消时仍可能等到解析完成或到期。

**建议与取舍**　让同一轮次从选择观察绑定开始就受调用方取消支配，继续使用原绝对期限。真实绑定和后续 ACK 资格保留在现有融合端点内。 只删后面的取消检查会失去执行阶段保证。现有测试从发包之后注入取消，准备阶段的实际延迟待测。

**验收入口**　locality 把前置等待和执行放在同一轮寿命内。leverage 涵盖常规主动计划与显式增强尝试。验收准备前已取消、解析中取消，以及已有 STUN 发包后取消。

[vendor/iroh/src/endpoint/member_fusion/udp.rs:746](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L746) · [vendor/iroh/src/endpoint/member_fusion/physical.rs:21](../../vendor/iroh/src/endpoint/member_fusion/physical.rs#L21) · [vendor/iroh/src/endpoint/member_fusion/udp.rs:769](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L769) · [vendor/iroh/src/endpoint/member_fusion/udp.rs:870](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L870) · [tests/support/member_fusion_udp.rs:703](../../tests/support/member_fusion_udp.rs#L703)

### mDNS 刷新没有对齐想监听与实际加入的接口

**Strong · 资源寿命 · 静态**

IPv4 刷新只添加当前地址。remove_all 清掉本机公告，接口 socket 与接收任务归另一处管理。IPv6 即使加入组播失败也保存整份请求列表；同一列表下次刷新会跳过之前失败的接口。

**建议与取舍**　让现有发现 module 按快照核对接口增减，分别记住成功加入和仍待加入的接口。保留运行链路优先和未显式配置时的 IPv6 默认行为。 只清空公告不能回收 socket。已核对持有关系；重复换网后资源增长量、具体平台发现延迟仍待测。

**验收入口**　locality 把报文出口和成员寿命放在同一 seam。leverage 覆盖 DHCP 换址和 Wi-Fi 切换。检查旧地址退出后资源回到基线，并验证瞬时加入失败后同一接口可恢复接收。

[vendor/iroh-mdns-address-lookup/src/lib.rs:324](../../vendor/iroh-mdns-address-lookup/src/lib.rs#L324) · [vendor/iroh-mdns-address-lookup/src/lib.rs:563](../../vendor/iroh-mdns-address-lookup/src/lib.rs#L563) · [vendor/swarm-discovery/src/guardian.rs:100](../../vendor/swarm-discovery/src/guardian.rs#L100) · [vendor/swarm-discovery/src/sender.rs:240](../../vendor/swarm-discovery/src/sender.rs#L240) · [vendor/swarm-discovery/src/socket.rs:412](../../vendor/swarm-discovery/src/socket.rs#L412) · [tests/lan_member.rs:44](../../tests/lan_member.rs#L44)

### QUIC 按流分路会重复扫描其他路径的发送项

**Worth exploring · 热路径成本 · 待测量**

在备路调度时检查发送流表；组包还会从公共待发队列弹出不属于本路径的项，随后放回。源码能确认额外工作，尚无当前并发负载下的吞吐和延迟差值。

**建议与取舍**　优先测量两条可用路径与多条并发流。若扫描占比显著，再让现有调度 module 持有按路径可发的工作。业务租约结束与仍待排空的发送端要分别计数。 不能直接删 Send.path 或 business_streams。业务占用与仍可发送的数据有不同寿命。

**验收入口**　locality 让分配、释放和待发队列一起维护。leverage 是减少与当前包无关的流遍历。对照单路、双路大量流以及路径退休后 FIN 和迟到 ACK。

[vendor/noq-proto/src/connection/mod.rs:1415](../../vendor/noq-proto/src/connection/mod.rs#L1415) · [vendor/noq-proto/src/connection/streams/state.rs:599](../../vendor/noq-proto/src/connection/streams/state.rs#L599) · [vendor/noq-proto/src/connection/streams/state.rs:620](../../vendor/noq-proto/src/connection/streams/state.rs#L620) · [vendor/noq-proto/src/connection/streams/mod.rs:483](../../vendor/noq-proto/src/connection/streams/mod.rs#L483) · [tests/support/member_fusion_udp.rs:1135](../../tests/support/member_fusion_udp.rs#L1135) · [vendor/noq-proto/src/tests/multipath.rs:2996](../../vendor/noq-proto/src/tests/multipath.rs#L2996)

### p2panda-net 的会话处理持有协议注册表读锁

**Worth exploring · 普通库入口 · 继承行为**

接收连接时在读锁内借用协议处理器，并跨越交接和 accept 的等待。若处理器服务一个长会话，此时动态注册协议需要写锁，端点 actor 的后续消息也会排队。影响以使用该入口且运行中注册协议的调用方为限。

**建议与取舍**　让注册表只负责取得能独立持有的处理器，再交给会话 adapter。保留现行 ALPN 选择、准入和错误分类。 删除整个协议表会把分发职责推给调用方。真正多余的是等待期间的表锁。该行为也出现在固定上游，不能归咎于此次适配。

**验收入口**　locality 缩小注册和会话寿命的耦合。leverage 让注册与已开始的长会话独立。用一个保持打开的合法会话检验新增注册和后续端点消息仍可完成。

[vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs:188](../../vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs#L188) · [vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs:217](../../vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs#L217) · [vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs:263](../../vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L263) · [src/main.rs:345](../../src/main.rs#L345) · [tests/p2panda_resolution.rs:124](../../tests/p2panda_resolution.rs#L124)

## 已执行的代码清理

iroh 路径 actor 的五处无条件 stderr 诊断改用现有 tracing 或并入已有事件。Noq-proto 两处分支改用 trace，去掉打印前临时收集路径与 CID 列表的开销。原返回错误、重试队列和探测条件均沿用。已核对现行测试与脚本的字符串消费者；TCP 和 NAT 验收所读的 MEMBER_DIRECT_RETAINED、LEGACY_RECEIPT 等事件继续按原方式产生。 [vendor/iroh/src/socket/remote_map/remote_state.rs:1039](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1039) · [vendor/iroh/src/socket/remote_map/remote_state.rs:2099](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2099) · [vendor/noq-proto/src/connection/mod.rs:669](../../vendor/noq-proto/src/connection/mod.rs#L669) · [scripts/check_member_tcp_recovery.py:104](../../scripts/check_member_tcp_recovery.py#L104)

Noq 的 `tentative_path_waits_for_peer_cids_and_leaves_a_spare` 通过（145 Fresh，1 rebuilt）。原生 QNT 的 `ordinary_qnt_keeps_its_position_until_the_path_acknowledges` 在宿主使用本机双端点通过（4.47 秒）；最初沙箱运行在绑定 socket 时返回 EPERM，宿主复用同一个测试二进制（原生构建 460 Fresh，9 rebuilt）。浏览器 `rtc-probe` WASM 目标构建通过（291 Fresh，5 rebuilt）。这些结果验证下述诊断清理。各候选的时序、规模或平台复验仍分别列在条目内。

## 已核对并保留

- p2panda-auth 的原始错误类型和扩展策略拒绝有各自的公开消费者。同步 crate 的 Rust 算法没有本地差异；上述管理历史问题位于 MioCube 的读写和交付路径。[tests/auth_compatibility.rs:21](../../tests/auth_compatibility.rs#L21) · [tests/p2panda_resolution.rs:124](../../tests/p2panda_resolution.rs#L124)
- UDP 物理探测名额可以先释放，已验证候选的临时资源要继续保留到双方确认。已有双载体验收区分确认前重置、有效确认后结束发送、原连接结束和授权改变。[vendor/iroh/src/endpoint/member_fusion/udp/execute.rs:317](../../vendor/iroh/src/endpoint/member_fusion/udp/execute.rs#L317) · [tests/support/member_path_commit.rs:60](../../tests/support/member_path_commit.rs#L60)
- 网关取消后的资源释放和 QAD 继承端点拥塞控制都有实际调用方与验收，不列为清理对象。[vendor/portmapper/src/lib.rs:629](../../vendor/portmapper/src/lib.rs#L629) · [vendor/portmapper/src/nat_pmp.rs:76](../../vendor/portmapper/src/nat_pmp.rs#L76) · [tests/udp_portmap.rs:419](../../tests/udp_portmap.rs#L419) · [tests/udp_observation.rs:287](../../tests/udp_observation.rs#L287)

## 前两轮七项记录

- **原生迁移被拒后占用节点**（入口条件已确认）。目标拒绝后工作线程结束，保留的进度阻止新迁移，并阻止未切换节点办理设备替换。让历史结果与当前操作占用分别表达。验收拒绝后重开、再迁移与替换。 [src/native_migration.rs:260](../../src/native_migration.rs#L260) · [src/native_migration.rs:497](../../src/native_migration.rs#L497) · [src/native_node_replacement.rs:161](../../src/native_node_replacement.rs#L161)
- **业务通道重接可能漏报接收窗口**（时序待复现）。握手发送进度后、本地发送循环初始化前，应用可能读空接收缓冲。发送循环用当前值作为已发送值。应继承实际发出的快照，验收反向静默时原方向仍继续。 [src/business_channel.rs:342](../../src/business_channel.rs#L342) · [src/business_channel.rs:394](../../src/business_channel.rs#L394) · [src/business_channel.rs:570](../../src/business_channel.rs#L570) · [tests/native_service_streams.rs:3099](../../tests/native_service_streams.rs#L3099)
- **浏览器入网超时可能留下过期内存视图**（时序待复现）。网络超时可丢弃 Rust 等待任务，已启动的 IndexedDB 写仍可提交。磁盘与 gate 版本不同时，当前节点的重试会被版本检查挡住。验收同页重试与重新打开。 [browser-probe/src/network.rs:3210](../../browser-probe/src/network.rs#L3210) · [browser-probe/src/storage.js:29](../../browser-probe/src/storage.js#L29) · [browser-probe/src/node_state.rs:52](../../browser-probe/src/node_state.rs#L52) · [browser-probe/src/node_state.rs:117](../../browser-probe/src/node_state.rs#L117)
- **原生文件发起跨成员排队**（锁范围已确认）。单个慢拨号持有所有成员共用的文件拨号锁。另一台已有健康会话的成员也须先排队才可复用。验收 A 离线时向在线 B 发起第二个文件。 [src/native_file_transfer.rs:439](../../src/native_file_transfer.rs#L439) · [src/native_member_services.rs:79](../../src/native_member_services.rs#L79) · [src/resources.rs:1392](../../src/resources.rs#L1392)
- **旧网退出进度的两种解释**（阶段语义待定）。MembershipAlreadyEnded 在原生映射为 Switched，在浏览器映射为 SourceExited。先决定阶段承诺，再统一对外推导；保留有无投递回执的区别。 [src/native_migration.rs:652](../../src/native_migration.rs#L652) · [browser-probe/src/network_migration.rs:153](../../browser-probe/src/network_migration.rs#L153)
- **浏览器入网材料交付次序**（架构候选）。普通入网和迁移目标各自编排授权、材料和回执的持久提交。可以收拢次序；交互分步、迁移后台推进和各自回执位置仍有实际差异。 [browser-probe/src/network.rs:3221](../../browser-probe/src/network.rs#L3221) · [browser-probe/src/network_migration.rs:643](../../browser-probe/src/network_migration.rs#L643)
- **TCP 与显式增强 UDP 最终保留**（架构候选）。两条路径分别掌管对端确认期间的临时资源。共享候选是最终确认到本端保留的寿命；UDP 的业务流证据、学习与自动备路有独立条件。 [vendor/iroh/src/endpoint/member_fusion/direct.rs:460](../../vendor/iroh/src/endpoint/member_fusion/direct.rs#L460) · [vendor/iroh/src/endpoint/member_fusion/udp/execute.rs:615](../../vendor/iroh/src/endpoint/member_fusion/udp/execute.rs#L615) · [tests/support/member_path_commit.rs:60](../../tests/support/member_path_commit.rs#L60)
