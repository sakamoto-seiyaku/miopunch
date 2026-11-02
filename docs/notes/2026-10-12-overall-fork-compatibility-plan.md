# iroh／Noq 与 p2panda：整体兼容性整理候选

2026-10-12，当前源码 HEAD `c08e9b4b32350f0588b533d6617b31fe9c724b62`。用户要求先全盘讨论，再逐项实施；socket 切片已经完成。本轮使用 research 与 codebase-design 技能静态复核，未修改运行代码、默认配置或依赖，未编译、运行网络测试、创建任务或提交。本文提出剩余工作的分组，**不是新的架构决定或兼容认证**。

原始来源与固定版本见[上游差异清单](../evidence/fork-change-audit-2026-10-12/README.md)。该清单和先前两份[iroh 评估](2026-10-12-iroh-upstream-interface-compatibility-feasibility.md)、[p2panda 评估](2026-10-12-p2panda-and-demo-compatibility-feasibility.md)是 socket 实施前的快照，旧 token 问题已经由 [ADR 0011](../adr/0011-native-local-socket-identity.md)及[实施与 demo 回执](2026-10-12-native-socket-api-and-demo.md)取代；不把它重新列为待实施。

本轮独立背景源码复核及原版／当前行号对照见[剩余兼容性复核](2026-10-12-remaining-fork-compatibility-recheck.md)。两份本轮笔记分别记录整体方案与逐项证据，均未执行兼容测试。

## 2026-10-13 访谈确认

用户已确认的受限端点、控制句柄、正常 `Connection` 交付、新准入接口及旧 hook 调用顺序已补入 [ADR 0010](../adr/0010-preserve-upstream-early-data-interface.md)。具体设计沿用已有 API 思路。用户进一步要求尽量保留原库接口与默认行为，原来默认启用的功能继续启用；接受已验证的通用缺陷修复，以及对 Backup ACK 等变化先补证据再分类，见 [ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md)。后面的表格仍是 10 月 12 日候选快照，已确认取舍以这两份 ADR 为准；运行代码和默认值尚未修改。

用户随后确认整体归纳准确，本轮 grill-with-docs 已达到共同理解：上述端点与 hooks 设计、p2panda 独立 checked error、旧 selector／新连接级选择的接口分离、默认行为与 features／产物兼容核对，以及旧消费者和现有 demo 的双向验收，作为后续整体规格的基础。具体方法名、feature 接线、分发产物构建方式和验证计划在规格阶段收敛；Backup ACK 的通用修复分类仍需实际证据。本轮没有进入实施或创建 Plane 任务。

同日用户调用 to-spec 后，整体规格已发布为 [MIOPUNCH-91：iroh／Noq 与 p2panda fork 整体兼容性整理规格](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6d5cd846-3c1b-4c21-977e-fdafc208f876)，发布时为 Todo、ready-for-agent，包含 44 条用户故事及接口、实施分组和双向验收契约。规格通过 native relates_to 关联 M69、M84；二者原正文和状态保持。已核对发布全文及元数据。后续规格和拆票以 Plane 为工作来源，本笔记保留讨论证据；本次没有实施源码、拆分 children 或创建 blocking edges。

## 兼容目标与范围

用户已经确认的工程取向是：其他使用原库的程序尽量无需修改源码即可重新编译、正常运行；新增能力优先走新增接口，确实无法保留的契约逐项说明。这包含旧签名、可穷尽枚举、公开结构构造、trait 必需方法以及默认运行行为。仅让当前 demo 通过不能证明该目标已经成立。

建议由旧入口保持普通使用契约，新增受限入口明确选择准入、资源许可与连接专属路径能力；内部共享 QUIC、TLS、路径和 CRDT 算法。这里存在真实的接口差异，值得隔离；不需要把每个修错都变成开关，也不需要一套全局 `miopunch=true` 模式。增强 socket、诊断、资源限制分别由实际能力入口选择，不能假定使用某一个新功能就必须启用全部功能。

库接口兼容与产品应用协议兼容分开：[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)已取消旧 miopunch 接线协议兼容要求。恢复原库接口不要求新版成员协议接受旧产品节点。保留 QUIC 编码及密码实现也不等于全部时序、调度、所有第三方 ALPN 都已互操作验证。

## 剩余兼容面

