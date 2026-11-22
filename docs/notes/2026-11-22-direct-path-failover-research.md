# 直连短时失速与中继回落的实现对照

2026-11-22。项目对照基于所列版本的源码和官方文档。实测数据来自 miopunch 的 Pixel 6a 与 r_proxy 现场记录。

miopunch 可以在路径仍有入站流量时发现发送方向停滞。低延迟下，待确认数据的 ACK 停滞下限是 1s。Tailscale 给独占直连的探测证据留出 6.5s，之后可同时向 UDP 和 DERP 发包。NetBird 在所选 ICE 候选超过 6s 未收到流量时启用已有中继。Linux MPTCP 可以先借备用子流重传，之后再把旧子流标为 stale。这些实现的证据和触发动作各异，比较应落到业务中断、换路代价与恢复时间。

## 当前现场能说明什么

Pixel 6a 约第 9s 在原 QUIC 连接首次选中 IPv6，随后约 66s 内记录了 8 段回落。按 1s 间隔采样估计，每段约 1–2s，合计约 13s。r_proxy 在首次直连后一直选用 IPv6。Pixel 6a 和 r_proxy 分别完成 71 和 70 次有效业务往返，两端均为 0 次失败，各自的连接标识始终一致。两端可以为各自的发送方向选不同的路径。[Pixel 首次直连](/tmp/miopunch-ipv6-fix-field-r6eduhqV/pixel6a-status.jsonl#L10)、[Pixel 最终状态](/tmp/miopunch-ipv6-fix-field-r6eduhqV/pixel6a-status.jsonl#L76)、[r_proxy 最终状态](/tmp/miopunch-ipv6-fix-field-r6eduhqV/r_proxy-status.jsonl#L76)

回落附近，IPv6 路径 RTT 约 35–55ms，选中的中继约 231–274ms，相应采样窗口内直连累计 `lostPackets` 增加约 6–10。第一次回落前后的业务往返采样从约 47ms 升为 303ms。现有数据记录了 QUIC 判丢统计、路径状态和业务往返。要区分此次由连续 PTO 还是 ACK 停滞触发，还需要切换时的 `pto_count`、`ack_pending_since` 和实际触发分支。[回落前](/tmp/miopunch-ipv6-fix-field-r6eduhqV/pixel6a-status.jsonl#L17)、[首次回落](/tmp/miopunch-ipv6-fix-field-r6eduhqV/pixel6a-status.jsonl#L18)

当前成员路径的默认重评间隔为 1s。`update_path_health` 会在实际 PTO 次数达到 2 时标记可疑。有数据待确认且 ACK 无进展达到 `max(重评间隔, 2 × 基础 PTO)` 也会触发成员路径的可疑标记。合格备路通过连接归属和必要的同绑定回程确认参与选路。新路径启用后，旧路径未确认的流帧重新进入调度，旧绑定保留以等待恢复或 QUIC 退休。旧直连取得新的健康证据后可重新参加评分。IPv4 TCP 和 IPv6 UDP 使用同一套成员路径健康判断，各自的基础 PTO 取自自己的路径统计。[重评间隔](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L54)、[健康判断](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L3015)、[候选证据](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L3397)、[流帧交接与旧路径保留](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2088)、[每路径 PTO](../../vendor/noq-proto/src/connection/mod.rs#L2965)

## 比较范围

| 实现与核对版本 | 直连失去优先使用权的依据 | 恢复直连的依据 | 时间值的含义 |
| --- | --- | --- | --- |
| iroh 1.2.0 / noq 1.3.0 上游 | 已建立路径参加主备分档与偏置 RTT 排序，所选路径被放弃后重新选路 | 新直连建立后可立即跨档胜过 relay | 5ms 是同档 RTT 切换差值，15s/30s 是路径空闲期限 |
| Tailscale v1.104.7 | 最佳 UDP 的独占信任过期后，下次载荷可同时发往已知 UDP 和 DERP | 匹配的新 direct Pong 可立即选回直连 | 6.5s 是独占信任期限，5s 是单次 disco ping 等待期限 |
| NetBird v0.83.2 | 所选 ICE 远端候选最后接收时间超过断开期限，应用收到 Disconnected 后使用已有 relay | 新 ICE 候选对成功选定后切换 | 配置断开期限为 6s，ICE Failed 总期限为 12s，异步处理影响实际切换时间 |
| ZeroTier 1.16.2 普通非 bonding 路径 | 发送时从仍有效的直接路径选择，找不到合适直连时经 root 转发 | 认证直连包刷新已有路径，新路径经预期 OK 确认 | 19s 的 alive 状态影响质量，243s 是普通直连记录有效窗口 |
| Syncthing v2.1.7 | 活动同步连接失败后重拨，具体关闭受其传输和应用状态控制 | 中继期间继续尝试独立直连，成功建立后按优先级替换 | QUIC 30s idle / 15s keepalive 属于传输配置 |

上述时间值分别从最后一次探测回包、候选接收、路径活动或重评开始计算。业务实际何时改走备路，还取决于备路是否可用和调度时机。

## 上游 iroh 与 noq

iroh 1.2.0 发布源码对应 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`，noq/noq-proto 1.3.0 对应 `c1f411562e6078852749b8bcf1190523096a107f`。版本与修订取自本机 Cargo 发布包的 `.cargo_vcs_info.json`。本文使用的 8 份源码均通过固定修订的 GitHub 原始文件与发布包内容逐字节比较相同。本仓库版本入口见 [Cargo.toml](../../Cargo.toml#L56) 和 [Cargo.lock](../../Cargo.lock#L2838)。

上游默认选择器把 IPv4、IPv6 放在 primary 档，relay 放在 backup 档，IPv6 获得 3ms RTT 优势。同档候选的偏置 RTT 至少改善 5ms 才切换，跨档立即切换。因此已经进入候选集合的直连会胜过更低 RTT 的 relay，同档 RTT 黏性不会延迟 relay 到直连的升级。[偏置与分档](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs#L18-L108)、[完整选择过程](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/biased_rtt_path_selector.rs#L136-L184)

连接加入时登记 `PathId::ZERO`，后续路径收到 noq `Established` 事件后加入，`Abandoned` 事件移除路径并重新选路。选择上下文汇集该远端的存活连接。应用结果把选中路径设为 `Available`，其余设为 `Backup`，客户端还会关闭多余 IP 路径并保留一条 IP 路径，relay 与 custom 路径继续保留。[初始登记](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L418-L445)、[事件与应用选择](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L585-L735)、[候选来源](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L1327-L1345)

noq 的数据调度检查远端 CID、未被放弃和路径验证状态。存在已验证的本地 `Available` 路径时，`Backup` 路径不承载普通应用数据。上游选择器和这段调度均没有当前 miopunch 的 `pto_count >= 2` 或 ACK 无进展过滤条件。上游公开 `PathStats` 也没有当前补充的 PTO、ACK 时间和待确认时间字段。[调度条件](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L1154-L1205)、[上游统计字段](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/stats.rs#L215-L251)、[本地新增字段](../../vendor/noq-proto/src/connection/stats.rs#L218)

noq 内部确有每路径 PTO 计数，PTO 到期安排探测并递增计数，ACK 处理复位计数，用于探测退避。PTO 表示一段时间没有收到预期确认，RFC 9002 明确要求 PTO 到期本身不把先前未确认报文标记为丢失。因此现场的 `lostPackets` 增量、PTO 次数和应用选路触发需要分别记录。[PTO 实现](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L3238-L3278)、[ACK 复位](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L3062-L3071)、[RFC 9002 §6.2](https://www.rfc-editor.org/rfc/rfc9002.html#section-6.2)

iroh 默认每 5s 保活，普通路径最大空闲时间为 15s，relay 路径为 30s。noq 的路径空闲计时至少为 3 × PTO，超时尝试放弃路径，本地不能主动关闭最后一条路径。已经放弃的路径随后还有 draining 和最终删除状态的阶段。15s、30s 和 draining 的 3 × PTO 分别控制路径存活与状态回收，不能据此推定应用等待多久才回落。裸 noq 默认没有路径空闲期限或路径保活，iroh 显式安装了这些值。[iroh 默认值](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket.rs#L105-L129)、[安装配置](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/quic.rs#L151-L162)、[relay 覆盖](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/socket/remote_map/remote_state.rs#L968-L1000)、[noq 默认值](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/config/transport.rs#L590-L595)、[空闲计时](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L876-L895)、[最后路径保护](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L635-L642)、[放弃后的 draining](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L5360-L5369)

## Tailscale

核对版本为 v1.104.7，固定修订 `cbc74fe48ef1b1649f69a4d9a602319427e25102`。活跃 disco 心跳周期为 3s，接受最佳路径 Pong 后，将该 UDP 路径的独占信任更新到 6.5s 后。[常量](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/magicsock.go#L4097-L4103)、[Pong 更新](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L1992-L2018)

信任过期后，下一次载荷发送可同时使用仍已知的 UDP 地址和 DERP。这个阶段允许重复发送，也允许继续探测直连。单次 disco ping 的 5s 等待期限到达时，只有其对应路径仍是最佳路径且信任已经过期等条件成立，才清除该最佳路径。因此 5s 与 6.5s 不能简单相加成固定回落期限。[发送路径决定](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L635-L651)、[UDP 与 DERP 发送](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L1144-L1205)、[ping 超时处理](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L1244-L1258)、[ping 等待常量](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/tsconst/ping.go#L8-L15)

匹配的新 direct Pong 可以立即建立最佳直连，direct 相对 peer relay 的优先级在质量评分前处理。同类路径的黏性另行参与比较。这一实现支持“回落需要旧证据失效，重新采用直连可以由新证据立即触发”，没有显示统一的恢复等待期。[Pong 与候选更新](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L1929-L2018)、[direct 优先级](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L2071-L2084)

## NetBird

核对版本为 v0.83.2，修订 `6d4814d22726c16961be79409dfbf6a2b2f7537b`，其使用的 `netbirdio/ice` 分支固定在 `d0e5dfacb5488d5d55c5b790729f6a85b11a864f`。应用配置为 keepalive 4s、disconnected 6s、其后 failed 6s。该 ICE 实现的循环取默认 2s 与其他启用间隔中的最小值，所选候选对启用 keepalive 后会随循环发探测。[应用配置](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/ice/agent.go#L22-L64)、[ICE 默认间隔](https://github.com/netbirdio/ice/blob/d0e5dfacb5488d5d55c5b790729f6a85b11a864f/agent_config.go#L20)、[循环调度](https://github.com/netbirdio/ice/blob/d0e5dfacb5488d5d55c5b790729f6a85b11a864f/agent.go#L401-L429)、[探测发送](https://github.com/netbirdio/ice/blob/d0e5dfacb5488d5d55c5b790729f6a85b11a864f/agent.go#L581-L596)

ICE 使用所选候选对远端的 `LastReceived()` 计时，超过 6s 进入 Disconnected，超过 12s 进入 Failed。应用在 Disconnected 就处理回落，关闭 ICE、恢复仍存活的 relay 并更新 WireGuard endpoint。起点是最后一次被该 ICE 候选接收逻辑接受的流量，异步回调、运行节奏和 relay 可用性共同影响实际切换时刻。[接收时钟与状态变化](https://github.com/netbirdio/ice/blob/d0e5dfacb5488d5d55c5b790729f6a85b11a864f/agent.go#L553-L596)、[Disconnected 处理](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/worker_ice.go#L651-L675)、[relay 与 endpoint 切换](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/conn.go#L496-L541)

新一轮 ICE 成功选定直连候选对后即提交切换。中继在线时，guard 先安排最多三次 ICE offer，随后每小时重试，链路或地址事件也可重置重试状态。响应端更新 WireGuard endpoint 时留出 5s 供 WireGuard 通过收到的认证流量学习地址。[选定新候选对](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/worker_ice.go#L300-L386)、[guard 调度](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/guard/guard.go#L85-L179)、[重试上限与间隔](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/guard/ice_retry_state.go#L10-L16)、[响应端 endpoint 处理](https://github.com/netbirdio/netbird/blob/v0.83.2/client/internal/peer/endpoint.go#L94-L132)

## ZeroTier

核对版本为 1.16.2，范围为普通非 bonding 路径。正常发送先调用 `Peer::getAppropriatePath(false)` 选择直连，没有合适直连再经 root 转发。普通直连有效窗口为 `4 × 60s + 3s = 243s`，以匹配该路径、经认证且零跳的直接接收时间 `lr` 判断。[发送入口](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Switch.cpp#L988-L1024)、[路径选择](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Peer.cpp#L245-L275)、[窗口常量](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Constants.hpp#L349-L370)

`Path::alive()` 使用原始入站包时间和 19s 窗口，到期后增加路径的选路成本。普通发送仍按 243s 的认证接收窗口判断该路径是否有资格参加选择。已有匹配路径可由被接受的直连包刷新，新路径需要预期的 OK 确认。经中继时，root 可以发出 rendezvous 通知，促成两端尝试直连。[alive 与质量](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Path.hpp#L276-L298)、[接收刷新与直连确认](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Peer.cpp#L68-L187)、[检查预期 OK](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/IncomingPacket.cpp#L509-L553)、[中继发起介绍](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/Switch.cpp#L148-L175)、[接收 rendezvous](https://github.com/zerotier/ZeroTierOne/blob/1.16.2/node/IncomingPacket.cpp#L655-L674)

## Syncthing 与 TCP 对照

Syncthing v2.1.7 的直接 QUIC 配置为 30s 最大空闲时间、15s keepalive。活动同步连接关闭后会调度重拨。官方文档说明中继期间持续尝试独立直连，成功建立后切换，并按连接类型优先级和最小升级差值处理替换。它提供的是应用连接建立及替换的对照，现有业务流的跨路径延续需要另看对应协议能力。30s idle 不能直接解释为丢包后的固定等待期限。[QUIC 配置](https://github.com/syncthing/syncthing/blob/v2.1.7/lib/connections/quic_misc.go#L22-L25)、[连接关闭后的重拨](https://github.com/syncthing/syncthing/blob/v2.1.7/lib/connections/service.go#L394-L403)、[优先级配置](https://github.com/syncthing/syncthing/blob/v2.1.7/lib/config/optionsconfiguration.go#L74-L79)、[官方中继说明](https://docs.syncthing.net/users/relaying.html)、[并行连接与替换说明](https://docs.syncthing.net/advanced/device-numconnections.html)

Linux MPTCP v6.18 提供传输层的补充对照。默认 `stale_loss_cnt=4` 使用重传间隔内没有接收进展且有未确认数据的证据。代码首次采样先建立接收时间基准，在有排队数据和更健康替代子流等条件下，以 `stale_count > 配置值` 判 stale 并安排重注入。重传选择也可在没有可用普通子流且最小 stale 计数大于 1 时利用空闲 backup。接收时间戳推进后恢复 active。这些次数和 TCP 重传时钟共同起作用，无法换算成相同次数的 QUIC PTO。[内核参数说明](https://docs.kernel.org/6.18/networking/mptcp-sysctl.html#stale-loss-cnt-integer)、[stale 判断与重注入](https://github.com/torvalds/linux/blob/v6.18/net/mptcp/pm.c#L880-L930)、[backup 重传选择](https://github.com/torvalds/linux/blob/v6.18/net/mptcp/protocol.c#L2187-L2226)、[恢复 active](https://github.com/torvalds/linux/blob/v6.18/net/mptcp/protocol.c#L1318-L1329)

## 对当前问题的含义

- **弱网仍有进展。** Pixel 6a 的往返记录已经暴露了短时回落的延迟代价。miopunch 的 ACK 停滞下限短于 Tailscale 和 NetBird 的直连证据窗口，同时可以识别仍有入站流量的单向停滞。前者让策略对短时抖动更敏感，后者能在持续单向故障时更早处理未确认数据。
- **持续单向黑洞。** 新鲜备用证据和未确认数据的交接会影响原流能否继续。单纯延长旧路径的保留期限不保证业务恢复，上游路径退休时间也不能直接替代当前应用回落判断。两端发送方向需要分别观察，现场 `r_proxy` 保持直连与 Pixel 回落可以同时成立。
- **回落与重新采用直连。** Tailscale 接受新 Pong、NetBird 选定新 ICE 对、上游 iroh 建立 primary 路径后，都能立即提交直连选路。各自的恢复速度取决于取得新证据的节奏。MPTCP 则提供了另一层对照，备用子流可以先帮忙重传，正式将旧子流标为 stale 采用更严格的条件。
- **确认与退休。** 建立候选、证明返回能力、允许承载业务、降级、放弃和最终删除状态是不同阶段。探测期限、ICE 断开期限、QUIC idle 和 draining 期限应按实际调用链归属，不能拼接成一个统一超时。

现有现场记录支持“发送路径发生短期反复，业务保持连续”。要判断应调整回落触发还是重新选择刚恢复的直连，需要将切换时的 PTO 次数、ACK 停滞时长、回程确认和业务往返时间对齐到同一条原连接、同一个路径绑定。

## KISS 复核

回落后，原连接保留降级的直连绑定，重评时继续在仍存活且获准的绑定上发 QUIC PING。在可疑判定后从旧直连发出的包得到新 ACK，就可以清除可疑标记，重新参与 iroh 选路。ACK 即使经其他路径返回，也归到实际发送的 PathId。业务的两个方向可以各自选路。建议沿用这里已有的恢复证据，撤回额外恢复探测的建议。[旧路 PING](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1812)、[保留旧路](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2088)、[恢复判据](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L3044)、[ACK 归属](../../vendor/noq-proto/src/connection/mod.rs#L3547)、[跨路径 ACK 验证](../../vendor/noq-proto/src/tests/multipath.rs#L2835)

现场八次短回落仍缺少触发分支证据。用现有的 1.5s 和 5s 单向黑洞场景对照短停顿和持续中断，沿用原流往返及现有时间线的观测项。将切换事件与 PTO 次数、ACK 停滞时长和恢复用的新 ACK 对齐。若确认某个分支频繁误判，就只调整该分支，并以持续黑洞时原流恢复的速度验收。历史受控双向黑洞的首个正确原流回复为 1.150–2.989s。[单向黑洞场景](../../tests/support/member_fusion_udp.rs#L1444)、[诊断与时间线](../../tests/support/member_fusion_udp.rs#L1544)、[历史受控结果](../evidence/native-iroh-member-udp-147.md#L15)

Tailscale 用匹配的新 Pong 直接更新选路。6.5s 控制直连的独占发送期限，到期后可以同时使用 UDP 和 DERP。miopunch 已有对应的恢复信号，并在备路可用时交接原流。如果故障紧跟一次成功确认，套用 6.5s 做回落等待会耗尽现有黑洞测试的 6s 业务期限。[Pong 更新](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L1929-L2018)、[独占期限](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/magicsock.go#L4097-L4103)、[过期后双发](https://github.com/tailscale/tailscale/blob/cbc74fe48ef1b1649f69a4d9a602319427e25102/wgengine/magicsock/endpoint.go#L635-L651)、[业务期限](../../tests/path_maintenance.rs#L1284)
