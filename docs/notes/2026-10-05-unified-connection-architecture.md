# 统一连接：共同协调与载体适配的实施设计

2026-10-05。依据 [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)。本文交付具体实施设计，不记录任务进度；工作状态和依赖以 Plane 为准。ADR 的行为约束已接受；以下内部类型名、模块接缝及 wire 编码是拟议实施方案，可在首个运行切片中调整并记录，毋须重新审批已确认的方向。

实施入口：[MIOPUNCH-69：统一连接流程、能力驱动建路与真实公网资格](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/96cc41d4-9049-4722-b728-656ae014d6c1)。具体切片及原生阻塞关系见该总票的子票。

## 基线和资格边界

基线为 main `9c95dcb`。已有 native/WASM SDK、成员准入、RTC lease、受限阶段 hook、原会话恢复和维护调度，不能按 README 的旧“无 runtime”表述从零重建。此前十一阶段及 MIOPUNCH-68 已完成，不重开。

`member_path_protocol::RtcRole` 当前将 browser client 固定为 offerer、native server 固定为 answerer；`rtc_member` 两侧仍有各自接线。`connection_recovery_core` 已有原 TLS exporter 绑定、原 QUIC 拨号侧开路径、fresh authenticated ACK 和 3 秒 repair cap。`path_selection` 已有 Quality/PreferDirect/PreferRelay、RTT 和 5ms 防抖，但 direct 分类尚未包含 RTC。`network_maintenance` 已提供 conservative/active 预算及联系并发，继续由它拥有 Node 维护任务。

Y700 回执证明真实 EMQX/Ergo 联系设施上的临时 RTC 互操作；实际数据选中 VPN 路径，不是完整 SDK 或公网 NAT 成功。native RTC 公网 STUN gather 超时未解决；IRC 完整受限 QUIC/TLS 引导在既有预算内未资格化。原实验的临时 AES 信令不是新产品协议，不能移植成平行握手。

## 外部 interface：业务无需选择载体

共同连接 module 向 native/WASM 包装层提供下列概念 interface，保留现有 SDK 的命名习惯与 iroh Connection/stream 语义，不要求调用者传平台组合或 MQTT/RTC：

```rust
connect(peer: PeerTarget, purpose: SessionPurpose, attempt: AttemptContext)
    -> Result<Connection, ConnectError>
accept(incoming: Incoming, purpose: SessionPurpose, attempt: AttemptContext)
    -> Result<Connection, ConnectError>
// Connection 继续提供既有 open_bi/accept_bi、closed 与路径观测。
```

`PeerTarget` 含预期身份与已获准联系线索；线索不是授权。`SessionPurpose` 复用 admission、member business、replica、recovery 的现有权限语义，不新增用途授权。`AttemptContext` 持有一次创建的 absolute deadline、取消 token、Node 维护预算租约。各阶段只消费剩余时间。入站取既有调用者/阶段预算，不给所有 native incoming 擅加新固定超时。取消一个未完成 connect 只清理其资源；取消后台优化不关闭仍健康的业务 M。

普通 Connection 只在身份、该用途权限和至少一条数据路径均通过后返回。受限 admission/control 句柄沿用原允许消息，不能因 data 尚不可达自动放开业务。原 M 终止时所有旧 stream 明确失败，后来 connect 返回新 M。

## 内部 module、所有权和 seam

