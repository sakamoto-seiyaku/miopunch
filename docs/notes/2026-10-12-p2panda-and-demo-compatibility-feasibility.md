# p2panda 与当前 demo 的上游接口兼容可行性

日期：2026-10-12。源码基线：`5beb2c791a929e5343576dfc3e26d7a831e2087f`。本轮按用户“先评估”的纠正，只读源码及已有证据入口；没有修改运行代码、manifest、lockfile，没有编译、测试、手机、VPN、网络操作或 Plane 写入。下文是方案评估，不是实施决定或新的验收要求。

**p2panda 的旧公开接口大体可以保住，当前 demo 也有明确的迁移接缝。** 最值得修整的是 auth 的公开错误枚举、net 的缺地址处理，以及 sync 的 native/WASM feature 选择。demo 依赖许多 iroh 新机制，但多数依赖可以继续走新增接口；少数当前与旧类型混在一起的扩展需要调整 root/browser 接线。不能由此承诺所有原版应用已经重新编译或运行通过。

本报告承接 [fork 审计](2026-10-12-iroh-p2panda-fork-change-audit.md)、[当前架构地图](2026-10-12-current-architecture-flow-and-performance-map.md)，与 [iroh / noq 对外接口评估](2026-10-12-iroh-upstream-interface-compatibility-feasibility.md)互补。原始比对材料是 hash 已核对的官方 p2panda git `083b48215964e92a564f3c83a1c607b00e94aa64`（auth/net）及 sync 0.7.1 发布 crate，见 [inventory](../evidence/fork-change-audit-2026-10-12/inventory.json)；本机 `/tmp/miopunch-fork-audit-20261012/{baselines,source-diffs,diffs}` 是本轮逐文件核查入口，不是持久交付物。

## 1. 兼容目标要分清三层

| 层次 | 本轮要保什么 | 不应推出的结论 |
| --- | --- | --- |
| Rust 源码兼容 | 原用户能继续使用旧方法、类型及穷尽 match；新增需求由新增 API 表达 | Rust ABI、二进制替换兼容；目前没有做这些承诺 |
| 库的默认行为 | 未选择新机制的原应用仍按原路径工作；独立修错可保留，但明确影响 | 必须复刻每个旧错误行为；为小 bugfix 增加大量开关 |
| miopunch 应用协议 | 当前新会话协议的身份、准入、用途、同一 M、Ready/ACK、撤销和取消不退化 | 与旧 miopunch 协议互通；[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)已明确取消这一要求 |

“其他原版应用重新编译后正常工作”应以**没有选择 miopunch 扩展的库用户**为主要回归对象。选择严格 admission gate 的端点必须继续拒绝未获准业务与受限 0-RTT，不能为了恢复一个旧签名而提前交出普通连接。需要标准浏览器、QUIC/TLS 和原 relay 的实际互操作，但无需新增旧 miopunch 协议回退分支。依据：[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)、[AdmissionGate 配置](../../src/admission_gate.rs)、[受限连接测试](../../tests/admission_gate.rs)。

## 2. p2panda-auth：一处真正公开破坏，新增策略可分开

### 2.1 可达 public surface

`p2panda-auth::group` 是公开模块；虽然 `group::crdt` 是 `pub(crate)`，其 `GroupCrdt`、`GroupCrdtError`、`GroupCrdtInnerError`、`GroupCrdtState`、`GroupCrdtInnerState`、`StateChangeResult` 均由 `group` 公开 reexport。因此不能因实现文件位于内部模块而忽略错误枚举破坏。相反，`validate` 是 `pub(crate)`，其新增 callback 参数不直接破坏外部调用。依据：[crate 根](../../vendor/p2panda-auth/src/lib.rs)、[group reexports](../../vendor/p2panda-auth/src/group/mod.rs)、[CRDT 源码](../../vendor/p2panda-auth/src/group/crdt/mod.rs)。