| 当前改变 | 对旧消费者的影响 | 候选处理与 demo 约束 |
| --- | --- | --- |
| iroh 入站 `Accepting::into_0rtt` 从直接返回改为 `Result` | 明确源码破坏；普通端点调用者也要改代码。出站接口原本就是 `Result`，不是同一个问题 | 已有 [ADR 0010](../adr/0010-preserve-upstream-early-data-interface.md)要求保留原版接口，受限模式用独立接口。须一起设计 Builder、Incoming、Accepting、Router、IntoFuture、accept_with，阻止受限端点取得早期业务句柄，不能靠 panic、关闭句柄或调用者自律。[当前入口](../../vendor/iroh/src/endpoint/connection.rs#L803) |
| `AfterHandshakeOutcome` 增加 `AcceptData`、`AcceptRestricted` | 原枚举可穷尽，旧外部 match 编译失败 | 保留旧 hooks 与原 outcome，另设受限准入结果，内部组合旧 Reject 与新准入决策。Reject 优先、多策略求交、冲突拒绝、coordinator 完成后才放行；不能仅恢复返回类型而让门禁失效。[枚举](../../vendor/iroh/src/endpoint/hooks.rs#L23)、[连接放行](../../vendor/iroh/src/endpoint/connection.rs#L439) |
| p2panda-auth `GroupCrdtError` 增加 `PolicyRejected`，新旧 process 共用该 error | 即使只调用旧 `process`，旧穷尽 match 仍会破坏 | 原 `process` 和 error 保留；新 `process_checked` 返回包装旧错误的独立 checked error。共用一次验证／应用算法，策略必须针对因果前驱，在 graph append 前检查；根 management_auth 适配新 checked error。[错误](../../vendor/p2panda-auth/src/group/crdt/mod.rs#L35)、[入口](../../vendor/p2panda-auth/src/group/crdt/mod.rs#L511)、[产品调用](../../src/management_auth.rs#L122) |
| iroh portmapper 默认 Enabled → Disabled | 原依赖 feature 加默认映射的程序不再探测／映射，属于行为破坏 | 库恢复原默认的候选具有较清晰分工：产品已有显式 Disabled／NatPmp 接线，保留用户保存的选择。所有 low-level fixture 与 p2panda wrapper 也须核对，避免产品测试因省略配置开始探测。[默认](../../vendor/iroh/src/portmapper.rs#L37)、[产品接线](../../src/resources.rs#L437) |
| 原生 pkarr 两个旧构造入口无条件禁止 HTTP redirect | 旧依赖跳转的 publish／resolve 可能失败，签名不变但行为改变 | 旧 raw 构造保持原契约；新增受约束配置由产品显式使用，并贯穿 publisher／resolver。受限 browser lookup 仍须遵守实际 origin 许可，不能因恢复普通默认而允许绕过资源限制。[builder](../../vendor/iroh/src/address_lookup/pkarr.rs#L586)、[new](../../vendor/iroh/src/address_lookup/pkarr.rs#L605) |
| p2panda-net 缺 transport entry 从即时错误变为 identity-only lookup | 改变网络请求、等待时间、错误与调用者重试语义 | 旧默认保留即时缺信息结果；新增 resolving 选项，root 的 wrapper probe 显式选用。公开 builder hook 本身是新增能力，actor 参数 tuple 是私有接线，不应当误列为外部破坏。[fallback](../../vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs#L264) |
| p2panda-sync 对 store 关闭 transitive default features | 原 native 依赖图可能失去此前隐式开启的 SQLite／macros；sync 的 Rust 算法没有修改 | 候选是普通 native 保留原依赖默认，browser 保留 traits-only。单加 sync 默认 feature 未必覆盖原 `default-features=false` 调用者；必须验证 target、workspace 和 dev feature 合并。不能用“browser feature”去减掉已统一开启的 SQLite，也不能把 build-script `wasm_browser` cfg 直接当成 Cargo 可用的 target 条件。[manifest](../../vendor/p2panda-sync/Cargo.toml#L73)、[产品依赖](../../Cargo.toml#L50) |
| PathSelector 旧方法签名保留，但现在 peer aggregate 和每条连接都调用 | 有状态 selector 的输入范围、调用次数与选路作用变了 | 旧注册入口保持 peer 语义；新增连接级选择入口用于新约束，内部共享 scoring。不得复制其他连接的准入或 socket 所有权；保留当前 demo 的 exact connection、path、socket。[两阶段选择](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1293) |
| Noq Backup pending ACK 优先回自身路径 | 未安装 gate 的普通 Noq 也受调度变化影响 | 须明确选择：独立通用 ACK 修复，并提供原消费者不变量证据；或者限定到显式 probe／qualification 能力。两者均保留 demo 在黑洞主路仍 Available 时取得 exact backup fresh ACK 的能力。本轮不能直接认定所有上游用法都应恢复或都应改变。[调度](../../vendor/noq-proto/src/connection/mod.rs#L1124) |
| raw iroh 也安装新增 packet／application／socket predicates | API 可编译不代表默认行为或热路径开销等效；未能反查的地址会拒绝 | 保留安全过滤能力，核对普通地址和旧 sender 的原放行；只在确实需要能力时安装相关约束，不能借优化删除新路径的最终收发检查。[连接注册](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L771) |
| iroh／relay lib 不再产出 cdylib；Noq 若干声明的 example／test／bench 文件缺失 | Rust dependency 构建与动态库产物、standalone all-targets/package 是不同兼容面 | 单独处理产物与打包完整性，保持共享缓存和 SDK 固定构件的可追溯性，不盲目恢复会覆盖同名产物的混合构建。relay 的 server binary 声明和 src/main.rs 当前仍存在，不能写成 binary 被删除。[iroh manifest](../../vendor/iroh/Cargo.toml#L104)、[relay manifest](../../vendor/iroh-relay/Cargo.toml#L110)、[Noq manifest](../../vendor/noq/Cargo.toml) |

## 可以保留的新增能力与修错

socket-aware I/O、LocalSocketId、显式绑定建路已经实施，旧构造与 trait 必需方法保留；使用新绑定的底层驱动须用新发包入口，这是新增能力的明确契约。旧 source IP 不再冒充资源 ID。[ADR 0011](../adr/0011-native-local-socket-identity.md)

CID limit 配置默认仍为五、fresh PathProbe、WeakConnectionHandle::is_open、原始 UDP datagram 发送、deactivate_and_wait、带默认实现的 custom sender 扩展等可以保留。新入口的有效范围、平台降级、生命周期及触发流量仍需说明；增加方法不等于给第三方实现颁发产品许可。[CID 配置](../../vendor/noq-proto/src/config/transport.rs)、[runtime 默认桥接](../../vendor/noq/src/runtime/mod.rs)、[custom 默认方法](../../vendor/iroh/src/socket/transports/custom.rs)、[UDP 扩展](../../vendor/netwatch/src/udp.rs)

mapping cleanup ownership、删除 mapping 使用正确协议、LocallyRejected 不报告成 reachability failure、backoff 当次 clamp、关闭路径的一致性等适合作为独立修错保留，记录有意行为差异和验证理由。backoff 时间粒度边界另需核对。packet witness 是运行时 opt-in 诊断，doc(hidden) 不表示编译或性能隔离；可后续评估独立诊断 feature。[mapping](../../vendor/portmapper/src/nat_pmp.rs)、[拒绝分类](../../vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs)、[backoff](../../vendor/p2panda-net/src/discovery/backoff.rs)、[witness](../../vendor/noq-proto/src/packet_witness.rs)

## 按依赖分组实施的候选顺序

1. **先收敛全局契约和验收样例。** 固定“旧入口保原契约，新能力显式选择，内部共享算法”；列出可穷尽 match、旧 into_0rtt、旧 sender／selector、原 native features 的消费者。先定义什么才算旧程序不改代码正常工作，避免每改一处再重新讨论总目标。
2. **把 early-data、hooks、准入和业务放行当作一个设计组。** 这是剩余最强耦合点；确认普通与受限对象的可达接口，以及 Router 等接线，再做一个完整 slice。避免先改签名、后补安全。具体公开名称尚未选定。
3. **独立处理 p2panda checked error 与构建 features。** 二者不要求重写网络模型；error 与 root 错误分类配套，features 与 Native／browser 图配套，可以分别交付。
4. **集中清理普通默认与扩展模式的混用。** portmapper、pkarr、missing-address lookup 各自通过实际配置入口选择。产品接线显式保留现有行为；不引入供终端用户理解的库兼容开关。
5. **处理选路、ACK 与热路径过滤。** 先决定普通旧 callback 和新连接级 callback 的契约；ACK 修复／opt-in 按独立证据选择，不因代码是新增就自动全部关闭。最后核对诊断隔离和 standalone 产物完整性。

每组同时准备两套验收：固定上游使用方式的源码无修改编译及对应默认运行行为；当前 Native／browser demo 和该组的受限负例继续通过。整套 demo 不必在每个小函数改动后都重跑，但耦合组的最终交付必须核对实际流程。现有 socket 的 516 项及公网 demo 只证明该切片，不是上述剩余工作已通过的证据。

本轮没有给实施工作估时，也没有推断公网性能、全平台/NAT 或第三方程序全面兼容。最值得先讨论的是 early-data／hooks 这组如何隔离；其他候选尽量沿现有 Builder 和新返回类型表达真实差异。