| Module | Interface / 输入输出 | 唯一职责及所有权 |
| --- | --- | --- |
| ConnectionCoordinator | connect/accept；内部 improve(M, AttemptContext) | 一套能力协商、尝试顺序、权限复查、候选提交/回滚和错误分类；不实现 ICE 或 socket 打洞算法 |
| AuthorizationView | snapshot(purpose, peer)、revision watch | 复用现有 gate 与管理记录；所有 adapter 只能收取可撤回的许可，不能授予成员身份 |
| ContactDirectory | 当前可用联系入口快照与变化通知 | 区分 direct hints、获准 data relay、member signaling、公共 signaling；失效撤回 |
| NodeMaintenance | 现有 Node 生命周期、预算与事件入口 | 拥有已配置启用的兜底 listener、退避和后台改善任务；无第二重连循环 |
| NativePathAdapter | prepare(context, protected_control) → CandidateLease | native↔native 复用现有 UDP/TCP 增强打洞、观测和资源许可 |
| RtcPathAdapter | runtime_capability()；prepare(context, protected_control) → CandidateLease | native/browser 双角色；浏览器/现有库负责 ICE/DTLS/SCTP；仅承载 QUIC datagram |
| RelayPathAdapter | prepare(context) → CandidateLease | 复用 iroh relay 与显式获准 member data relay，保持端点间 QUIC 加密 |
| ProtectedContactAdapter | open_restricted(target, context) → 受限原会话 | member forwarding/MQTT/IRC 递送既有受限 QUIC 握手；没有独立密码协议 |
| SessionPathRegistry | attach/probe/retain/retire，绑定原 Connection | 将候选资格映射到 iroh 通用接口；weak M、lease、revision、IO/pump 的精确退休 |

这些是实现 seam，不全部暴露为用户配置或动态插件 interface。共享协调逻辑可放入两端均编译的 Rust module；WASM 对象线程约束和 native driver 留在 adapter。已有两个真实实现的 RTC seam 值得抽取；不要为未出现的协议建立通用插件框架。

`CandidateLease` 至少包含原 M 的本地弱引用、attempt ID、adapter route、Direct/Relay 类别、用途、资格 revision 和资源持有者。用于诊断的本地 stable ID 不能充当线上身份。跨联系会话引用原 M 时沿用私密 exporter binding；日志只存“匹配/不匹配”，不存 binding。候选租约不得写入 peer-wide bootstrap cache、跨 M 继承或跨重启保存。

## 一次连接和持续路径状态

| 状态 | 允许动作 | 转移条件 / 失败动作 |
| --- | --- | --- |
| Planning | 查许可、运行时能力、预算和当前线索 | 可用获准 direct 优先；需要打洞时可用 relay 先行；无 data 起点进入受限联系 |
| Restricted | 正常 QUIC/TLS 身份与用途 gate；必要私密控制 | member signaling 优先于配置公共 MQTT/IRC；未验证业务路径时拒绝普通 STREAM/DATAGRAM |
| Preparing | 在受保护 stream 协商 native/RTC 候选 | 端 socket 自行 STUN；候选失败继续允许的路线，deadline/cancel 收尾 |
| Probing | 在原 M、精确 route 上开 path、fresh probe | 原 QUIC dialing side 调 open；对侧观测；DataChannel open 不构成成功 |
| Committing | 复查 revision/权限、双方 ready、retain | fresh authenticated ACK + 精确 lease + 双方确认后才放行；竞态失败退休候选 |
| Active | 返回普通 Connection；业务和 replica 正常运行 | 有预算时改善路径；候选失败保留当前获准 path；不等待打洞完成才开始 relay 业务 |
| Repairing | 原 M 活着时借受限联系找新候选 | 原 binding 和 peer 匹配后按相同 probe/commit；原 M closed 优先终止 |
| Closed | 清理该 M 全部 lease、pump/queue、PC、控制句柄 | 不允许迟到事件恢复；新 connect 是新 M |

Active 的当前数据路径与 candidate 生命周期分开。候选状态为 Reserved → TransportOpen → Probed → Retained → Retired；所有中间态可退休，Retired 不可逆。候选失败不修改成员持久化材料。权限失效优先于“保留健康路径”：撤销成员资格必须终止该成员通信，禁用一项资源则立即退休其路线并仅保留其他仍获准路径。