| 已有差异 | 源码／行为影响 | 可行处理与必要性 | 成本判断 |
| --- | --- | --- | --- |
| `GroupCrdtError::PolicyRejected(OP)` 新 variant | 原 enum 没有 `non_exhaustive`；旧穷尽 match 会编译失败，给原 enum 补 `non_exhaustive` 同样会破坏它 | 恢复原 variant 集合；新 `GroupCrdtCheckedError` 承载 `Group(original_error)` 与 `PolicyRejected(OP)`，由新 API 返回。高优先级 | 小范围 auth 与 root 错误映射调整；需要因果/拒绝回归 |
| 新 `process_checked(state, operation, check)` | 新方法是可加的；目前返回旧 enum，牵连旧用户 | 保留此方法或等效新名称，但返回 checked error；明确 reexport 新 error。原 `process` 保留完整旧 `Result` 签名和原校验顺序 | 小到中；共享实现不需要复制 CRDT 算法 |
| `process` 现在转发永真 policy | 原成功/CRDT错误流程预计可保留；签名文本虽相同，错误 enum 扩张仍破坏源码 | 把共享内部算法按 policy-error 类型分开，原路径使用 `Infallible` 或直接保留原错误类型；避免把“永真”假设变成运行时 panic／把 policy rejection 伪装成原 CRDT 错误 | 小；不需新持久 schema |
| State / InnerState 无条件 `Clone` | 新 trait impl，一般是源码增量；没有字段或 serde 形状改动 | 保留。demo 为失败不污染已应用视图而 clone 后检查，移除会增加快照管理成本 | 低；无必要为此恢复 test-only Clone |

上述提议不要求拿掉 policy。当前 callback 在操作进入 graph 之前，读取与原 CRDT 权限校验相同的**因果前驱状态**；依赖不等于当前 heads 时，临时 graph 先经原 `RS::process` 重建。不能改成对最终 current view 做提前检查，也不能只在 root 随意检查后调用原 `process`，否则会改变并发撤销的含义。原 `Resolver` 泛型、`StrongRemove` 与原 `apply_action` 仍可保留。依据：[process/validate](../../vendor/p2panda-auth/src/group/crdt/mod.rs)、[root policy 接线](../../src/management_auth.rs)、[并发到达顺序测试](../../tests/management_auth.rs)。

`Clone` 增量没有保存 policy callback 或新增 resolver 实例；state 仍是原 operations/ignore/mutual_removes/states/graph。serde derive 的字段和 bounds 未因这个补丁改变。它会复制已有 graph/snapshots，有性能成本，但错误类型分离本身不要求复制更多状态，也不要求管理日志迁移。依据：[State 定义](../../vendor/p2panda-auth/src/group/crdt/mod.rs)、[管理快照序列化及 replay](../../src/management_auth.rs)。

**demo 接线影响：** `ManagementAuth::accept_record` 调用 `process_checked(self.state.clone(), ...)`，`AuthError::Group` 当前包旧 `CrdtError`；新 checked error 需要在此显式分流。较稳妥候选是 root 保留 `Group(old_error)` 并新增自己的 policy rejection，或让 root 的 Group 包 checked error；两者影响 root API 的程度不同，需要另选。`management_store` 用 `AuthError::Group(GroupCrdtError::Inner(_))` 判缺历史，必须继续正确识别原 CRDT `Inner`。三个现有 management_auth 测试匹配 `PolicyRejected` 要同步改期望。Browser 复用 root management_auth，因此不用另维护第二份 policy。依据：[ManagementAuth](../../src/management_auth.rs)、[管理 ingest](../../src/management_store.rs)、[测试](../../tests/management_auth.rs)、[Browser 共享模块](../../browser-probe/src/lib.rs)。

## 3. p2panda-net：公开新增与内部接线分别判断

### 3.1 endpoint hook 的公开新增与内部 tuple 接线

公开入口是 `p2panda_net::Endpoint` / `p2panda_net::iroh_endpoint::{Endpoint, EndpointError, Builder, IrohConfig, EndpointAddr, RelayUrl}`。`Builder::endpoint_builder_hook` 是新增闭包方法。`builder` 与 `actors` 模块私有；`EndpointBuilderHook` 虽写作 `pub struct`，未被 reexport，外部不能按公开路径命名；`build_args` 是 `pub(crate)`，`IrohEndpointArgs` 所在 actors 私有，新增 tuple 第六项是内部变更。Builder 字段也是私有；`IrohConfig` 的四个 public 地址/端口字段未改。依据：[模块 exports](../../vendor/p2panda-net/src/iroh_endpoint/mod.rs)、[Builder](../../vendor/p2panda-net/src/iroh_endpoint/builder.rs)、[Endpoint API](../../vendor/p2panda-net/src/iroh_endpoint/api.rs)、[IrohConfig](../../vendor/p2panda-net/src/iroh_endpoint/config.rs)。

