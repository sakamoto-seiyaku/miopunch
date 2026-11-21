# 连接流程改进：实施方案与任务边界

初稿 2026-11-17，实施前复核修订于 2026-11-18。状态：设计及验收已修订，待实施。M 指原业务会话，C 指一次改善使用的独立受限控制连接。

2026-11-19 更新。本文中的独立控制连接 C 描述当时的改善流程。融合后的设计默认通过已有共享连接协调换路；首次联系需要引导或原连接失去联系能力时，才使用独立联系入口。见 [ADR 0019](../adr/0019-miopunch-application-on-p2panda-and-iroh.md) 和[共享成员业务连接决议](../adr/0020-shared-member-business-connection.md)。

实施父规格：[MIOPUNCH-124](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/70c91dd0-805d-40e1-9ecd-e53f161dd9a9)。任务状态、认领和依赖以 Plane 为准；本文保存设计依据、实现约束与验收定义，不维护进度台账。本版整体替代旧初稿与“最终审核修订”，不再叠加覆盖规则。

2026-11-20 修订：下文的 P=32F 是原实现的取值。现行节点策略以 `enhanced_udp_cooldown_ms` 控制增强 UDP 冷却（保守 160 秒、积极 32 秒），以 `path_retry_base_ms` 控制路径重试（保守 5 秒、积极 1 秒）。两者独立于探索间隔。待命连接见 [ADR 0020](../adr/0020-shared-member-business-connection.md)。

## 1. 目标与既有保证

沿用 [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)：已有可用且获准的直连优先，否则获准 relay 尽快承载业务；后台在同一会话上改善。新路径仍须原会话验证、新鲜 ACK、双方提交与权限检查。默认 Quality 和显式偏好不变，有直连不表示必须胜过质量更好的 relay。

- 业务优先：后台联系的互斥与失败退避不能拒绝前台建连。同一原会话的 repair 和改善保持一个维护所有者，不能并发修改 M。
- 公平改善：每条缺少获准直连的存活成员 M 都能在节点预算内获得机会，包括按公开 API 持有业务租约期间。预算等待和真实建路失败分别计数。
- 长期运行：观察与探测包都有上限和恢复入口，重试、取消、clone、重绑不能绕过限制，也不能因进程终身额度耗尽永久停用建路。
- 连通性：优化、repair 或请求超时不等于 M 死亡；原 M 关闭后的新连接、新流不能假称恢复旧流。
- 范围：不合并同 peer 会话、不跨会话借用非主打洞 socket、不新增每会话任务、不重做 TCP/RTC 内部算法、不统一调大 QUIC 超时、不新增产品 UI 或依赖。

时间来源为 caller 绝对截止、协议/实际路径参数及现有维护政策。维护间隔为 F，会话退避上限 P=32F；默认保守型 F=5 秒、P=160 秒，积极型 F=1 秒、P=32 秒。有效自定义政策按同一公式计算。两秒本地候选窗口暂留，它是实现参数，不是 RFC 保证。

## 2. 证据与事实更正

基线为当前工作区，包括已完成的 MIOPUNCH-123 编号回收修复和未提交实测代码。实施前核对实际差异，不回退既有工作，不把它算成本批新增成果。

| 证据 | 支持的结论及边界 |
| --- | --- |
| Linux 52 秒内联系被拒 53 次，进行中 32 次、退避 21 次 | peer 联系资源隔离有现实需求；不能将这些拒绝全部归为会话锁 |
| 本次移动网络采样中 A1 出现 39 次 Native 改善阶段启动，其中 22 次在路径 114 首次提交之后；同段未见 B1 进入该阶段 | 首项选择和改善理由判断有问题；不能断言 B1 无法打洞 |
| Windows 多次 direct exchange interrupted，后来 Mode 2 成功 | 五秒控制窗口会截断高 RTT 协商；不是所有网络永久不能打洞 |
| 最后一次 repair 到 terminated 约 3.011 秒 | SDK 主动关闭是强假设，源码也有对应风险；demo 另有流失败关闭入口，实测尚未唯一归因 |
| Linux stderr 116 次 MaxPathIdReached、62 次 RemoteCidsExhausted | 存在瞬时资源受阻；requested=8、max=7 不等于八条活跃路径占满 |
| A1 路径 114 切 UDP，随后新增 188 次成功请求、零新增失败 | 该会话完成真实接管并承载业务；不代表所有会话和长期运行都通过 |
| 两端 QAD 超时后由两个 STUN 来源补上 | 必须覆盖完整回退链；QAD 设施根因尚未确定 |
| 公网观察端口抖动触发 network_changed | 通知混入噪声；相同本地 IP 也不能排除真实路由变化 |

原始材料：`/tmp/miopunch-connection-flow-discussion-20261117.md`、`/tmp/miopunch-handoff-18/proxy-logs.json`、`windows-logs.json`、`mobile-acceptance.json` 及计时样本。它们是历史证据，不是实时宿主状态。采样耗时不作为参数或通过线；旧 repair 分类列举未覆盖所称总数，后续按事件重建，不沿用不完整分类。