2026-10-06 实施细化（[MIOPUNCH-74](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/5e225722-aa1c-4e3a-8d51-3474735e3af9)）：初次建立 M 时，两端 RTC adapter 各自取得精确原 M 路径的 initial fresh ACK，复查当前权限后，通过现有 `lease.retain` 和 `retain_control_data_transport_path` 建立初始路径资格、封定受限 Control 路径。共同协调器随后完成原有双边 nonce Ready；`PreparedPath.with_retain` 的 finalizer 再执行 BeforeRelease 检查、两端各自的新 final fresh probe、当前授权复查与资源所有权交接，协调器和 endpoint 的最后检查通过后才返回 ordinary Connection。共同 Ready 接管 adapter 中重复的串行确认，原 caller deadline 保留。此处记录实施选择，实际运行资格仍须由新证据确认。

初始路径已经 qualified 并受限 seal，不能称为完全未 retain 的候选；finalizer 提交的是最后的证明与持有者交接。现有底层资格按精确 M/route 控制全部 STREAM/DATAGRAM 帧类别，没有逐流 private-only permit；普通业务的边界是初始 hook 完成前不暴露 ordinary 句柄。exact Control seal 只执行一次，final probe 续验与 initial 水印分别记录。Ready 成功本身不证明所有确认字节都走 RTC，最终精确路线 fresh ACK 仍必需。已有业务流的 Active M 保持 candidate validation-only，通过受限控制会话或当前获准 carrier 确认后才提交；不复用初次建 M 的提前 qualification，也不增加逐流 permit 接口。

## 能力、角色与 wire

新协议用单一严格版本的有界受保护能力封套，移除旧 v2/v3 兼容分支。封套至少携带版本、平台运行能力集合、用途、attempt context、资源许可交集；具体 ALPN/字段编号在首个切片定稿。未知版本、超长消息、重复/矛盾能力或角色明确拒绝，不降级旧 wire。沿用现有身份、准入 secret 证明与用途协议；能力消息不能改变这些定义。

浏览器启动能力探测检查所需 API、真实 PC 和约定 DataChannel 能否创建，并清理探测 PC。缺失、配置关闭或策略禁止均广告 rtc=false；支持状态与资源许可分开保存，合并广告本次有效能力。API 可用后网络建路仍可能失败，交共同协调回退。

| 端点组合 | 可选 direct adapter | 本次发起者 / 应答者 |
| --- | --- | --- |
| Native → Native | 原生 UDP/TCP | 原生协调，不为两端都 native 强行启用 RTC |
| Browser → Native | 双方 RTC 可用时 RTC | browser offerer / native answerer |
| Native → Browser | 双方 RTC 可用时 RTC | native offerer / browser answerer |
| Browser → Browser | 双方 RTC 可用时 RTC | 当次发起 browser offerer / 对侧 answerer；交换方向重复 |

对于已有 M 的优化/修复，角色绑定该次协商上下文，不能由平台推断，也不能把恢复控制会话的 initiator 误当原 M 的 QUIC dialer。原 M 开路径仍遵守 noq 的原 client 限制。并发的两次独立 connect 有各自 M/attempt，沿用已有重复协调语义，不在两条连接间偷换 lease；同一 M 的并发改善由 coordinator 串行占用其建路租约，过时 attempt 不能提交。

DataChannel 继续采用已验证 negotiated ID 0、unordered、maxRetransmits=0，检查双方 maxMessageSize，保留现有 2048-byte datagram、64KiB 队列及 association 上限。标准浏览器互操作必须成立；不实现完整 WebRTC/RFC/音视频栈。本轮不接 TURN 配置/设施，不隐式启用，库中现有 TURN 能力无需删除。