因此可以保留现有 hook 作为 opt-in 扩展，无需为了 tuple 兼容新增一套公开类型。hook 为 `Arc<Fn + Send + Sync>`，在每次 actor bind/restart 时复用；默认 None 保留 p2panda bind 管理。调用者仍不能假定覆盖任意配置：当前 hook 应用在原 secret/relay/lookup/transport 配置之后、p2panda bind_addr 之前。supervisor 重启后必须继续应用同一 policy，不能只配置第一次 endpoint。依据：[endpoint bind](../../vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs)、[supervisor](../../vendor/p2panda-net/src/iroh_endpoint/supervisor.rs)。

### 3.2 行为差异的可行分类

| 已有差异 | 影响面 | 评估建议 | 成本／必要性 |
| --- | --- | --- | --- |
| AddressBook 无 iroh transport 时改用纯 identity `EndpointAddr` | 原 `connect` 立即 `TransportInfoMissing`；现在可能启动 lookup、访问外部设施并等待。store 读取错误仍立即失败 | 候选：保留旧 `connect` 默认；新增 builder resolution policy 或新 resolving connect。仅在需要 lookup 的应用显式选择，保持资源 policy；不凭 hook 存在就认定许可 | 小到中。若要最大程度保原默认行为，这是主要 net 分离点 |
| local rejection 不再记地址/载体 Failed | 改 address book health/report，caller 仍失败；原 iroh 也已有 `LocallyRejected`，不只严格 gate 可触发 | 可保留为独立语义修正：本地 policy 拒绝确非不可达证据。明确默认行为影响，验证真正 transport failure 仍记 Failed | 小。无须为了错误降权原行为增加开关 |
| 新 `DiscoveryBackoff` / `DiscoveryBackoffConfig` reexports、`bounded`、`duration` | 原 backoff 模块私有；旧外部用户没有同名 API。新 API 是增量 | 保留新公开 pacing 接口，用于 root 自有 admitted discovery，而非启动另一组 Discovery actors | 小；demo 使用此能力 |
| increment 从可超 ceiling 改立即 clamp | 旧 Discovery actors 的默认调度也变，接近 ceiling 的一次等待会缩短；最终 ceiling 原本已存在 | 可保留为独立限额修错，记录行为影响，检查 reset/随机性/最大值。没有必要为轻微旧 overshoot 专造兼容开关 | 小；“尽量/因地制宜”不要求复制旧 overshoot |
| test_utils 随机 listener 改 20,000..30,000 | test/test_utils 对外 helper 给出的随机端口改变，不是普通运行 defaults | 可保留避免 OS 临时端口冲突，但不宣称此区间永远无占用；`test_utils` feature 用户也会看到变化 | 低 |

依据：[缺 transport 分支](../../vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs)、[connection report](../../vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs)、[backoff exports](../../vendor/p2panda-net/src/discovery/mod.rs)、[backoff 算法](../../vendor/p2panda-net/src/discovery/backoff.rs)、[listener helper](../../vendor/p2panda-net/src/test_utils.rs)。

**新增 pacing API 的参数风险：** `bounded` 检查 Duration 的排序，但 `random_increment` 使用 `as_millis()` 的半开范围；`1ns..2ns` 等合法递增值会截成空区间并导致 RNG panic。`(value + increment).min(max)` 在 clamp 前相加，对极端 Duration 也不能视为防溢出。当前 demo 传毫秒级正常范围，不构成已复现 demo 故障；新 public API 若作为稳定接口，需要校验毫秒换算后的非空/可表示范围及安全加法。依据：[bounded/random_increment/increment](../../vendor/p2panda-net/src/discovery/backoff.rs)、[demo backoff 配置](../../src/network_maintenance.rs)。

**不能损 demo 的具体接缝：** `src/main.rs::send_wrapped` 地址只放进 hook 的 `MemoryLookup`，AddressBook 未预填，因此确实依赖新 fallback；`tests/native_probe.rs::p2panda_endpoint_reaches_iroh_lookup_without_an_address_book_entry` 专门覆盖这一用途。如果旧默认恢复立即缺 transport，必须把 wrapped 客户端显式迁到新增 resolving 用法，否则这一 demo 会坏。NativeNetwork 主入口直接 bind iroh 并装 coordinator，不借这个 p2panda fallback 完成建路。依据：[wrapped CLI](../../src/main.rs)、[probe 测试](../../tests/native_probe.rs)、[Native bind](../../src/native_network.rs)。

## 4. p2panda-sync：原协议未改，feature 仍会改变原生用户