代码复核事实：

- `resources.rs` 的 peer 互斥之外，`session_maintenance.rs::acquire` 还有原会话互斥；只拆前者仍会 Busy。
- `network_maintenance.rs::connect` 要求持有业务租约，而 run、step、improve_live_member 都有活动门控；改善后的提前 return 还会跳过联系人和发现。
- `nat_connection.rs::observe` 分开扣 QAD/STUN；每次补一可能被 QAD 抢走，凑不出完整 STUN 链。execute 预扣整轮探测包上界，prepare_upgrade 的 64 份不是全部成本。
- `nat_direct.rs` 拨号侧最终 Result 的错误 nonce/类型目前会变成 Ok(None)；本批必须修正分类，不能写成“原行为已正确”。
- noq open_path 独立检查递增编号与对应远端 CID。iroh Connection::paths() 排除已关闭路径；noq 的全部已知 paths 不能直接作存活证据。
- 现有选路只看 RTT/偏好。刚发 ping 不是已获新鲜 ACK；PTO 会在网络变化时归零，零计数不独立证明恢复。
- noq 本端 close 的 close_reason 是 LocallyClosed，单靠它无法区分 SDK 和 demo 的调用来源。

协议依据：[RFC 9002 §6.2](https://www.rfc-editor.org/rfc/rfc9002.html#section-6.2) 的 PTO 到期本身不确认丢包；[RFC 9000 §8.2.4](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2.4) 的路径验证计时不是业务回落 SLA。十五秒路径 idle、连接 idle、keepalive 是当前配置事实，不能视为任意网络的严格停顿上限。

## 3. 任务边界与依赖

| 任务 | 交付 | 依赖/协作 |
| --- | --- | --- |
| MIOPUNCH-125 | 日志关联时间线和关闭来源 | 优先形成观测基线 |
| MIOPUNCH-126 | 前后台联系隔离、两套 repair 入口的原 M 所有权、受保护的暂缓结果 | 提供 129/130/131 共用的结果与所有权接口 |
| MIOPUNCH-127 | demo 流隔离、请求策略、正确网络通知 | 与 125 协调 demo 文件修改 |
| MIOPUNCH-128 | 控制截止/错误分类；候选资源推进 | 两个独立验收子项，同票同实施者 |
| MIOPUNCH-129 | 业务期间公平改善、维护轮转 | blocked_by 126、130 |
| MIOPUNCH-130 | 完整 NAT 轮次预算、分步 API 许可交接及冷却结算 | blocked_by 126；预算内部可先开发，公开入口验收消费 126 的暂缓结果 |
| MIOPUNCH-131 | repair 失败结算及 Browser 成员数据中继存活判定 | blocked_by 126；两票同一实施者联调，独立于 132 |
| MIOPUNCH-132 | 黑洞回落先测后改、持续选择和恢复 | 只有真实失败才改 fork |
| [MIOPUNCH-134](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/c124af09-72b2-4df0-91a6-d4ec43f98edd) | active IP 黑洞下 relay 及时获得机会 | blocked_by 125 |
| MIOPUNCH-133 | 集成与多网络验收 | blocked_by 全部实施票，包括首连路由票 |

先用 125 记录关闭基线，再用 127 验证 demo 隔离。128-A 的协议边界与 128-B 的资源推进分别验收，两者均通过才完成 128。129 等待 126/130 的接口，133 复用分票的真实入口证据并组合验收。不增加额外流程材料、全量重测或独立审查门槛。任务状态在 Plane 维护。

## 4. 共用验收约束

- 使用现有 SDK 公开入口及真实载体：节点维护测试、路径维护 Pair/代理/relay、netns NAT 实验室、Browser 对应用例。时序变化覆盖低 RTT 和约 1～2 秒高 RTT 代表点；纯日志/分类不机械重跑完整网络矩阵。
- 已确认失败保留原触发与核心断言，再验证修复；条件性优化若当前满足性质，保留证据即可，不为制造红灯改坏测试。预算时钟、选择纯函数可以有小型确定性测试，但不替代 SDK 真实入口。
- 核实环境和前置，区分产品/测试/环境失败；不得靠放宽 caller 截止、弱化新鲜 ACK、关闭准入或合并会话使测试通过。
- 换路检查双方原 stable_id、原流、实际字节和选路；覆盖撤权、取消、原 M 关闭后不提交。
- 范围内反例继续阻塞原任务和 133。可另立缺陷票，但必须保持验收阻塞；新增需求和范围外设施问题另行跟踪，说明实际限制。
- 按项目构建缓存规则运行受影响行为及必要检查；复用有效证据，不因无行为变化重复全 workspace 或全部慢 NAT 场景。

## 5. MIOPUNCH-125：时间线与关闭来源

### 实施

在现有日志补进程内单调毫秒、attempt id、原 M stable_id、存在时的控制 C stable_id、阶段、结果、耗时，不建新事件体系。早期拨号用本地 attempt id，保护协商取得共同 id 后记录关联；两端 stable_id/单调时钟各自独立，不能当全局身份或直接相减。

覆盖每路由拨号、控制建立、hello、候选、观察、打洞、路径打开、新鲜 ACK、ready/commit、回滚。125 记录现有预算，130 补新预算等待、余额和恢复字段。不记录 TLS binding、凭据、证明或秘密材料，不为诊断新增摘要。

`connection_coordinator.rs::dial` 记录每个实际启动的路由和结算，`active_session` 关联共同 attempt 与本端 M/C；区分本端发起、应答及触发来源（前台 repair、维护器改善），同一阶段的两端日志只算一次协议尝试。对 `connection_coordinator_native.rs::repair`、`browser-probe/src/member_connection.rs::recover_path` 及 demo 的接收循环、停止、重连分支，在 close 当下记录调用者和原因。demo 在再次 close 之前记录已有 close_reason；本批 SDK/demo 主动关闭点另记 owner、固定原因类别、stable_id，再关联 connection-ended。单独 LocallyClosed 不能归因；缺证据时明确 unknown，不改线上关闭协议来实现诊断。

### 验收

- 首连和改善的实际阶段可关联成单调时间线；每条启动路由都有成功、失败或取消结果。
- 先在 `ProbeServer::accept` 的单流读写失败和显式 SDK repair 失败场景分别采集关闭时间线；实际调用 close 的一方、原 M 和触发事件可对应。独立对端关闭和材料不足的 unknown 单列。127 修改行为后复用相同触发核对。
- 无秘密日志，连接行为不变；完成受影响入口检查，不要求全仓测试。

## 6. MIOPUNCH-126：联系类别与同会话协调

### 实施

在 `src/resources.rs` 统一 `connect_attempt` 和 `begin_contact_attempt_until` 的占用、失败与 guard 释放逻辑，二者现在各自检查 `in_flight`。ContactClass 定义一次，各调用点显式归类。业务 connect/control、用户准入/访客/设备管理、设备/管理恢复、repair 为前台；维护器同步、改善、后台恢复和发现为后台。按调用责任传递，不仅按函数名分类。

peer 表按 (peer,class) 隔离，同类互斥，跨类不检查对方 in-flight 或退避。任一类完整联系成功清两类失败退避，但不能清对方进行中状态；guard 只释放自身类别。新联系失败只影响本类，改善路径失败归会话，repair 失败不新增 peer 退避。并发成功/失败按实际结算顺序更新。

同一 M 保留唯一维护所有者。repair 遇到本地已有该 M 的改善/应答操作时，复用 registry 当前 epoch 的完成通知，在原 repair 截止内等待，不另开控制协商、不直接以 Busy 宣告 peer 不可达。结果已完成双方提交且当前资格仍有效才可复用为成功；失败后尚有时间则重新争取一次所有权执行 repair，时间不足按 131 判定。等待只针对本地已登记的操作，不承诺接管远端尚未建立的操作。

在 `src/session_maintenance.rs` 的 Slot 下保存本轮完成通知；在同一次加锁中决定取得所有权或订阅已有 epoch，再释放锁等待。完成、错误、取消和 lease drop 均须唤醒等待者。等待者不占住所有者完成所需的资源；取消/超时只结束本次等待。复用成功须重新核对原 M、准确路径、成员及资源许可。等待者无权关闭 M；如需失败结算或开路，在剩余截止内取得所有权后进行。普通前台同类互斥保留。

接线覆盖 `CoordinatorHook::repair`、维护器改善、`serve_active_session` 和 Browser 的 `improve_path/recover_path`；调整 Browser 的 `begin_path_attempt` 与取消令牌归属，等待者不能覆盖活动所有者的令牌。旧版公开入口 `NativeNetwork::recover_connection_path[_until]` 和 `recover_connection_path_confirmed_until` 汇合在 `src/connection_recovery.rs::recover_with_confirmation`，在拨 C 前参加相同原 M 仲裁。应答端 `serve_connection_path_recovery` 在验证请求并解析 binding 后、首次开路前取得同一 registry 的所有权；开路、确认、必要的 RTC 退休均在所有权内。三秒上限从调用入口起算，等待后沿用同一绝对截止。

旧协议保留两字段请求、可选 confirmation 及 `ConnectionPathRecovery` 的真实 path、watermark、control_stable_id、peer_confirmed。未确认或缺少调用方所需字段的成功只能唤醒等待者，在剩余时间内执行其自身协议；不得从另一个操作的 `Ok(())` 合成上述字段或双方提交。新 active 协议只有原 M 新鲜 ACK 与双方 committed 齐备才发布可复用成功。

由 126 定义一项可识别的受限暂缓结果。对已验证身份、ALPN 和原 M 绑定的 active C，原 M 忙时用约定的应用关闭码或等效显式协议结果回复；Native 与 Browser 应答 handler 保留该原因，发起端在 C 的生命周期内解码并传回维护器。130 的远端轮次冷却使用同一暂缓语义。本地预算持有者可返回本地 next_due，远端暂缓按本地维护节奏再试，绝对时钟不跨节点传递。普通 EOF、对端失联、鉴权拒绝及畸形帧不得猜成 Busy；身份/协议错误终止该次协商。关闭 C 的处理不得关闭 M。

### 验收

- 后台改善进行中及后台失败后，前台业务/控制仍立即进入自身拨号；各用户恢复入口归类正确。
- 同类两个并发只放行一个，跨类各一个可并行；一方成功不释放另一方 guard。
- 同 M 改善期间 repair 不产生第二个修改者；可复用完成，等待超时/取消不关 M，只有剩余时间允许才尝试自己的 repair。
- 在 `tests/path_maintenance.rs` 的受保护旧 repair 入口上，同时发起同 M 新改善；覆盖请求端及接收端作为原 QUIC 拨号方，观察最多一个开路/退休者。保留旧线格式、确认字段和 caller 截止断言；未确认成功不得被另一调用解释成双方提交。
- 用真实 active C 使应答方的同 M lease 已被占用；从发起端读到暂缓，M 保持、无第二次开路。用无标记的控制断开与错误 binding 作对照，二者都不能变成暂缓。129 接续验证 failures。
- repair 失败不污染下一次 connect；后台完整联系成功清前台既有退避。
- 同步/发现仍受业务租约门控，改善例外由 129 验收。

## 7. MIOPUNCH-127：demo 错误隔离与网络通知

### 实施

在 `src/bin/miopunch-network-demo.rs::ProbeServer::accept` 内把单流请求解析、读写、finish 的错误结算放到该次迭代，`observe_peer` 的请求失败分支移除自动 repair，保留 stop、已关闭检测及重连。单流 read/write/timeout 只结束该流，连接级 accept_bi 失败才结束接受循环。客户端请求超时只计数、释放本流，不自动 repair；原 M 实际关闭才重连，显式停止照常关闭。126/131 的显式 repair 验收不能随 demo 调用移除而删除。

`sample` 现每秒比较 `network.addr()`；在现有采样循环保存并比较同一 endpoint 的 `udp_network_generation`，真实变化时仅调用 `MaintainedNetwork::network_changed` 唤醒维护器。避免再次调用会强制递增世代的底层 `Endpoint::network_change`。真实接口/路由事件可覆盖本地 IP 相同的切网；观察映射、relay 广告和操作员标记不推进世代。129/130 各自对真实变化只重置一次，重绑本身不补发许可。

### 验收

- 请求超时但连接寿命未结束，停顿后原 M 后续请求恢复。
- 单流取消/错误不影响同 M 的其他与后续流；停止无泄漏。
- 超时不 repair，实际关闭后新 stable_id 重连。
- 观察端口/地址顺序/relay 广告变化不触发重置；真实接口/路由变化仍通知，覆盖支持的同 IP 切网入口。

## 8. MIOPUNCH-128：直连协商与候选资源

### A. 截止与错误分类

在 `src/nat_direct.rs::attempt` 去掉包住 Hello 至 Result 的固定五秒计时。将一次绝对截止从 `native_path_adapter.rs`、`NatConnector::prepare_upgrade` 传至该交换；首连用建连截止，active 用本次 C/lease 的截止，应答取本地 responder 上限与可用 caller 截止的较早者。长期存活 M 的初次建连截止不能用于限制新 C。候选验证用 min(本次截止, 候选阶段开始 + 2 秒)，跨帧、重试均不重新计整次时间。

`NativeNetwork::upgrade_udp → NatConnector::upgrade` 是绕过维护器的公开入口。保留原签名，在入口确定一次有界操作截止，有 caller 约束时取较早者，无显式时钟时在入口将 `connection_budget::remaining(None)` 换算为绝对截止，内部共用带截止实现。逐段传递或保留外层取消保护；到期按原 `CommitError` 契约归还或终结受限会话，保留队列所有权。

拨号方先解析最终 Result 的帧型、nonce，再处理 available；同轮合法否定才返回 `Ok(None)` 进入增强。将畸形帧、错类型及错 nonce 映射到现有 `InvalidControl` 分类，保留权限错误、取消、截止的独立结果。沿 `prepare_control_session`、`active_session` 的候选循环及 `ConnectError` 映射传播；协议错误不得在上层落成普通 `DataPathUnavailable` 或触发其他候选。

### B. 候选与资源推进

`nat_direct::attempt` 发送端在 `addresses.truncate(64)` **之前**过滤无效地址、去重并按既有私网/本地 IPv4、IPv6、公网 IPv4 分组交替取样，保留 64 项线格式上限。接收方先验证帧与条数，再去重分组，在本地两秒窗口和可开路径资源内轮转。各类有机会，容量不足时在同一存活 M 上推进起始类和组内游标，发送端也能在后续广告轮到被截断的有效地址；关闭清理。不按跨子网过滤。

区分已建立/验证中路径、已使用/放弃编号、本地/远端编号额度及下一编号 CID 到达。不能用 8 减 paths().len() 推导全部资源。优先复用 open_path 权威结果，确需查询只加当前消费者需要的窄接口，不建通用容量框架。

为增强胜出路径保留一个实际可开启名额，计入未完成验证的候选后停止灌入。MaxPathIdReached/RemoteCidsExhausted 是暂时资源受阻，不等于地址不可达；沿既有事件/受限重试推进，所有等待纳入本地窗口与整次截止。仅退休本次拥有、未提交的失败候选，保留 relay 和既有/他人路径；不假定 close 立即归还编号/CID。

### 验收

- **128-A 单独验收**。用 `scripts/unified_native_nat_lab.py` 对控制/relay 注入高 RTT，复现旧五秒中断；总预算足够时首次合法协商得到直连或进入增强，保留低 RTT 对照。再由 `upgrade_udp` 真实入口令对端分别停在 Hello、最终 Result，均在入口确定的截止内结束。
- 首连、改善、应答的截止/取消/撤权有效；从公开调用的结果或保留的底层原因确认错 nonce/类型为协议错误，观察次数、增强轮数和随后候选拨号均不增加；合法同轮否定能进入增强。
- **128-B 单独验收**。构造私网地址已超过 64 且另有 IPv6、公网 IPv4 的本地集合，读取实际发出的 Offer，验证三类都有代表、上限 64；在接收端用超过八个候选和有限编号验证跨轮推进、跨子网与增强胜出所需名额。
- 延迟 CID、编号受阻、退休后补充均覆盖；允许受阻日志，不允许忙循环、永久停滞或资源恢复后仍无推进。
- 增强胜出后可在原 M 验证提交，业务与 relay 保留；复用 123 已有回归，只补新增影响。

## 9. MIOPUNCH-129：业务期间公平改善

### 实施

在 `src/network_maintenance.rs::Coordinator::{run,step,improve_live_member}` 替换 `registry.live().into_iter().next()` 和改善成功后的整批提前返回。只改善缺少获准、未关闭直连数据路径且仍有可尝试机制的普通成员 M。IP、已识别 TCP、属于该 M 的已验证 RTC 可满足；未知 custom/信令不能。已有合格直连即跳过，即使 Quality 当前选 relay；直连放弃后重新合格。

现有维护器持有 stable_id → failures/next_due/轮转小状态，随 live 清理；选有理由且到期最早项，同到期稳定轮转。首次可进下一维护机会，第 n 次真实失败后间隔 F×2^min(n,5)。本地 Busy、预算冷却、策略禁用不是 peer 失败；本地同 M Busy 推后一个 F 并继续其他项；消费 126 的远端显式暂缓时也按 F 延后且不增加 failures。130 的 next_due 只约束增强 UDP。已有证据表明本 M 只剩增强可试时，可将该次重试排到预算到期；出现新的地址、可用载体或许可变化时重算。若 TCP、普通 UDP 候选或该平台组合的 RTC 仍有机会，应允许本次 C 尝试这些方式；不得用全节点的 UDP 冷却直接过滤整条 M。重复相同线索不能造成反复拨 C；两端实测持续冲突时在本票补有界抖动，不先增加策略参数。

业务租约暂停同步、发现和普通后台拨号，允许既有 M 的有界改善；run/step/improve 三处门控一致。活跃期间改善和预算到期仍可唤醒；暂停的发现计时器不能忙循环。撤权、禁用、取消、原 M 关闭立即生效。

无租约时，每批至多启动一条改善，随后仍给到期联系人和发现机会；既有邻居同步与改善不能因提前 return 相互饿死。单项按已有截止结束后继续，不无限遍历 Busy、不增每会话任务。公平等待上界包含有限队列前面的操作截止、节点冷却和 F，不承诺任意会话数都在固定几个 F 内执行。

只有真实 udp_network_generation 变化重置会话退避，一代一次。重复 network_changed 和观察抖动不重置；新联系线索可唤醒检查，不重新发全节点额度。

### 验收

- A 已直连、B relay：B 尝试，A 不冗余；首项不合格/Busy 不挡后项；同 peer 多 M 与跨 peer 均覆盖。
- 在 `tests/network_maintenance.rs` 从 `MaintainedNetwork::connect` 取得连接与 BusinessActivity，持续持有**返回的同一个租约**并在同一打开的双向流上传输有序字节。以 B 进入改善、同步/发现暂停、双方原流可读且租约释放后同步/发现恢复作为断言。
- 多条难打通 M 持续到期，联系人/发现仍推进；真实失败指数退避，预算等待不增加 failures。
- 本地增强许可冷却时，另一个 M 仍可通过普通 UDP 或已启用 TCP 改善；混合平台 RTC 的调度不读该许可。只剩增强的 M 按 next_due 等待、不重复拨 C；远端明确暂缓后失败计数不变，解除后双方能再试。单许可连续多轮分给不同到期 M。
- 路径放弃后恢复资格，真实世代只重置一次，重复通知不绕限，关闭清理状态。

## 10. MIOPUNCH-130：完整 NAT 轮次预算


### 实施

将 NatConnector/NodeProbeBudget 的终身减法额度改为节点拥有的有限轮次额度，不新增调度器。clone、for_endpoint 重绑、主动/应答共享同一账本。禁用和显式零额度不因计时恢复权限。

在 `src/nat_connection.rs` 的增强入口取许可，`src/nat_mapping.rs` 的节点账本随 `for_endpoint` 共享，传入现有 `MaintenancePolicy` 的 F，独立公开入口沿用有效政策或其缺省值。增强轮次首次可立即开始，同一网络世代相邻启动至少间隔 P=32F，节点同一时刻最多一个增强轮次许可。next_due 从成功取得许可时计算；上一轮未结束不能复用。取消/失败不退时间名额。正常恢复发放完整轮次机会，不能逐份补一被 QAD 抢走。真实世代改变可重置等待、使旧工作失效，但先等待旧许可释放；同代重复通知、clone/重绑和观察抖动不补发。

观察前按本次允许配置一次预留 QAD 加可能 STUN 回退的完整链成本，在该轮内消费，不二次争抢节点零散余量。不得超过现有 observation 上限；成本超上限明确报告本地配置/预算限制，不算 peer 不可达或无限等待。QAD 失败后仍有 STUN 份数；早退、取消或失败的预留不转赠并发轮次。

同一增强轮持有既有 policy.peer_datagrams 上限的有限 envelope；得到 plan 后按 peer_datagram_bound 原子预扣，listener、重复探测和取消均计入原界，不缩减算法。轮次结束销毁余量，下次获准轮取得新 envelope；不是把数字改无限大，也不是每次函数重试补满。

在 `prepare_upgrade` 中保留 TCP/普通 UDP 可先尝试的顺序。直连预检查沿用 64 份准入要求和两秒本地窗口，显式包上限小于 64 时保持旧拒绝语义；合格预检查按本次操作限额执行，不扣终身余额。普通候选返回合法无直连后才尝试获取增强许可。前台仅能打洞且许可冷却时，返回有界暂缓；默认保守政策 P=160 秒，晚于默认 60 秒 caller 截止时，本次前台请求可能没有增强机会。已有直连、获准 relay、可用 TCP 以及混合平台的 RTC 各按原流程推进。

公开分步 API 与整段 upgrade 共用相同许可。`prepare` 在观察前取得一份，跨 QAD、STUN 与计划协商把该轮所有权交给返回的 `nat_punch::Attempt`；`execute` 在所属节点凭这份许可扣一次计划上界，覆盖本轮探测和原路径准备，完成/失败/取消释放占用。只保留胜出 socket 或业务反馈的 Attempt 不继续占用增强名额。分开调用 `observe → coordinate → execute` 时，让快照关联同一次观察的本地许可，clone 不产生新消费权；外部提供的有效快照在首次协调时取得本节点许可。对跨节点 Attempt 不得借用另一账本的额度。分步调用丢弃中间结果即释放，在下一次领用时可回收已过期且尚未执行的分步许可；旧快照/Attempt 再消费须失败。已在执行的工作先按原截止退出并释放，才能交给下一轮。同一轮的重复 execute、取消重试都不能再次领包。

`max_rounds` 仍限制单次调用的最多轮数；第 n+1 次增强是新一轮，须重新满足 P 和原绝对截止。控制交换不等完整冷却，不能把上轮许可传进下一次 prepare。应答端缺许可时按 126 的受保护结果显式返回暂缓，主动端即使已领本地许可也结束本轮，不将其计成路径失败。沿用同轮协议在进入实质探测前结算双方是否可继续；若为首次受限建连则保留该入口的候选/失败返回契约。状态区分占用、最早时间、观察预留和包余量。

### 验收

- 可控时钟先复现终身耗尽；新规则跨多个 P 后实际完成完整观察和探测，不只验证余额回升。
- 单 QAD 超时加两个 STUN：下一获准轮实际发起两个 STUN，不存在每次新 token 被 QAD 花掉的饥饿。
- 探测包耗尽/计划失败/早成功后下一获准轮可执行；单轮不超策略上限。
- 在 `tests/legacy_nat_runtime.rs` 保留真实 `prepare → execute` 检查，补分步 observe/coordinate、重复执行、丢弃与过期中间结果、跨账本和 `max_rounds > 1` 的入口。主动/应答/多 M 在同一节点至多一轮，同世代启动间隔至少 P；下一轮超出 caller 截止时返回暂缓，当前业务或未交付队列保持原语义。小/零额度不因等待或重绑增加。
- 难打通会话不独占机会，与 129 联合验证；真实世代恢复、重复通知不补发。
- 至少一次 Native SDK 真实 QAD→STUN→增强入口，状态与消费一致；确定性时钟加真实入口，不靠长 sleep 穷举。

## 11. MIOPUNCH-131：repair 存活判定

### 实施

在 `src/connection_coordinator_native.rs::CoordinatorHook::repair` 与 `browser-probe/src/member_connection.rs::recover_path` 共用失败结算：持有 126 的原 M lease，在失败后读取原连接当前 close_reason 与开放路径，调用平台数据资格谓词。原 M 已关闭则 Terminal；仍开且有一条获准数据路径则 Retryable（Browser 沿用对外 unreachable）；仍开而合格路径归零时由所有者记录关闭来源、关闭并返回 Terminal。已经取消或仅在等其他所有者的调用不得执行主动关闭。维持两个 API 原有返回形状；恢复成功仍要求本次或可复用的双方提交结果。旧 repair 返回结构和线格式按 126 保留。

去掉失败分支中用同一已耗尽截止再验证存活的工作，建路成功路径保留新鲜 ACK。Browser 将 `BrowserGate::existing_relay` 中对具体地址的判定复用于 repair：普通 relay 查获准 origin，成员数据中继 MID1 查当前注册的 `member_data.permits_address(address)`，RTC 仍用该 M 的 stable_id 及实际绑定查资格，MQTT/IRC 等纯信令不计。Native 沿用 `permits_repair_survival` 的资源/RTC 资格并检查当前成员资格。捕捉许可撤销与路径消失时复查当前快照，禁止仅按 custom transport ID 放行。

保留 repair 上限及 caller 更早截止。实查仅信令路径残留；若会维持已无数据路径的 M，在既有路径/资格事件入口调用同一判定，不加保活轮询。尚未被传输层放弃的物理黑洞不假称无路径，最终由真实路径/连接寿命结束。

### 验收

- relay 停顿超过 repair 窗口、未超过实际连接寿命：repair 失败后 M/原流保留，解除后传输恢复，关闭来源可追踪。
- 唯一载体移除后在传输层寿命内终止，旧流失败，新建为新 stable_id。
- 撤权/禁用导致合格路径归零及时关闭；信令不能延长业务寿命。
- 复用已验收的三成员 MID1 载体，在 Browser 的唯一合格路径是 MID1 时使公开 `recover_path` 失败，断言结果可重试、原 stable_id 与已打开的流仍收发字节。撤销该条 MID1 转发许可且无其他数据路径时，原流终止；若另有获准 relay 则保留 M。纯信令单独覆盖。与 126 共用“改善占用 M、repair 等待超时、所有者继续、原流继续”的用例。
- 更新 47/81 依赖旧主动判死的断言，保留其他核心负例与新鲜 ACK；语义变化已有授权。

## 12. MIOPUNCH-132：回落、持续选择和恢复

### 实施

先扩展 `tests/path_maintenance.rs::TcpProxy` 的 `forward_frames`，按完整帧双向丢弃并可恢复，保持注册和两端连接；在同一已打开双向流持续发送有序数据，保留健康 relay 备份。用现有 netns 覆盖受影响的 UDP 入口。先运行此入口并保存时间线；基线满足下列性质即可完成。

若需修改，先沿 `vendor/iroh/src/socket/remote_map/remote_state.rs::reevaluate_paths → src/path_selection.rs::QualityPathSelector::select` 检查周期 ping、对应路径 ACK 水位与可用备份；再看 `vendor/noq-proto/src/connection/mod.rs::pto_time_and_space` 和 `packet_builder.rs` 对最近一次待确认发送的记时。先复用每路径统计，仅在选择器缺所需输入时加窄接口。PTO≥2 是项目可疑阈值，不作死亡证据。存在合格且新鲜确认的备份时，可疑路径在所有候选比较中降级；当前可疑则绕过五毫秒防抖立即回落。之后每次重评估仍降级，不能当前换成 relay 就恢复旧 RTT 排序。

原路径取得其自身在可疑判定之后的新鲜认证 ACK 才解除本次可疑；仅计数归零、网络变化或已发 ping 不够。复用 ACK 水位/统计，状态限于活路径。所有可用路径都可疑时保留当前，不盲切不关 M。业务空闲本身不判可疑，实际健康探测仍服从 QUIC 计时。资格先于评分，恢复后沿 Quality/偏好和原防抖，不强迫 PreferRelay 切回直连。

### 验收口径

- 记录黑洞启用、首次未确认业务发送、后续真实待确认发送（含维护 ping）、实际第一/第二次 PTO、备份 ACK、选路及原流下一次有序业务确认。两端按路径分别取单调时间。在持续发送与单次在途后静默两种负载下运行；若第二次 PTO 始终未触发，照实记录并仍执行原流恢复断言。
- 当确实记录到第二次 PTO 且健康备份已有新鲜确认时，到选中备份最多一个重评估周期加预定调度容差。保持旧路径注册和黑洞直到备份承载原流，不能靠解除黑洞、新建流或手动 close 触发通过。
- 业务要求是黑洞仍持续时，健康备份在同一 M 上补齐未确认字节并收到应用确认。测试在注入故障前固定业务截止，记录其依据（该夹具的实际发送节奏、配置的重评估周期、传输重试及备份 RTT/容差），并用相同截止跑低/高 RTT 的健康备份对照。独立报告首个未确认业务字节到恢复、第二次 PTO 到选中。后续发送会更新 PTO 计时起点，禁止用首次发送 + 3×基础 PTO 代替真实事件。业务截止内仍不恢复就保留失败，区分 PTO 被推迟、备份 ACK、选择和原流重传，修复实际阻塞处。
- 黑洞持续多个周期不再选旧低 RTT 路径；解除且新鲜 ACK 后恢复正常比较；双方 stable_id/原流保持。
- 低/高 RTT，明确故障方向；10% 丢包固定种子与时长，记录 PTO/ACK/切换，不要求任意随机样本绝不切换，但禁止无恢复证据周期性切回。另测无在途包的空闲。
- 若改 noq，回归计数递增、对应路径新 ACK 清零、空闲不增长、网络重置不冒充恢复，保留已有 multipath 回归。

## 13. MIOPUNCH-134：首连 active IP 黑洞先测后改

### 实施

从 `MaintainedNetwork::connect → ResourceEndpoint::connect_attempt → ConnectionCoordinator::connect_delivery` 使用由 `SessionPurpose::from_alpn` 识别的成员业务 ALPN；另走 `connect_control_alpn` 验证受限用途。先建可用 IP 形成 active hint，使 IP 单路在测试截止内持续黑洞，保留健康且获准的 relay。在固定 caller 截止下先验证 relay-only 可完成准入和首个业务往返，再用相同条件加入 active hint，记录 `dial` 每一路开始、完成、取消和交付。

若串行 active 消耗 relay 机会，复用 AttemptContext/子取消令牌有限并发：同轮启动一条 active 和一条获准 relay，每组内部沿既有受限推进，同时最多两条拨号。整组占一次前台额度、共用 caller 截止。无 relay 不添加设施，沿既有直连/受限联系回退。

首个完成身份、准入和所需数据资格的结果才交付；同轮都成功优先直连，relay 先成功即交付不等待直连。取消回收败方，不能关闭胜方或已有其他 M。确定身份/权限拒绝沿既有 Terminal 语义，不用另一条路由掩盖；候选副作用和 guard 随操作回收。

不加固定两秒或新的先行计时器，不把另一会话 Active 标记当新 socket 可达保证。

### 验收

- 基线门槛：active 单路在固定窗口内得不到连接，relay-only 成功；加入 active 后，relay 仍应在 active 黑洞持续期间取得足够拨号时间，业务在同一 caller 截止内完成。基线实现若已满足，附实测结票。若 active 耗尽该机会，实施上述有限竞速，再增加机制断言：active/relay 同轮启动、不等待 active 超时、最多两路在途、只交付一个合格结果。
- active 可用/relay 不可用、同时成功、无 relay、只有信令无数据分别正确。
- 身份拒绝、撤权、截止、取消不被竞速绕过；败方无遗留 C/M、流、guard。
- 公开业务与受限控制的实际共享调用链验证，不在测试重写拨号逻辑。

## 14. MIOPUNCH-133：整体与多网络验收

### 实施

先在可控环境组合 125～132 和 MIOPUNCH-134，再覆盖 Linux 家宽对 Windows 手机热点、同局域网及至少一个其他可用组合。分时段重复，报告阶段耗时、尝试次数、切换、请求失败/恢复分布，记录网络、许可、政策、有效预算与代码/产物版本。设施不可用如实报告，不冒充产品通过。

至少两小时连续运行并含真实切网；它不替代 130 确定性耗尽/恢复用例。公网打洞有概率性，不能要求所有可打通组合前几次必成功；受控可复现拓扑检查首次合法进入增强，难打通真实组合检查 relay 保持、有界预算、公平尝试及可恢复。

### 验收

- 后台不拒绝前台；新旧 repair 在同一 M 争用时只有一个维护者。Browser 仅有 MID1 时保持原流，撤权后按 131 结算；远端显式暂缓和协议错误在发起端可区分。
- 复用 129 持有公开 connect 原始租约的持续业务用例：全部合格 M 在预算/轮转界内获机会，释放后同步/发现推进。demo 在 connect 后立即释放租约且每个心跳另开流，两小时公网心跳只用于衡量请求与连接寿命；原租约、原流分别用公开 SDK 持租约用例与 `tests/path_maintenance.rs` 的已有流断言验收。
- 复用 128-A/B 与 130 的真实入口证据，高 RTT、截断前的地址分组、QAD→STUN、冷却期间可行的其他方式、分步 API 许可归属与跨轮恢复可由时间线解释。
- 切换/回落保持双方 stable_id、原流、真实字节；恢复与偏好符合 132。
- active 黑洞不耗尽 relay 机会，不同 M 的非主 socket 隔离。
- 两小时结果及确定性预算测试通过，无已知范围内反例；另立缺陷必须保持对 133 的阻塞。
- 交付实际记录和必要限制，复用有效证据，不再追加无范围依据的全量检查。

## 15. 后续范围

应答方预开孔、QAD 设施根因、握手往返压缩、健康 M 复用控制通道、统一更改本地路径验证窗口和延长 repair 上限仍留后续。预算必须覆盖 QAD 失败回退，但不顺带重做设施。若验收表明本批承诺被这些问题阻塞，保留反例并回到父规格评估具体缺口，不能直接通过或无限补丁。

本次方案修订未运行新产品测试或实机验收；文中验收为后续实施要求，已有实测与本轮静态推演在第 2 节区分。