2026-10-06 用途迁移实施选择（[MIOPUNCH-83](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/9013f9ed-719c-430d-bd2a-a1c6dcc6ee5c)）：普通成员保持 `miopunch/session/member/1` / purpose1；首次 manual 请求使用 `miopunch/session/admission/1`，precode 使用 `miopunch/session/precode/1`，二者在同一严格 MUC1 version1 封套中为受限 purpose2，以 ALPN 明确选择已有处理方法；正常副本使用 `miopunch/session/replica/1` / purpose3。已迁移用途不协商旧 ALPN 或 legacy capability 版本。显式恢复用途仍保留其原有独立权限，不能作为正常 Replica 的降级入口。

Admission A 在既有 TLS 身份和受限 stage gate 之后、`after_handshake` 内完成共同能力及原会话 nonce 确认；双方 permitted data mask 为零，不准备或保留路径、不创建 PC、不返回 ordinary 句柄。既有 invitation/TLS claim、网络/Create、稳定请求及有效 grant 检查仍由实际准入 handler 执行，AUTH/history/secret 交付和 once-only durable save 不变。Native 以实际 live SessionA revision 复查，Browser 以当前邀请、管理权限和租用的持久 store 复查；A 的观察不构造成员 proof 或 data qualification。

Replica 新建当前成员 M，沿用成员 secret proof，共同 purpose/capability、获准路径、原 M 的 fresh ACK 和双边 Ready 才进入既有 LogSync、完整记录验证、durable save/apply 与发送者 receipt。初次 foreground consumer/provider 使用原 absolute deadline 的剩余时间和既有 operation ceiling 的较小值；已维护 M 上后来独立的 sync operation 保持已有 ceiling。诊断记录实际 purpose，避免把前一条 Replica 的 path-ready 当成后来 Business 的证明。以上为实施选择；物理 Android/Browser 公网资格必须由完整 SDK 回执另行证明。

## 选路、listener 和预算

已知可达且获准 direct 优先启动；没有现成 direct 而需打洞时，获准 public/member data relay 先承载安全 M 和业务，优化在后台消费现有 Node 预算。RTC 验证后归类 direct；分类依据精确已验证 route，不因 custom transport 类型泛化。后续选择复用 Quality 默认、真实 RTT、5ms 防抖及既有显式 PreferDirect/PreferRelay。沿用 iroh 的 timer/event 选路，不加平行质量循环。UDP/TCP 并发间隔不是此架构的新产品参数。

Node 运行期间，选定且配置启用的兜底联系入口保持轻量监听，即使该节点自己 relay 健康也应接收另一端经旁信道的联系。每个 listener generation 的 ready 才可发布；失效先撤回，再由现有维护退避；旧回调不能重发 ready。不是所有备用 broker 常驻，也不是每次 connect 都先连 MQTT。默认服务名单与数量另待设施资格，不把查询四来源上限写成默认表。

无 data 起点时先可用 member signaling，再获准配置公共联系设施。若 IRC 未通过完整引导资格，则明确 unavailable 并停止该分支，MQTT 路线独立继续；不能因为短 SDP 投递成功放行 IRC 完整 SDK。没有可用数据路径时返回 DataPathUnavailable，不能透过 MQTT/IRC 偷运普通业务。

## 错误和资源结算

| 错误类别 | 对调用者的含义 | 保留/回收 |
| --- | --- | --- |
| IdentityOrAdmissionRejected | 身份、准入或用途权限不成立 | 关闭本次受限 M；不伪装网络故障 |
| ProtocolUnsupported / InvalidControl | 新 wire 不兼容、长度/上下文非法 | 拒绝本次；不进入旧协议 |
| DataPathUnavailable | 联系可达但无当前获准数据路径 | 普通句柄不返回；持久成员材料保留 |
| DeadlineExceeded / Cancelled | 本次预算耗尽/调用者取消 | 所有未提交候选、PC/pump/queue/控制句柄退出 |
| CandidateFailed | 某候选网络或验证失败 | 仅内部诊断；健康 M/当前获准路径继续 |
| RepairRetryable / SessionTerminated | 原 M 尚活 / 已终止 | 已终止旧 stream 不恢复；新 connect 单独进行 |