sync 的协议、traits、manager、dedup Rust 源码与发布 crate 相同；唯一正常依赖差异是 `p2panda-store default-features: true → false`。原 store 0.7.1 的默认功能是 `sqlite` 与 `macros`，sync 自己的 `default = []` 不表示其 transitive store 默认为空。现 root 自己用 SQLx 管理持久库，p2panda-net `address_book` 又显式开 store/sqlite+macros，不能据当前 workspace 能编译就推出原独立 sync native 应用不受影响。依据：[sync manifest](../../vendor/p2panda-sync/Cargo.toml)、[保留原 manifest](../../vendor/p2panda-sync/Cargo.toml.orig)、[改动说明](../../vendor/p2panda-sync/MIOPUNCH.md)、[net feature](../../vendor/p2panda-net/Cargo.toml)、[root manifest](../../Cargo.toml)。

本轮核对的原 store manifest 是本机固定 0.7.1 registry 文件 `/home/js/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/p2panda-store-0.7.1/Cargo.toml`；相同版本第一方源码入口为 [p2panda 固定 commit 的 store manifest](https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-store/Cargo.toml)。没有查询最新版本或用最新版行为替代本次固定基线。

| 候选 | 原 native 旧行为 | WASM demo | 取舍 |
| --- | --- | --- | --- |
| common store 保持 traits-only，native target 的正常 store dependency 显式 `sqlite, macros` | 原 native 即使 sync/default-features=false，仍与原来一样得到 native store 功能 | browser target 不带 native dependency，只保 common traits | 最贴近“原 native 用户不改源码”。需要界定 browser builtin target cfg；单纯 `not(wasm32)` 同时改变 WASI，不可把它称作只影响浏览器 |
| 新 `native-store` feature 作为 sync 默认，映射 `store/sqlite, store/macros`；browser 显式关闭默认并选新 wasm/traits feature | 普通默认 native 用户可保；原 native 显式关闭 sync 默认者与原 transitive store 行为仍不同 | [browser manifest](../../browser-probe/Cargo.toml)已通过 workspace 关闭 sync 默认，能迁接；其他 wasm 用户要关闭默认 | 比较明确可配置，源码不变，但不能宣称覆盖原 native 的全部 feature 组合 |
| 只恢复 store/default-features=true，再增加 wasm feature | 原 native 可保 | wasm feature 无法取消已经开启的 store default | 不够；features 加法合并，不是互斥配置 |
| 维持当前 traits-only，由所有 native 旧应用自行开 store | 要求部分原应用改 manifest，且功能/依赖图不同 | 当前 demo 可用 | 不符合本轮优先目标；除非决定接受这条兼容边界 |

workspace 使用 resolver 2，它能隔离未激活 target 的依赖 feature，以及部分 host/target 构建域；**不能取消同一 target、同一 normal package 上来自另一路依赖的 feature**。因此不能靠加 `wasm` feature、`cfg(feature)` target 表或改名同一个 package，消掉已开启的 sqlite。要分别核对 native、browser wasm32-unknown-unknown、native+browser workspace 选包方式、test_utils/dev feature；根 native 的 SQLite 功能在 native build 合并是预期，不必强行隔离。上表为 manifest 可行性推导，尚未做 Cargo feature graph 或构建实证。依据：[root resolver/依赖](../../Cargo.toml)、[browser 依赖](../../browser-probe/Cargo.toml)、[sync test_utils/dev 依赖](../../vendor/p2panda-sync/Cargo.toml)。

对 demo 的 LogSync 无需新 wire 格式：Native [offline_replica](../../src/offline_replica.rs)和 Browser [replica](../../browser-probe/src/replica.rs)直接调用 `LogSync::{...}` / `Protocol`，使用自有 store traits 和管理 consumer；Browser 仅把未改的 p2panda-net codec 文件按 path 纳入，未把 native p2panda-net actor 引入 WASM。保留协议与 traits 是低成本且必要的边界；改变 store features 不应修改管理 record 编码、完整体保存、apply/replay 或 source-scoped completion 语义。依据：[Browser codec 引入](../../browser-probe/src/lib.rs)、[snapshot](../../src/replica_snapshot.rs)、[持久 consumer](../../src/management_store.rs)。

## 5. 当前 demo 对 iroh 新机制的依赖地图

下表系统归并当前生产接线和旧正式 NAT CLI 的依赖；每个类别均有源码入口。iroh 原类型的逐项公开可达性、错误枚举与签名方案由 [iroh 评估](2026-10-12-iroh-upstream-interface-compatibility-feasibility.md)详细覆盖。本表判断**保原接口再给新 opt-in 接线，是否能继续满足 demo**，不承诺完全零修改 demo。