沿用既有错误 variant 时做稳定分类映射，不强迫全仓错误类型重写。成功后 lease 归 M 所有，不随 setup deadline 到期退休；失败/取消前已提交路径需以原 M 实际资格复查。shutdown 等待自有任务完成并记录异步 socket 回收的晚期观察，不能把强杀说成 graceful cleanup。

## iroh/noq 最小改动清单

1. 复用 EndpointHooks::before_data_release、caller deadline/cancel、受限帧门控与既有身份握手；仅补足共同新协议需要的通用上下文，不在核心判断 browser/native/MQTT/IRC。
2. 复用 CustomTransport 注册、精确资源用途限制、connection-bound lease/revision。确有缺口才补通用 route metadata，以便产品 selector 获取已验证 Direct/Relay 类别；不在核心硬编码 RTC offerer/answerer。
3. 复用 open_data_transport_path、retain_data_transport_path、retain_control_data_transport_path、原路径 fresh probe/ACK 以及 retirement。保持 Control 与 Ordinary 放行区别、原拨号侧约束和 receive-before-ACK gate。
4. 核心保持常规 native/relay 管理；原生增强 UDP/TCP、RTC、公共服务重连和平台策略留产品 adapter/coordinator。不会为了统一接口搬入 ICE、IRC pacing 或 Node 预算。
5. 每一项 fork diff 必须由纵切运行故事证明必要；已经存在的接口不重复新增包装层。不保留旧产品连接 wire 兼容，但通用 iroh 非产品消费者的语义不得无故改变。

## 最先执行与验收分层

首个最小切片：两份独立持久化 native 成员使用共同 connect/accept、新严格协议与已有获准 UDP 路径，真实双向 stream 传输；加入错误版本/取消与无业务路径拒绝。它同时固化上述 interface、状态及 fork inventory，能运行后再迁移下一种 adapter。公网 STUN 与 IRC 窄资格调查可同步先跑，不等待整个重构。

每个后续切片经相同 SDK interface 实跑，优先一个真实成功和一个关键失败，再扩大角色/环境。完整 SDK 验收包含持久身份/成员材料重开、实际 gate、新协议、原 QUIC bridge、业务收发、非空 replica 验证保存；只测 1200B RTC datagram 不够。

公网分层报告：公共设施 TLS/WSS 可达；完整受限 QUIC/TLS + gate 在 caller 预算内完成；实际数据路线和同 M fresh ACK；真实异网无 VPN/overlay NAT 数据成功。四层分别列结果。真实异网环境若未就绪，创建现场环境验证工作，不把 WSL/WARP/Tailscale 结果提升为公网 NAT。

保留首失败、原始 command exit、source/artifact hash、双方角色/路径类型/route、deadline 与 cleanup 独立耗时。公开证据不含 raw SDP、exporter、secret、私钥或临时身份材料。仅重跑受影响短回归，不重跑已完成十一阶段或扩大旧 Go/OpenSpec 门槛。公共服务预算不靠提高发送速率或延长产品 deadline 过关；IRC 失败分支明确未资格化，主流程可继续使用已资格化路线。

## 实施迁移的切片纪律

首个 Native UDP/成员 stream 故事先 prefactor 共同 seam，再固化该故事的新协议，不能一次重构全部用途。已有 Browser→Native 的迁移与新增 Native→Browser 分成两个可运行切片；Browser↔Browser 在双角色具备后接入。升级和首次准入只依赖其实际使用的已迁移方向。

每个迁移批次保持其受影响故事可运行。实施中可暂留内部旧调用者以完成 expand–migrate–contract，但不承诺旧线上协议兼容；最终有限公网整合切片明确承担 contract，清除已迁移正式入口的旧 callers/旧 wire 分支后才验收统一实现。公网 STUN/IRC 前置调查可先使用现有正式入口；其成功不自动覆盖新协议，后续必须在新版 SDK 再次验收。