| demo 入口／模块 | 当前依赖的新增或改变接口 | 与旧接口并存的可行性／必须保留的行为 |
| --- | --- | --- |
| [NativeNetwork bind](../../src/native_network.rs) | `admission_gate`、多个 `hooks`、`before_data_release`；新增 `AcceptData`/`AcceptRestricted`；custom carriers、资源配置、observation、有限 CID | gate/coordinator 均显式安装。旧 hooks/outcome 可恢复原形，严格 scope 用新 outcome／新分类方法或新 gated trait；root gate 与 CoordinatorHook 迁接。新分类仍要交集而非最后一个 Accept 覆盖前面 Restrict |
| [ResourceConfig/ResourceEndpoint](../../src/resources.rs) | `relay_resource_policy`、`with_custom_addresses`、`default_path_selector`、`portmapper_config`、observation_servers；带 caller deadline 的 coordinator/connect | 配置与权限是显式 opt-in，新 builder 方法可留。资源 policy 必须在 bind、lookup/publication、cached/actor route、发送处生效，不能只在 SDK 挑候选时检查 |
| [BrowserNetwork](../../browser-probe/src/network.rs) | 同一 gate/scoped hook、relay policy、custom MQTT/IRC/member/RTC、CID config、path selector；incoming receipt/deadline/cancel、final release | 可迁接新的 scoped API，不改 browser 产品流程或新会话协议；RTC capability effective=false 时仍走获准 relay，缺数据路径清楚失败 |
| [公共 coordinator](../../src/connection_coordinator.rs)、[native hook](../../src/connection_coordinator_native.rs)、[Browser member](../../browser-probe/src/member_connection.rs) | `ConnectOptions` 新 route/deadline/cancellation；原 Incoming/Accepting context、`caller_deadline` / `attempt_cancellation`；区分 LocalRejected/DataUnavailable/Cancelled | 原 connect/options 接口可留，新上下文经新增 request/options 类型接入。所有耗时阶段消费同一个剩余预算；cancel/drop 关当前 pending M，不能重置预算或误关健康成员 M |
| [AdmissionGate](../../src/admission_gate.rs)、[BrowserGate](../../browser-probe/src/network.rs)、[ExistingMemberDataRelease](../../src/message_bootstrap.rs) | TLS gated early-data 禁止；`before_data_release`、scope/data carrier 列表；weak handles 和普通 connection 接口 | 新 gated 行为显式选用，原普通用户默认仍可走旧握手/0-RTT。demo 不依赖提前返回 0-RTT handle；保持受限 0-RTT 拒绝，不能以签名兼容绕过门控 |
| [TCP](../../src/tcp_transport.rs)、[MQTT](../../src/mqtt_transport.rs)、[IRC](../../src/irc_transport.rs)、[member forwarding](../../src/member_transport.rs)、[Browser datagram](../../browser-probe/src/custom_datagram.rs) | `CustomTransport::{resource,bind_with_portmapper_config}`、`CustomEndpoint::shutdown_handler`；`CustomSender` connection ownership/qualification 默认方法；`RecvInfo::new` | 新 trait 方法已有默认实现，原 custom transport 无需新增实现；demo adapters 明确 override。RecvInfo 新 ip_binding 为 crate-private，原 CustomAddr 构造方法保留，demo 不构造内部 IP metadata；不要误判此字段为外部 struct-literal 破坏 |
| [RTC transport](../../src/rtc_transport.rs)、[Native RTC](../../src/rtc_member.rs)、[Browser RTC](../../browser-probe/src/rtc_member.rs) | `open_*transport_path`、retain/retire/commit、exact `Path::start_probe` / noq fresh ACK；sender `can_bootstrap_connection`、`is_valid_connection_addr`、`guards_application_data`、`is_data_path_qualified` | 新 path/qualification API 可以全部保留。default false/valid 的原 custom sender 行为仍可用；RTC override 继续把 association 绑定原 live M，只准 validation/probe，fresh exact ACK 与共同 Ready/ACK 后放普通数据。不能因已有别的健康路径/旧 probe 就跳过确认 |
| [NativePathAdapter](../../src/native_path_adapter.rs)、[ControlSession](../../src/control_session.rs)、[NatConnector](../../src/nat_connection.rs) | 新 open_data_path/from、open_control/relay、retain/retire、commit；`Path::{local_socket_id,close,start_probe}` | 全部可作为新增机制保留；物理 winning socket 对原 M 的 ownership 不能退回按 peer 共享。接口封装可变，真实来源 socket 与原 connection 标识不可丢 |
| [mapping / punching](../../src/nat_punch.rs)、[scheduler](../../src/nat_scheduler.rs)、[旧正式 NAT CLI](../../src/bin/udp_nat_probe.rs) | `UdpObservation/MappingSequence/ObservationValidity`、STUN/relay 同 socket observation；traversal registration/senders/listeners/winner leases；LocalSocketId；raw send | 主要新增 API，没必要恢复成另一个 UDP socket 再伪称 QUIC socket 观察。保留 deadline、epoch、round/identity/socket 绑定、256 listeners 限额与回收。CLI 内仍有历史 adapter 控制流程，可继续运行其限定实验，不作为旧协议兼容产品分支 |
| [选路](../../src/path_selection.rs)、[maintenance](../../src/network_maintenance.rs) | 新 selector reevaluation default method；`FourTuple`、path stats、quality policy；DiscoveryBackoff 新公开 API | selector 增量可保。demo 使用原 FourTuple 的 IP/Relay/Custom 形状和访问方法；root quality policy 没要求一个新的普通 variant。若物理 socket metadata重构，保原 tuple 外观并另外取绑定 ID，不泄漏内部 token 当公网地址 |
| [Native replica](../../src/offline_replica.rs)、[Browser replica](../../browser-probe/src/replica.rs) | 已 gated 的原 `Connection::open_bi/accept_bi` 与 p2panda LogSync；新用途 ALPN | 原 stream 接口可完全保留，replica 没理由改成新的“兼容同步协议”。必须仍先获准 member/data scope，再同步、保存、apply、reopen |

**Portmapper 特别说明：当前源码默认仍是 Disabled，没有在本轮恢复 Enabled。** 恢复原 `PortmapperConfig::Enabled {}` 默认是待选兼容建议；[ResourceConfig::configure](../../src/resources.rs)总是按保存的 `PortMapping::{Disabled,NatPmp}` 显式设置，因此主 NativeNetwork/ResourceEndpoint 不会因默认恢复而自动打开网关。Browser 未启用 portmapper，wasm client 又是 no-op。低层 native probes、p2panda wrapped endpoint 及测试 fixtures 若没显式配置而编译启用了 portmapper，才可能增加网关探测；需把 demo 明确意图落在它们的 builder 上。普通原 iroh 用户恢复原默认可以改善原连通策略，但不代表 miopunch 保存的禁用选择被覆盖。依据：[当前 default](../../vendor/iroh/src/portmapper.rs)、[root features](../../Cargo.toml)、[Browser features](../../browser-probe/Cargo.toml)、[wrapped probe](../../src/main.rs)。

**0-RTT 接线规模：** 当前 `src/` 与 `browser-probe/src/` 的正式生产接线没有调用 `into_0rtt`；[admission_gate](../../tests/admission_gate.rs)与 [mqtt_upgrade](../../tests/mqtt_upgrade.rs)用它检查 early handle 必须被拒绝。因此将原 raw incoming 签名与新 fallible gated 入口分开，demo 主要迁移这些负例检查，不能反过来让它们使用无检查的旧 raw 入口获得受限句柄。旧 raw consumers 的源码行为由伴随 iroh 报告评估；本报告没有编译验证。

## 6. 必須验证的是行为边界，不能只做 API inventory

以下为将来选择兼容整理方案后所需的受影响验证地图，**没有要求本轮执行所有测试或物理矩阵**。本轮不运行它们，不把旧 receipt 等价为新源码验证。先选真正受改动影响的 subset；已有未改慢 NAT 概率/物理组合可跳过，只有 socket/scheduler/ownership 行为改动才重开相关实验。

| 优先级／边界 | 必須保持或补验证的 public 行为 | 已有可用入口与范围 |
| --- | --- | --- |
| P0 旧 auth 源码 | 原 `process` return type 与原 error variant 穷尽 match 能编译；常规、重复、resolver error、并发撤销不变；checked rejection 不入 graph | auth 原 inline CRDT tests；[management_auth](../../tests/management_auth.rs)覆盖 root policy。旧穷尽-match consumer、原错误到 checked 错误的 mapping 是需要新增的小型兼容 fixture，现有 root 测试不能替代它 |
| P0 受限/0-RTT | gated connect/accept 均拒绝 early handle；未审批/secret-only/known revoked 不得取得 ordinary 或读取历史；默认旧 endpoint/hooks 与原 0-RTT 仍正常 | [admission_gate](../../tests/admission_gate.rs)：`genesis_recovery_stays_restricted_and_early_handles_are_unavailable`、secret-holder、verified-member/revocation、default-hook、p2panda-hook；[offline_replica](../../tests/offline_replica.rs)：unadmitted。原 ungated incoming/outgoing 0-RTT 仍需独立 consumer fixture，不能仅证明严格拒绝 |
| P0 同一 M / Ready-ACK | 原 M 的双方身份、用途、nonce、fresh exact-path ACK/Ready 共同完成后才 release；错误角色/版本/长度/旧 ALPN不伤健康 M；path 改善不暗建新业务 M | [unified_connection](../../tests/unified_connection.rs)、[member_path_protocol](../../tests/member_path_protocol.rs)、[rtc_control_release](../../tests/rtc_control_release.rs)、[rtc_release_validation](../../tests/rtc_release_validation.rs) |
| P0 取消/close | caller deadline 不刷新；drop/cancel 只终止本次 original M；final gate 与 queued send 复查；late answer/late open 不复活；pump、queue、socket、port mapping能终止 | [member_attempt](../../tests/member_attempt.rs)、[negotiated_accept_context](../../tests/negotiated_accept_context.rs)、[rtc_path_qualification](../../tests/rtc_path_qualification.rs)、[rtc_association](../../tests/rtc_association.rs)、[rtc_invalidation_barrier](../../tests/rtc_invalidation_barrier.rs)。若改 portmapper cleanup，再选实际映射回收 case |
| P1 p2panda lookup/report | 旧默认缺 transport 立即失败、opt-in resolving真正查找到 peer；store error不变；local rejection不惩罚地址、真正 transport failure仍报告 Failed；hook重启持续装 gate | [native_probe](../../tests/native_probe.rs)包含 wrapped lookup 正例；[p2panda actor gate](../../tests/admission_gate.rs)；[vendor API/e2e](../../vendor/p2panda-net/tests/api.rs)。原默认 negative 与 report 区分/重启仍需针对性补覆盖 |
| P1 sync save/apply/reopen | 完整 body 校验后保存；无依赖暂存、乱序重试、重复幂等；错误记录不阻断后续；已commit record/process crash/replay一致；source scope completion不冒充全网最新 | [management_store](../../tests/management_store.rs)、[offline_replica](../../tests/offline_replica.rs)、[unified_admission_story](../../tests/unified_admission_story.rs)。feature-only 改动先跑最相关 short case；没有算法变动不必重跑全部 crash matrix |
| P1 relay / MQTT / IRC / RTC互操作 | 普通原 relay/stream 保持；MQTT/IRC是受保护 control入口，不是业务fallback；无获准数据路径明确不可达；新 RTC 失败保健康 relay，撤销/resource失效则立即退役；禁用施設不被缓存/lookup/地址覆盖绕过 | [mqtt_upgrade](../../tests/mqtt_upgrade.rs)、[mqtt_bootstrap](../../tests/mqtt_bootstrap.rs)、[irc_bootstrap](../../tests/irc_bootstrap.rs)、[resources](../../tests/resources.rs)及当前 Browser runners。MQTT/IRC用受控服务的 tests/runner，不能全都归类为纯内存快测 |
| P1 native/WASM features | 原 native sync/store sqlite+macros可见性；browser traits-only；root/browser实际 feature选择；不额外拉native SQLx networking到wasm | 原独立 consumer fixture + native/wasm分target检查；[sync manifest](../../vendor/p2panda-sync/Cargo.toml)、[Browser manifest](../../browser-probe/Cargo.toml)。Cargo.lock不记录每次激活feature，保持lock不等于feature兼容已验证 |
| P2 independent pacing fix | bounded参数非空毫秒范围、极值不panic；clamp/reset/randomness；现 maintenance预算退避与业务优先 | [backoff inline tests](../../vendor/p2panda-net/src/discovery/backoff.rs)、[network_maintenance](../../tests/network_maintenance.rs)。对新增参数边界需小型针对性补测，不为旧overshoot增兼容开关 |
| 条件式 socket/选路 | winning socket身份、epoch失效、原会话路径退休、Backup ACK scheduling；原 tuple输出和 custom defaults | [legacy_multi_socket](../../tests/legacy_multi_socket.rs)、[legacy_nat_runtime](../../tests/legacy_nat_runtime.rs)、[unified_path_selection](../../tests/unified_path_selection.rs)。若本次只分公开类型而没有改这些机制，优先限定本地 ownership/failurecases，跳过未改概率矩阵 |

### 6.1 当前脚本入口与历史脚本的区别

- [check_unified_maintained.py](../../scripts/check_unified_maintained.py)提供当前 maintained/positive/causal-witness/network-failure 受控故事及 frozen-artifact 验证；[check_current_browser_relay.py](../../scripts/check_current_browser_relay.py)覆盖 RTC API缺失、PeerConnection拒绝、DataChannel拒绝的真实 relay降级；[check_unified_browser_browser.py](../../scripts/check_unified_browser_browser.py)有 host-host/host-pixel 与 forward/both/cancel。它们是当前协议入口，但真实运行需要对应 source/artifact receipt 和环境；不是本轮可随手重跑的免费纯本地检查。
- [check_unified_admission_current.py](../../scripts/check_unified_admission_current.py)是当前 Android/Browser完整准入故事入口；[check_unified_browser_native.py](../../scripts/check_unified_browser_native.py)与 [check_maintained_browser_native.py](../../scripts/check_maintained_browser_native.py)也核 frozen inputs。选有实质受影响的平台故事即可，不自动要求所有方向、手机、网络一起完成。
- [check_browser_rtc.py](../../scripts/check_browser_rtc.py)的 `reject_retired_fixture` 明确拒绝历史 member RTC fixture并指向上述当前入口；[check_browser_rtc_cut.py](../../scripts/check_browser_rtc_cut.py)引用该 guard。旧 `wss-member-rtc` / `rtc-member-replica` 脚本名仍存在，不能把它们当当前统一协议完整故事的可直接运行证明。synthetic RTC channel、datagram/fragment单项仍可作为限定机制工具，区别于当前应用流程。
- [check_browser_network.py](../../scripts/check_browser_network.py)、[check_browser_irc.py](../../scripts/check_browser_irc.py)、[check_unified_native.py](../../scripts/check_unified_native.py)保存了短检查/历史fixture入口，其中部分内嵌直接 Cargo调用、`--skip-build` 或按小清单取source。后续选用时要遵守 [build-cache规则](../agents/build-cache.md)，经共享 `scripts/cargo_cache.py` 建立必要图一次、保留source/lock/artifact前置校验后复用；不能把任意旧 `--skip-build` 视作当前artifact正确性的证明。

### 6.2 既有证据怎么复用

已有测量与实机故事入口由 [当前架构地图](2026-10-12-current-architecture-flow-and-performance-map.md)和 [demo能力研究](2026-10-12-demo-capabilities-and-product-opportunities-research.md)记录。它们限定源码、方向、网络和完成阶段，不能由一次 Native↔Browser成功推导所有原应用/所有组合兼容。可复用未变模块的receipt作为历史基线，接口整理后必须重新绑定受影响的选定artifact；无需复跑没有改动的慢NAT概率矩阵来证明一个error enum或manifest feature方案。

## 7. 评估后的候选顺序

1. **先保真正的旧源码 surface。** auth原enum/`process`保留，新checked error分离；net hook与backoff public additions可留；不要把内部tuple/privatewrapper当作兼容工程。iroh真正旧类型修整由伴随报告逐项处理。
2. **让旧native依赖行为与Browser适配共存。** sync优先比较native target正常dependency与default native-store方案，写清显式关默认用户/WASI/dev feature边界；不改LogSync wire或管理保存语义。
3. **只对实质策略分叉显式opt-in。** 缺transport从立即失败变为外部lookup有真实副作用，可让wrapped demo显式选resolving；原本错误的local rejection健康报告与轻微backoff overshoot可作为独立fix保留并说明，无须每处开关。
4. **按受影响链保demo。** root/browser迁接新scoped hook/error/context；仍用现在的新会话协议，保证未审批/撤销/0-RTT拒绝、原M+freshACK+Ready、LogSync保存应用重开、取消关闭。先原consumer源码/feature小fixture与本地失败case，再选必要的当前Browser故事。

这轮结论是源码层面的“可行且有明确接缝”；尚未完成兼容实施和回归验证。当前 demo 可以作为不可退化的应用锚点，但不能要求通过兼容整理保留所有 fork API 的当前名字、所有不正确旧默认行为，或重新接回已取消的旧 miopunch 协议。
