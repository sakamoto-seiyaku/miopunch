# iroh / noq / netwatch / portmapper：保留上游接口的可行性评估

2026-10-12；读取基线 `5beb2c791a929e5343576dfc3e26d7a831e2087f`。本报告使用 `research` 技能，只读当前源码、已保存的发布 crate 对照和现有证据。**没有修改运行源码、编译、执行测试、操作手机、网络或 Plane；包括恢复 Portmapper 默认开启在内，下面全部改造方案均未实施。** 用户最新要求是先评估。

**可以明显改善“新增接口归新增、已有程序重新编译继续工作”的程度，同时保留 demo；但当前 fork 还不能直接宣称满足这个目标。** 最明确的两个 source break 是 incoming `into_0rtt` 返回值变化和原可穷尽 `AfterHandshakeOutcome` 新增 variants；最明确的默认行为偏离是 iroh Portmapper 默认关闭。还有不使用产品 gate 也会遇到的 pkarr redirect、selector 调用范围、Backup ACK 调度、已退休 path 状态、mapping 失败/取消清理和打包入口变化。不能靠恢复一个默认值便解决全部兼容，也不应机械回滚修复。

评估取向分为三类：

- **新增入口保留原契约**：原 enum、原签名和旧 raw 使用路径保留，新受限连接、资源限制、单连接选路从新增接口进入。
- **产品策略显式接线**：demo 的准入、资源用途、CID2、选择器和网关选择由产品配置表达；旧库消费者未选择这些能力时保留原行为。
- **底层缺陷修复有依据地保留**：正确删除自己 TCP/UDP mapping、撤回已失效的地址、closed path 状态等，可能值得直接保留；需声明实际行为变化并验证，不为追求逐包一致把错误恢复成默认。

“兼容”分别指 Rust 源码、原默认运行行为、QUIC/TLS 编码与互操作、构件/feature/targets。这里没有把这四层合并成保证。ADR 0009 取消的是旧 **miopunch 应用连接协议**兼容；本评估不恢复旧协议 fallback。受限连接的 early data 禁止、身份验证、frame 加密和普通业务授权都须保留。[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)

## 依据与公开边界

原版对照为 iroh / iroh-relay **1.2.0**、noq / noq-proto **1.3.0**、netwatch / portmapper **0.19.3** 的已保存发布源码。来源 hash、上游 commit、逐文件 hash 与差异范围在 [inventory](../evidence/fork-change-audit-2026-10-12/inventory.json)，生成说明在 [证据 README](../evidence/fork-change-audit-2026-10-12/README.md)。本轮没有查询最新上游，也没有把某个别的上游版本当作对照。完整文件变更审计见[既有报告](2026-10-12-iroh-p2panda-fork-change-audit.md)。

本轮逐个检查上述六个包的所有 Rust runtime diff、公开声明改动、manifest 改动，再沿实际 reexport 判断可达性：

| 源码位置 | 实际公开范围 | 不应误判的地方 |
| --- | --- | --- |
| iroh `endpoint` | `Accepting`、`Incoming`、`Connection`、hooks、`PortmapperConfig`、Path/PathList、`LocalTransportAddr` 常规公开 | `hooks` 和 `connection` 的模块可私有，但类型被 `endpoint` 重导出，因而是真正外部契约。[endpoint:98](../../vendor/iroh/src/endpoint.rs#L98) |
| iroh `endpoint::transports` | `CustomTransport/Endpoint/Sender`、`FourTuple`、`PathSelector` 和 context 只在 `unstable-custom-transports` feature 下公开 | 上游明确不受 semver 保证，但用户希望尽量保留，因此仍评估其迁移成本；不能按稳定 API 承诺，也不能因为原模块 private 忽略外部消费者。[endpoint:30](../../vendor/iroh/src/endpoint.rs#L30) |
| 新 UDP observation / traversal / physical binding | 多项从 `endpoint` 常规 reexport；native 项受平台 cfg 控制 | 是新增公开 API，pool 本体、其 token 函数、内部 actor 消息均不公开。[endpoint:51](../../vendor/iroh/src/endpoint.rs#L51)、[endpoint:4737](../../vendor/iroh/src/endpoint.rs#L4737) |
| noq | Connection、WeakConnectionHandle、Path 和新增 PathProbe/PathFilterStats 常规公开 | Connection driver、State、`Shared::terminal` 私有；新增字段不构成外部 struct literal break。[noq lib:80](../../vendor/noq/src/lib.rs#L80) |
| noq-proto | Connection、NetworkChangeHint、TransportConfig 等经 lib 重导出；FourTuple 常规公开 | `PathSchedulingInfo`、CidQueue 和 packet builder 内部签名不属于外部 API；行为可经公开方法观察。[proto lib:46](../../vendor/noq-proto/src/lib.rs#L46) |
| witness | `#[doc(hidden)] pub mod packet_witness`，当前没有独立 feature gate | `doc(hidden)` 不等于 private 或 test-only；外部 fixture 真实依赖它。[proto lib:30](../../vendor/noq-proto/src/lib.rs#L30) |
| netwatch / portmapper | UdpSocket、Client 常规公开 | netwatch 仅 udp.rs 有 runtime delta；portmapper `nat_pmp::Mapping` 虽写 `pub`，`nat_pmp` 模块私有且无 reexport，不能列成公开 struct 新字段破坏。[netwatch lib:12](../../vendor/netwatch/src/lib.rs#L12)、[portmapper lib:22](../../vendor/portmapper/src/lib.rs#L22) |

对照 diff 未见旧公开 trait 必需方法被删除或改签名；新增 trait 方法均有默认实现。唯一确认改动的旧公开函数返回类型是 incoming `Accepting::into_0rtt`。`Connecting::new`、`Accepting::new`、actor `bind` 和 relay handle 函数的签名修改均为内部。`RecvInfo` 和 Builder 新字段不是公开字段；PathSelectionContext 的 source 枚举私有。新增方法仍可能与第三方扩展 trait 同名，不能用静态 diff 证明任意未知消费者零改动。

## 明确的签名与 enum 断点

### Incoming early-data：能保 raw 使用，gated 使用不能靠更名就安全兼容

| 项目 | 上游 → 当前 | 外部影响与 demo 依赖 | 兼容候选及成本 |
| --- | --- | --- | --- |
| `Accepting::into_0rtt` | `IncomingZeroRttConnection` → `Result<IncomingZeroRttConnection, Accepting>` | 所有旧 caller 要适配，包括未启用新 gate 的 caller。保留的上游 example 仍按旧返回值直接 `accept_bi()`/`closed()`，已是具体静态反例；本轮未编译。[connection:803](../../vendor/iroh/src/endpoint/connection.rs#L803)、[example:148](../../vendor/iroh/examples/0rtt.rs#L148) | **原 raw Accepting 保旧返回值；新增 fallible 入口/专用 accepting 类型**有可能，但不能只加 `try_into_0rtt` 就完成：同一旧 Accepting 还能代表受限连接，旧 infallible 方法无法表达政策拒绝。专用 guarded builder/endpoint/Incoming/Accepting 包装或泛型状态区分需评估；中到高成本，涉及 accept/Router/IntoFuture 接缝。示意命名不是定案。 |
| outgoing `Connecting::into_0rtt` | 原 `Result<OutgoingZeroRttConnection, Connecting>` 保留；新 gated/control-only endpoint 一律 `Err(self)` | 没有签名破坏；新能力的合理 opt-in 行为约束。root/browser 没有调用 `into_0rtt`；demo 通过完整 handshake + hooks，依赖禁止提前放行 | 可以保留；专用类型整理时继续禁止受限 early-data。不能为了旧签名重新启用 gated early data。[connection:678](../../vendor/iroh/src/endpoint/connection.rs#L678) |
| TLS early-data 默认 | raw endpoint 仍启用；有任何 control-only ALPN 或 admission gate 就全 endpoint 禁用 | raw 默认基本保留；同 endpoint 的其他普通 ALPN 也会受新限制，不是按 negotiated ALPN 单独处理。demo 安全依赖这个限制 | raw/new endpoint 分离可更清楚表达边界；不能未经分析在混合 endpoint 上开 early data。TLS session/ticket 与 accept_with 自定义 config 路径也要核对。[endpoint:338](../../vendor/iroh/src/endpoint.rs#L338)、[tls:98](../../vendor/iroh/src/tls.rs#L98) |

需要明确一个不可同时满足的组合：**保留旧“立即可用 early-data handle”返回行为、让同一个旧类型可承载新受限 endpoint、同时不在身份/准入完成前给业务 stream 权限**。现有代码没有提供这个组合的安全实现。用 panic、返回已关闭句柄或让应用“记得别发”都不能作为正常兼容方案；延迟/拒绝所有 stream 操作也会改变旧 handle 的原契约，并要求检查每个 stream/datagram 入口。较可信候选是让新增受限模式从新类型进入；若成本过高，也可明确保留这个 source break 并记录迁移，不能把“尽量”解释成必须隐蔽绕过授权。

demo 的 Native/Browser gate 和共同协调 hook 均在完整握手之后处理，因此没有原业务对 incoming early-data 的直接依赖。现有负例明确依赖新 fallible 返回值，整理时需迁移至新受限入口并保留拒绝断言：[admission negative:370](../../tests/admission_gate.rs#L370)、[MQTT negative:315](../../tests/mqtt_upgrade.rs#L315)。专用 endpoint 包装会影响 endpoint 传递和 Router 接线；不能由“没调用 into_0rtt”推断包装改造无成本。[Native gate:606](../../src/admission_gate.rs#L606)、[Browser hook:1143](../../browser-probe/src/network.rs#L1143)、[维护 Router 接线](../../src/network_maintenance.rs)

### Hook outcome：保旧 enum，新 admission 输出另设契约

上游 `AfterHandshakeOutcome` 只有 `Accept`、`Reject { error_code, reason }`，不是 `non_exhaustive`；当前新增 `AcceptData { allowed_data_custom_ids }` 和 `AcceptRestricted`，因此旧外部穷尽 match 编译失败。`EndpointHooks::before_data_release` 是有默认 `Ok(())` 的新增方法，本身不要求所有旧 impl 重写。[hooks:23](../../vendor/iroh/src/endpoint/hooks.rs#L23)、[hooks:133](../../vendor/iroh/src/endpoint/hooks.rs#L133)

**可评估的保留方法**：旧 `EndpointHooks::after_handshake -> AfterHandshakeOutcome` 和原 enum 完全保留；新增 admission trait/output 承载 scope、允许的 custom IDs 和 protected release hook，`Builder::admission_gate` 接新增 trait。已有旧 hooks 继续执行身份后 Accept/Reject；新 gate 独立执行 scope 判定，内部状态用私有 enum 合成。这样原消费者不必知道新 variants，demo 只改 Native `AdmissionGate`、`CoordinatorHook`、BrowserGate 与其安装点，不要把新 scope 塞回旧 enum，也不要试图现在补 `#[non_exhaustive]`：补属性本身就会破坏旧穷尽 match。

合成规则须沿用：旧 hook 拒绝仍关闭；多个新 gate 的许可求交；restricted 与 data scope 冲突拒绝；要求 gate 的 endpoint 不能用旧普通 Accept 替代显式准入决定；`before_data_release` 成功后还须复查/authorize。这是接口适配候选，**不会把认证、权限或原连接身份外移成可跳过的应用约定**。[hooks:205](../../vendor/iroh/src/endpoint/hooks.rs#L205)、[connection:435](../../vendor/iroh/src/endpoint/connection.rs#L435)

demo 直接使用这些新分支：[admission_gate:769](../../src/admission_gate.rs#L769)、[CoordinatorHook:508](../../src/connection_coordinator_native.rs#L508)、[BrowserGate:1267](../../browser-probe/src/network.rs#L1267)、[Browser before_data_release:1313](../../browser-probe/src/network.rs#L1313)。这个 seam 比整套 endpoint 类型包装小，成本中等；必须核对两个 hooks 的执行次序以及连接掉 future 的回收。旧 hook default、多个旧 Reject 的 first rejection 行为与普通连接路径应保持。现有 `Builder::admission_gate` 注释仍写普通 `Accept` 会启用 candidates，但实际 gated 分支拒绝普通 `Accept`、要求 `AcceptData`；迁移契约须以运行代码为准，并同步修正说明。[builder 注释:656](../../vendor/iroh/src/endpoint.rs#L656)、[实际拒绝:452](../../vendor/iroh/src/endpoint/connection.rs#L452)

`ConnectWithOptsError` 新 `InvalidControlRoute/InvalidBootstrapRoute` 和 `ConnectingError` 新 `DataPathUnavailable/AttemptCancelled` 所在原 enum 已有 `non_exhaustive`，不是新增分支导致旧穷尽 match 的同类 source break；旧 caller 的 `_` 错误分类和重试决策可能改变，需要说明。[endpoint:1059](../../vendor/iroh/src/endpoint.rs#L1059)、[connection:601](../../vendor/iroh/src/endpoint/connection.rs#L601)

## 默认配置与资源行为

| 项目 | 上游 → 当前 / 对外影响 | demo 实际依赖 | 评估建议与待验证点 |
| --- | --- | --- | --- |
| iroh Portmapper | `Default = Enabled {}` → `Disabled`；开启 portmapper feature 的旧消费者失去默认网关探测/映射。`NatPmp` 是新增 non-exhaustive variant，通常不导致旧 match 编译失败。[portmapper:18](../../vendor/iroh/src/portmapper.rs#L18) | ResourceConfig 已显式设置 `Disabled` / `NatPmp`；TCP bind 读取同配置决定是否 NAT-PMP。[resources:437](../../src/resources.rs#L437)、[TCP:485](../../src/tcp_transport.rs#L485) | **恢复库 `Enabled` 是最明确、范围最小候选**；产品继续显式选择。同步修正 builder/default 文档，覆盖 Native 正式节点、独立 probe、p2panda raw Endpoint 和 custom TCP bind；不能从一处 ResourceConfig 推断所有 probe 已显式设置。本轮未实施，未决定产品 UI 默认。 |
| 新 `RelayResourcePolicy` | 默认 closure 全允许、`independent_uses=false`；显式 `.new` 开启按用途策略。原 RelayMap 不是 allowlist，新增最终实际资源检查不应成为 raw 默认禁用。[endpoint:191](../../vendor/iroh/src/endpoint.rs#L191) | ResourceConfig、Browser startup、contact listener 显式配置；区分 Observation/Signaling/DataRelay/Lookup | 保持 permissive raw 默认与显式策略。不是简单从 core 删除检查：peer/lookup 学到的地址、替换 relay、重试/final sender 都必须受实际用途授权。动态 callback 不重入底层锁，许可失效及时生效。 |
| 新 observation map | 不设时沿用配置 relay map；显式 observation map + data relay 需 explicit policy，否则 bind error；显式 map 强制 minimal net-report，避免额外 HTTPS/captive-portal 请求。[socket:1967](../../vendor/iroh/src/socket.rs#L1967) | demo 独立观察和 data relay 许可 | 已较好隔离为 opt-in 新入口，可保留。原 `net_report_config` 不应无故被新默认覆盖；explicit 新模式覆盖属于新契约，应文档说明。data-only relay 的 initial home/retry 仅 explicit policy 模式触发。[relay actor:1147](../../vendor/iroh/src/socket/transports/relay/actor.rs#L1147) |
| `AddressLookup::resource` | 新 default `Local`；原 impl 能继续编译/解析。Facility 根据 Lookup policy 检查；Invalid 禁止；WASM explicit resource policy 下外部 Facility 一律拒绝，因为 Fetch redirect 无法限制。[lookup:350](../../vendor/iroh/src/address_lookup.rs#L350)、[lookup:510](../../vendor/iroh/src/address_lookup.rs#L510) | demo 有显式资源 policy；地址 resolver/config 会调用 resource 检查。[resources:768](../../src/resources.rs#L768) | 不宜把 default 改成 Invalid 破坏所有 raw 第三方 lookup；也不能把 Local default 当作受限 endpoint 的充分许可声明。可新增声明式 lookup 注册/包装器，在显式受限模式要求实际 facility，旧 raw `.address_lookup` 保留。third-party 自启动任务不能由 service.publish 过滤兜底；构造/bind 前与每次请求的检查须完整。 |
| native pkarr redirect | 两个原构造入口现在**无条件** `.redirect(Policy::none())`，未受 explicit policy flag 控制；旧依赖 redirect 的 publish/resolve 会改变。[pkarr:586](../../vendor/iroh/src/address_lookup/pkarr.rs#L586)、[pkarr:609](../../vendor/iroh/src/address_lookup/pkarr.rs#L609) | demo 需要实际 origin 约束；当前 Browser 直接设置受限资源，不能借 redirect 偷用另一设施 | 可保留旧 raw 构造器默认，新增 constrained builder/client 模式，并让产品显式选择；需沿 publisher/resolver builder 传递约束，不能仅修改一处 reqwest。WASM 仍 fail-closed，除非以后确实有能约束跳转的 carrier；本轮不新增它。成本中等。 |
| DNS lookup resource | 原 resolve 签名未变；新增 `dns://origin/` 声明，parse fail 就 Invalid。[dns:114](../../vendor/iroh/src/address_lookup/dns.rs#L114) | 显式 Lookup 许可 | 合理的 fail-closed；核对原 builder 接受的合法 DNS origin 能形成该 URL，否则 raw 默认也可能误禁用。未复现，不报告已发生故障。 |
| 新 custom transport methods | `bind_with_portmapper_config` 默认调用旧 `bind`，resource 默认 None，shutdown 默认 None；旧实现仍可编译且不会自动按新网关配置关闭自己私有的探测。[custom:27](../../vendor/iroh/src/socket/transports/custom.rs#L27)、[custom:145](../../vendor/iroh/src/socket/transports/custom.rs#L145) | 本产品 TCP override gateway bind；MQTT/IRC 等声明具体 facility；RTC/member transport 使用 shutdown owner | 默认是兼容层而非第三方安全认证。新受限注册可要求声明/adapter，但 raw 旧 bind 保留。不能对未声明第三方 mapping carrier 承诺 Disabled 已限制它。 |

pkarr publisher 构造时已经创建 service task，`AddressLookupServices` 的 policy 检查发生在 add/publish/resolve；因此核对“实际设施是否允许”必须包含构造前接线及 task 生命周期，而不仅看新增 trait 的 default。源码不能证明所有任意第三方 lookup 都遵守新声明。[publisher:325](../../vendor/iroh/src/address_lookup/pkarr.rs#L325)、[services:544](../../vendor/iroh/src/address_lookup.rs#L544)

## 选路、connection-owned carrier 与物理本地身份

### PathSelector 的签名没变，context 的语义和调用次数变了

`PathSelector::select(&PathSelectionContext) -> PathSelection`、context `current/paths`、data `network_path/stats` 和 `PathSelection::none/set` 保留。新增 `reevaluation_interval` default None，不让旧 selector 自动周期醒来；产品 Quality 显式启用周期并由 actor clamp 250ms–30s。[selector:2313](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2313)、[product selector:155](../../src/path_selection.rs#L155)

但原 actor 先由一个 peer-wide context 选出地址，再把同地址应用至这个 peer 的各连接；当前仍有 peer hint selection，又在 `apply_selected_path` 对**每条 connection**重新调用同一个旧 selector，候选过滤 control、closed、不 qualified、别的 connection-owned UDP binding。签名相同不保证旧有状态 selector/peer RTT 聚合/防抖的行为相同；多连接会增加调用次数，`current` 也变成每连接自己的 selected path。[remote state:1241](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1241)、[remote state:1304](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1304)、[context:2197](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2197)

**候选**：旧 `path_selector` 保留 peer 聚合契约；新增 connection-scoped selector registration/trait（context 显示 scope）给 demo。内部仍在应用结果时执行 ownership/permit 校验，旧 selector 不可选择 forbidden 路径。无法合法应用 peer hint 时保留当前 qualified path或在内部安全选路，不复制其他连接 capability。无需恢复违反 exact-M 的旧传播行为；需要决定 raw ordinary、多连接但无 scoped resources 的路径何时继续原传播。成本中等偏高，因 actor 状态、路径 open/close/status 与旧 selector 的副作用共同影响结果。

新增 `default_path_selector` 只在没有显式高级 selector 时安装产品默认，既有 `.path_selector` 显式值不被 ResourceConfig 覆盖，方向合理；它没有解决 context 契约变化。demo 真正调用它，并在 Native/Browser 共用 Quality/偏好规则。[endpoint:985](../../vendor/iroh/src/endpoint.rs#L985)、[resources:397](../../src/resources.rs#L397)

### CustomSender 默认兼容比较充分，安全能力仍须 opt-in

新增默认分别为：`can_bootstrap_connection = is_valid_send_addr`、`is_valid_connection_addr = is_valid_send_addr`、`guards_application_data = false`、`is_data_path_qualified = is_valid_connection_addr`。因此旧 custom sender 没有自动被要求取得新 RTC qualification；其旧 peer-wide可用性基本保留。[custom:212](../../vendor/iroh/src/socket/transports/custom.rs#L212)

demo RTC override，绑定 exact original connection、association、route、qualification；member/MQTT/IRC/TCP 也各有实际实现。应保留这些 default，不把 legacy sender 统一强制改成 guarded；同时新 connection-owned register 必须要求实现必要限制。对旧普通连接也安装了 noq packet/application-data predicates，默认 sender 大多返回 allow，但它们会增加 callback/lock 工作并过滤无法反查的 mapped 地址。[register:709](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L709)、[RTC sender:1227](../../src/rtc_transport.rs#L1227)、[Native member sender:688](../../src/member_transport.rs#L688)

新 `WeakConnectionHandle::is_open` 是 additive；原 `is_alive` 保留存活分配语义，没有换成 terminal-open。产品使用 lock-free is_open 避免 callback 重入 QUIC lock。这个差别不能被“统一方法”抹掉。[noq:1574](../../vendor/noq/src/connection.rs#L1574)、[iroh:1516](../../vendor/iroh/src/endpoint/connection.rs#L1516)

### Physical local token：新增能力，不是原 IP accessor 的透明扩展

pool 将 Noq `FourTuple.local_ip` 用作物理 socket 身份，token 来自 `fd6d:696f:3631:0::/64`，kernel send 前再转真实 source IP；`LocalSocketId` 不重复使用，binding 关联原 connection/peer，pinned IP 与网络 epoch 控制生命周期。该机制服务同一 QUIC M 上的 UDP 增强打洞，不能随意移除。[pool:42](../../vendor/iroh/src/socket/transports/ip/pool.rs#L42)、[pool:87](../../vendor/iroh/src/socket/transports/ip/pool.rs#L87)

然而旧公开 `Path::local_addr -> LocalTransportAddr::Ip` 的文档仍表示本地 IP；当前 `from_noq_local_ip` 对 IP path 直接传回 Noq local，不翻译 token。新增 `Path::local_socket_id` 能识别 token，但旧读取 IP 的 caller 不会自动改用它。[transport addr:787](../../vendor/iroh/src/socket/transports.rs#L787)、[from_noq:827](../../vendor/iroh/src/socket/transports.rs#L827)、[Path:489](../../vendor/iroh/src/socket/remote_map/remote_state/path_watcher.rs#L489)

还有需要明确列出的**静态推断风险**：`from_token` 只按前缀识别；send 和 network permit 路径对符合前缀的值直接查 pool，因此正常配置的真实 IPv6 ULA 若恰在该 `/64` 内，可能被误当物理 capability，即使旧 caller 没有创建新 pool。此处未运行复现，不能报告已发生的网络故障。[from_token:55](../../vendor/iroh/src/socket/transports/ip/pool.rs#L55)、[send:1085](../../vendor/iroh/src/socket/transports.rs#L1085)、[permit:735](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L735)

**候选**：旧公开 actual-IP accessor 继续返回真实 IP；新增显式 local-binding identity/route accessor 供新 selector/probe 使用，私有运行路由仍按 connection capability 校验。必须同时核对 `PathSelectionData::network_path`、context `current` 和 `PathSelection::set`：不能只把诊断视图改成真实 IP，就让返回选择结果丢失物理 descriptor ownership。更彻底的实现可让 Noq 内部另存 opaque physical binding，不依赖合法 IP 命名空间；Noq FourTuple 字段本来私有，具备内部扩展余地，但 Eq/Hash、new/local_ip/network_path/recovery、Transmit.src_ip、pktinfo 及 iroh 的公开 unstable FourTuple 必须一并核对。不能新增公开 enum variant就假称毫无 break，也不能只按 pool 是否存在略过 namespace 混淆。身份跨越多个 owner，成本取决于所选存放与传递接口；此前“高成本”是对完整拆分范围的初步判断，不代表必须更换全部公开接口。

用户继续要求探索其他解法后，[socket 身份放置专项评估](2026-10-12-socket-identity-placement-alternatives.md)进一步列出 private PathKey、独立收发 context、connection-local registry、iroh remote alias、私有 custom UDP adapter 和过渡 facade。一个更局部的机会是：`PathSelectionData` 已有 `(conn, path_id)`，`PathSelection` 的字段私有，旧 `set(&data)` 可内部保存精确身份而保持公开签名；并非必需先新增整套 selector trait。该专项仍只做静态评估，没有修改源码或取得运行验证。

用户进一步纠正评估方向为“socket 身份应是底层已有概念，不该靠地址或额外适配绕行”。专项报告已将原生多 socket 模型列为当前评估方向：Endpoint 管理 socket 集合，既有 PathData 引用 local socket，真实地址继续独立描述通信位置，收发直接使用该绑定。具体实现与旧接口接线尚未决定；原候选列表保留为比较材料。

demo 当前依赖 `local_socket_id`、UdpTraversalLease、retain_winner、`open_data_path_from`，以及控制/adapter 对 exact socket+remote+M 的筛选；保留真实 descriptor、pinned address、original-M ownership 和 fresh ACK，才算保留 demo 增强 UDP 功能。[nat_punch:1088](../../src/nat_punch.rs#L1088)、[native adapter:304](../../src/native_path_adapter.rs#L304)、[control:289](../../src/control_session.rs#L289)

### socket ID 可放置的其他位置（评估候选）

用户询问内部 socket ID 是否可放在其他位置。可行方向不只 IP token；关键是该身份必须贯穿接收、路径识别、发送、网络变化和回收，而不是仅供诊断。以下方案尚未选择或实现。

| 位置 | 如何保持旧接口 | 主要代价与限制 |
| --- | --- | --- |
| Noq 私有路径身份／元数据中的独立可选 binding ID | 原 `FourTuple::new(remote, local_ip)` 仍构造无额外绑定的路径，`local_ip()` 继续是真实 IP；新入口设置／读取绑定身份。现有 FourTuple 字段私有，可以评估内部扩展或另设私有 path key | 要同时传递接收来源和发包目标 socket；Eq/Hash、路径匹配、重绑、许可、排队发送及回收都要理解该身份。旧路径无 binding 时须保持原语义；新的身份不是 QUIC wire 字段 |
| iroh 旁表：原 connection + path ID → binding ID | 旧地址对象可保原形，额外绑定放在 owner 的映射表；对外以新 accessor 暴露 | Noq 必须向收／发适配层提供足够的路径身份。只按 remote + local IP 建表会合并同 IP、不同端口的 socket，不能解决根因；建连初始包、验证包、CID/path 退休与晚到事件也要同步。旁表不自动意味着更少改动 |
| 用现有 connection-owned custom transport 承载额外 UDP socket | 新能力显式注册为 custom carrier，借用现有地址映射、发送与关闭接口；公开查询正确表达 Custom，而不将 socket ID 冒充 IP | 将改变该路径的 IP/Custom 分类、原生候选发现、NAT 协商和选路接线；需新增直接数据载体分类并保持原 M 的资格。更适合统一 Adapter 的探索，不能直接认定与当前 native UDP 功能等价 |

从职责和地址语义看，**优先深入评估独立的 binding 元数据**：本地 IP 表达网络地址，binding ID 表达实际 OS 资源的路由身份。当前 Noq 的 receive 会从 `RecvMeta.addr/dst_ip` 构造 FourTuple，而普通 transmit 只提供 destination/src_ip，所以增加一个内部字段并不足够。[接收入口:965](../../vendor/noq/src/endpoint.rs#L965)、[路径字段:373](../../vendor/noq-proto/src/lib.rs#L373)、[发包结构:335](../../vendor/noq-proto/src/lib.rs#L335)

传输侧可评估新增带 binding 的收／发入口或包装类型；旧 `AsyncUdpSocket::poll_recv`、`UdpSender::poll_send` 及旧 metadata 结构保留，默认桥接无额外 binding 的普通路径，新 pool Adapter 显式提供身份。不能直接给现有 public-fields 的 Transmit/RecvMeta 追加必填字段，造成另一个外部 struct-literal 编译破坏；旧 sender 不支持新绑定时也不能静默选错 socket。[旧 runtime traits](../../vendor/noq/src/runtime/mod.rs)、[公开 Transmit](../../vendor/noq-proto/src/lib.rs#L335)

无论放在哪里，都应使用与原 connection/epoch/lifetime 关联的逻辑 binding ID，而非裸 OS fd 或仅本地端口；系统资源复用后，旧标识不得路由到新 socket。物理 source IP/port、每个报文的真实接收来源、winner pin 和最终 path disposal 仍须保留。本节仅收敛可评估的位置与契约，尚未证明新方案兼容或运行通过。

## Noq 调度、gate、CID 与观测

| 项目 | 上游 → 当前 / 外部影响 | demo 依赖 | 可行性与证据限度 |
| --- | --- | --- | --- |
| packet path restriction | 无此 API → 新 `restrict_paths/restrict_network_paths`，one-way求交；driver lock 内检查入站/出站，撤回已 buffered forbidden transmit | 受限控制、resource/connection-owned路径、取消与故障 mask；iroh 每条连接注册安装部分过滤 | 新 API 可保留；raw Noq 未安装时默认 None。iroh raw 仍安装过滤，应验证 old allow 行为/成本。callback 不重入；撤回 buffered packet 与 loss recovery 交互需要实证。[noq:343](../../vendor/noq/src/connection.rs#L343) |
| 普通 frame gate | 无此 API → 新 `restrict_application_data_paths`，无 permit 时 allow；不 qualified path 禁止普通 frame、仍可验证/ACK。receive 认证解密后、dedup/ACK 前整体丢弃含普通 frame packet | RTC pending → fresh ACK/Ready → retain；即使旧主路断了也不得 fallback 到 pending RTC 普通业务 | 保留为 opt-in 动态 gate。不能删 receive 检查，只保 sender；不能 ACK 已丢弃 STREAM。需验证 whitelist、混合 frame packet、key update/flow control/close、0-RTT范围与 loss recovery。[proto:2777](../../vendor/noq-proto/src/connection/mod.rs#L2777)、[receive:4448](../../vendor/noq-proto/src/connection/mod.rs#L4448) |
| Backup ACK scheduling | 原主要 data-capable路径发送 ACK → validated Backup 有 pending ACK 时优先回自己 ACK，使用 own_acks_only/validation_only；**raw 无 gate 也会生效** | `Path::start_probe` / PathProbe 的 exact fresh authenticated ACK；黑洞 Available 主路仍保留时，RTC改善/repair必须能验证备路 | 两个合理候选：把 new scheduling 显式绑定 connection-owned qualification/probe 模式，让 raw保旧；或作为独立 multipath ACK缺陷修复保留、明确行为改动。不能预先断言是普遍正确修复，也不应为兼容破坏 demo fresh probe。[proto:1082](../../vendor/noq-proto/src/connection/mod.rs#L1082)、[scheduling:1269](../../vendor/noq-proto/src/connection/mod.rs#L1269) |
| path status / PathList | `path_status` 对 retained abandoned path 曾能返回 status → ClosedPath；iroh新 PathList过滤实际已关但actor未收到Abandoned的entry | admission release前退休信令路径，不能把stale条目作为业务候选 | 更像一致性修复，可以保留并声明。旧观测程序若依赖短期 draining状态会改变；新增 retained/draining观察 API 可满足需要。不要为它自动加产品开关。[proto:800](../../vendor/noq-proto/src/connection/mod.rs#L800)、[PathList:364](../../vendor/iroh/src/socket/remote_map/remote_state/path_watcher.rs#L364) |
| fresh PathProbe | 无此 API → `Path::start_probe`、PathProbe watermark/is_acknowledged、proto start/ack API | candidate exact M/path 的 freshness证明 | 纯新增入口方向合适；确认只在established原path，旧ACK不满足watermark、关闭原path报ClosedPath。它会产生PING/ACK流量，仅caller调用触发。[proto:2817](../../vendor/noq-proto/src/connection/mod.rs#L2817)、[noq PathProbe](../../vendor/noq/src/path.rs) |
| CID pool | 固定5 → 可配2–5，默认仍5；receive window、retire范围、transport参数使用同一值 | 仅独占IRC产品配置用2；其他默认5；路径槽位保持8 | 可保留additive配置，不回滚改善。invalid2–5边界的assert/panic作为新入口契约应说明；旧原默认和CID轮换应对照，不能把CID2当无wire差异。[config:408](../../vendor/noq-proto/src/config/transport.rs#L408)、[Native:688](../../src/native_network.rs#L688)、[Browser:1884](../../browser-probe/src/network.rs#L1884) |
| NetworkChangeHint | 新 local_ip_for_recovery default None，原impl仍编译；原清local IP行为保留，新hint能保存token | 物理binding recovery用真实IP liveness检查token存活 | 保留默认；内部token整理必须核对currentepoch、socketfinaldisposal与hint，不能保留过期capability。[proto:7424](../../vendor/noq-proto/src/connection/mod.rs#L7424)、[socket recovery](../../vendor/iroh/src/socket.rs) |
| ConnectionEvent accessors / network_paths | 新 datagram source/networkpath accessor、新 network_paths读取含opening/draining | exact本地/远端restriction，原connection路径维护 | 新API，原enum编码/构造shape未改；原路径观察与retained/draining需区分，不以network_paths条目存在证明open/qualified。[shared](../../vendor/noq-proto/src/shared.rs)、[noq:568](../../vendor/noq/src/connection.rs#L568) |
| witness | 原无 → hidden public、runtime全局install；上限16384records/16MiB，多处ring条件同步hash/copy | `rtc_packet_witness_fixture`安装；Native RTC/pump存在记录点；ordinarydemo无需observer | 可考虑独立诊断feature并显式fixture启用；依赖feature合并需检查，不能以doc(hidden)说明生产隔离。默认disabled仍有分支/context工作，启用时callback在driver锁路径，不I/O/等待/重入；不要移除frame加密或把payload观测当networksend证明。[witness:56](../../vendor/noq-proto/src/packet_witness.rs#L56)、[fixture:253](../../src/rtc_packet_witness_fixture.rs#L253) |

Backup ACK 的既有新增测试特意覆盖“仍 Available 的黑洞主路、无普通数据、备路 probe fresh ACK”，因此不能简单判成不必要功能；但这些测试不足以证明 raw 全部 multipath公平性、ACK frequency、loss/PTO、不同 path RTT、server/client、单path、非ring组合。[multipath tests](../../vendor/noq-proto/src/tests/multipath.rs)

如选 opt-in 调度，最小传递边界应沿 **existing original connection/probe qualification seam**，在创建/注册 connection 或启用 fresh probe机制时明确开启，保持validation/ACK gate和scheduler一致；不使用全局“miopunch=true”猜测，也不为每项修复新增UI配置。如选通用修复，需准备独立原因与old/new不变量回归证据，然后接受有意的行为差异。本报告不替用户选择其中一条。

## Netwatch TTL 与 mapping 生命周期

netwatch 原 UdpSocket 的 bind/send/recv/noq发送等签名均保留；新增 `poll_send_datagram` / `poll_send_datagram_from` 只走调用者选用的 raw traversal 路径。Linux/Android用ancillary per-packet TTL与IP_PKTINFO，不改socket default TTL；unsupported metadata、非Linux、ttl0/非IPv4走既有Noq UDP发送并返回degraded标记。新test hook有featuregate。原QUIC发送实现不因此自动切换为raw sendmsg。[netwatch:409](../../vendor/netwatch/src/udp.rs#L409)、[sendmsg:32](../../vendor/netwatch/src/udp.rs#L32)

这是可直接保留的新增入口；成本在跨平台语义和热路径维护，而非旧API迁移。demo traversal须继续区分degraded，不把TTL实际未应用写成成功实施；真实source选择、重绑、WouldBlock/Interrupted、同socket并行QUIC与rawprobe、取消后descriptor复用都需要核对。Linux证据不能替代Windows/macOS或Android整套后台资格。[nat scheduler:300](../../src/nat_scheduler.rs#L300)

portmapper `Client::deactivate`仍保留，新增`deactivate_and_wait`等待service处理，但不是“gateway已证实删除”：错误日志后也可能返回；pending guard取消路径仅best-effort非阻塞删除。iroh close多等每个custom handler最多1秒，再mapping最多1秒；多handler的累计延迟不是总计1秒。[portmapper:250](../../vendor/portmapper/src/lib.rs#L250)、[close:2276](../../vendor/iroh/src/socket.rs#L2276)

同时存在对旧调用者生效的修复：mapping renew/error/JoinError现在invalidate先前owned lease；取消等待mappingtask退出并释放race完成的mapping；NAT-PMP allocation已获ack后publicaddress查询失败/取消也承担cleanup；completedmapping保存TCP/UDPprotocol并按正确protocol释放。原API/Config default没改，原NAT-PMPrequest/responseencoding没改。[service:578](../../vendor/portmapper/src/lib.rs#L578)、[pending allocation:77](../../vendor/portmapper/src/nat_pmp.rs#L77)、[release:212](../../vendor/portmapper/src/nat_pmp.rs#L212)

**这些不建议为了兼容直接回滚**：释放错误protocol、丢失ack后cleanupowner、继续发布可能已失效的地址都有明确不变量理由。应把bugfix与新增acknowledged shutdown入口拆开说明；特别测试renew失败后还能否重试、watcher何时撤回、gatewayreply丢失/超时、abort与lease期限、仅删除自己mapping。旧消费者依赖“renew失败仍保留旧外部地址”会观察变化，需说明可用性/安全取舍；不能以无签名break声称逐时序不变。

## 构件、feature 与原版程序的构建入口

| 项目 | 当前事实 / 原消费者影响 | demo与可行候选 |
| --- | --- | --- |
| iroh / iroh-relay crate-type | 原 `[lib, cdylib]` → `[lib]`；Rust dependency 使用与独立 DLL/C ABI 产物是不同范围。iroh-relay 所有 Rust 源码保持原版；MIOPUNCH-52 记录 Windows PE export ordinal 限制是删去库 DLL 的实际原因。[iroh manifest:104](../../vendor/iroh/Cargo.toml#L104)、[iroh 原因:253](../../vendor/iroh/UPSTREAM.md#L253)、[relay 原因](../../vendor/iroh-relay/MIOPUNCH.md) | **适合作为因地制宜的明确例外，不能机械恢复混合 artifact 默认。** 保持当前普通 Rust 库形态；若确有旧消费者依赖 cdylib，应单独评估该目标/平台的构件入口与 export 限制。本项目上层普通 Native 库须遵守 rlib-only、JNI 单独显式生成及共享 cache lease 规则，不能为了“上游形态”重新产生会被不同 graph 覆盖的未哈希库。[build-cache:45](../agents/build-cache.md#L45) |
| noq missing targets | 保留manifest声明examples、tests、bench，实际删了对应目录 | 原消费者仅作dependency可用不代表vendor独立all-targets/package完整。可恢复pin版本的原文件；或明确裁剪package/分开fullsource distribution，并整理manifest。前者更符合“原版程序重新编译”。不得靠删除原测试冒充compat通过。[noq:118](../../vendor/noq/Cargo.toml#L118) |
| noq-proto missing bench | `benches/send_buffer.rs`未保留，但manifest仍声明 | 同上；benchmark恢复属于验证基础，不是product新feature。[proto:105](../../vendor/noq-proto/Cargo.toml#L105) |
| iroh原0rttexample | 文件仍在但未适配新的incoming返回值，后续直接调Result不存在的accept_bi/closed | 明确oldusage静态反例。保旧API方案验证应把这个unchangedexample作为主要consumer，不能仅把example改成unwrap然后宣布兼容。[example:148](../../vendor/iroh/examples/0rtt.rs#L148) |
| lock/provenance裁剪 | 若干vendor Cargo.lock、vcs/releasemetadata未保留，rootpatch/lock控制实际组合 | 不等于registry消费者会解析同dependencygraph；兼容检查须固定version/source/checksum与features，不re-resolve临时lock。恢复打包完整性应保留公开inventory证据与原artifact。 |
| feature合并 | 新miopunch-test-hooks不默认启用；packetwitness无独立gate；根workspace/build和上游workspace不相同 | 单独核对原consumer default/explicit feature，native/WASM、TLSprovider、portmapperon/off、unstableon/off。已有根build通过不证明vendorall-target或所有原应用能build。 |

上游完整all-target入口与vendor裁剪问题应先修复/明确，否则“其他原版程序重编译可正常工作”的验证范围会被自己的缺文件缩小。本轮未运行cargo，静态target缺失与上述0rtt调用反例是证据，不是本轮新测试结果。

## 新接口可以保留的范围清单

以下均无同名旧公开方法被删除的证据。保留它们并不要求强制所有旧consumer使用；它们的原connection/权限条件仍是新能力的安全契约。

- **Builder/ConnectOptions**：resourcepolicy、control-onlyALPN、admissiongate、CIDlimit、observationservers、defaultselector；bootstraproute、caller绝对deadline与attemptcancel。[endpoint:260](../../vendor/iroh/src/endpoint.rs#L260)、[ConnectOptions:2279](../../vendor/iroh/src/endpoint.rs#L2279)
- **连接metadata/诊断**：Incoming/Acceptingreceived_at与deadline/cancelsetter、Connectiondeadline/cancelgetter/pathfilterstats、Weakis_open。没有默认给所有原connection加60秒budget；caller预算与setup/lifetime分开。[connection:156](../../vendor/iroh/src/endpoint/connection.rs#L156)、[connection:1313](../../vendor/iroh/src/endpoint/connection.rs#L1313)
- **原M路径管理**：open/retain data transport、sealcontrol-data、openUDP/frombinding、primeUDP、open/retirecontrol、opencontrolrelay、commitdata；普通权限/资源gate不能被公开入口绕过。[endpoint:1153](../../vendor/iroh/src/endpoint.rs#L1153)
- **观察/traversal**：UDP/STUNmappingsequence、watcher/freshness/generation/localcandidates、boundedtraversallease/sender/listener/retainwinner/physicalID、Pathclose/startprobe/localID；观察地址不是准入或reachability证明。[endpoint:4737](../../vendor/iroh/src/endpoint.rs#L4737)
- **test-only入口**：pathIDretry、reopencontrol、openadmittedcustom、limitpaths、TTLunsupported、QNTcount受`miopunch-test-hooks`控制；与未featuregate的witness分开看。[endpoint:1106](../../vendor/iroh/src/endpoint.rs#L1106)

`PendingAdmission` drop guard现在关闭原M，即使hook私有stream另有owner；对旧普通after_handshakehook被取消也会更积极关闭连接。这是资源生命周期改变，应保留或明确评估，不可在typedwrapper重构中遗失；需要区分setupcancel原M与activeimprovecancel只撤候选。[connection:357](../../vendor/iroh/src/endpoint/connection.rs#L357)、[work/cancel:490](../../vendor/iroh/src/endpoint/connection.rs#L490)

## 从真实调用者开始的最小整理顺序（建议，未批准实施）

1. **固定oldusage反例和实际demo符号**：上游incoming0rttexample、旧hook穷尽match、rawdefaultportmapper、旧pkarrclient、statefulpeerselector；对应NativeAdmissionGate/CoordinatorHook、BrowserGate、ResourceConfig、RTCsender、NatConnector。先固定source/lock/artifact，再做任何候选；不从“重新设计所有fork”起步。
2. **独立评估Default恢复**：库Enabled+产品显式Disabled/NatPmp是小而可审查的候选；查独立probe/p2pandaendpoint及customTCP默认影响。用户本轮要求先评估，本报告没有执行。
3. **拆旧hook输出与新admission契约**：优先小seam，更新两个平台hook，保原hook签名/enum。验证reject优先、scope交叉、protectedrelease后资格复查、drop/cancel关闭原M，确保申请仍受限、已批准材料仍须durable保存。
4. **单独决策incomingearlydata边界**：先做类型/ownership设计评估，原Accepting不能同时保持infallible旧语义与任意gated表示。查Router、IntoFuture、accept_with、Connection传参成本，再决定typedwrapper还是明确sourcebreak，不为赶兼容提前暴露stream。
5. **把影响rawconsumer的策略分离**：pkarrconstrainedclient、connection-scopedselector、新lookup/custom声明注册；现有产品resource/qualification/fail-closed继续有效。公开API不能只靠全局feature切换签名：同一binary可能同时有raw和gatedendpoint。
6. **独立审查底层修复**：ACKscheduler选opt-in或有证据通用bugfix；closedpath/PathList、mappingcleanup有充分不变量则保留并声明行为变化。不为每一项增加开关，不恢复错误protocolcleanup。
7. **physical binding 专项与诊断隔离**：先补实际 IP/binding 契约和 token namespace 证据，比较私有路径 key、收发 context 与 iroh adapter 等存放/传递方案，按实际影响面判断成本；witness 独立 feature 只作为构件/成本候选。恢复/明确原 manifest targets 之后，才有完整 old consumer 构建入口。

这个顺序只整理已存在能力，不提出新功能或新产品设计。实施可拆小范围，不能把文档所有候选一次性变成必须完成的重构。

## 后续验证应沿已确定的连接、授权、record seams

本轮未运行下列检查。这是判断候选是否保demo和oldusage所需的验证点，不是已取得PASS，也不新增公网设施/产品scope。

| Seam | oldconsumer与demo需同时核对的具体事实 |
| --- | --- |
| 原raw入口 | unchanged上游0rtt调用、旧Accept/Reject穷尽match、旧CustomSender/AddressLookup/PathSelector/NetworkChangeHintimpl；不装新policy/gate时原default、redirect、peerselector及普通connect/accept行为。恢复missingtargets后验证其实际入口。 |
| 受限新入口 | Native/Browser双向connection、purpose2申请、正常成员用途；原M身份、ALPN、deadline/cancel、restricted无业务句柄、候选不提前公开、earlydata拒绝；typedwrapper不得返回能绕gate的原rawaccepting。 |
| RTC/原M改善 | exactassociation+route+freshACK，双边nonceReady、finalrevision复查、retain/rollback；relay先业务、RTC改善、carrierrepair、无RTC路径保健康relay；旧Available主路黑洞时freshbackupACK仍可取得，pendingRTC不得普通fallback。[coordinator](../../src/connection_coordinator.rs)、[RTC](../../src/rtc_transport.rs) |
| Native增强UDP/TCP | rawTTL不污染同socketQUIC，physicalID/source/pinnedbinding、token真实ULA冲突、networkepoch/descriptorfinaldisposal、socketlease取消和winnerretain；TCPgateway显式Disabled/NatPmp，mappingcleanup不误删otherprotocol/otherowner。 |
| 资源许可 | learned/explicit/lookup/substitutedroute实际设施检查，publish/resolve/redirect/relay重连最后一步仍授权；WASMexplicitpolicy保持fail-closed，third-partydefaultLocal/None不被误当已经审计。 |
| 授权与record | NativeManagementAuth/AdmissionGate和Browserdurablesnapshot仍基于已验证管理记录；未批准、撤销、rotation、migration原规则不变。manual/precode批准交付后保存grant+secret再放行；LogSync完整记录验证/保存/apply和receipt、重开持久身份仍成立。[auth](../../src/management_auth.rs)、[record](../../src/management_record.rs)、[replica](../../src/offline_replica.rs) |
| 生命周期与热路径 | droppedprotectedfuture关闭其原M；improvecancel撤候选而健康M继续；Nodeclose分别观察draining、customhandler、mappingack、自然socket/task终态。callback锁顺序、selector调用数、disabled/enabledwitness、FD和shutdown累计成本需单独计量。 |

当前代表demo故事、手机异网RTC、管理副本和IRC请求各有自己的有限证据；完整M69/M84方向/环境矩阵、默认IRCprofile、所有terminalsocket/task资格仍未齐全。[现有架构/性能地图](2026-10-12-current-architecture-flow-and-performance-map.md)、[当前mainline dossier](../evidence/current-connection-mainline-2026-10-11/README.md)、[换Wi-Fi故事](../evidence/rtc-switched-wifi-retest-2026-10-11/README.md)

TLS verifier/crypto、QUICframe/packet/coding定义、congestion实现与原发布源码相同是已有hash证据；新gate、scheduler、CID值和carrier仍改变行为，所以源码保留不等于全部wireinterop已保证。兼容整理应优先减少对raw消费者无必要的影响，同时保留准入、授权与真实路径/资源生命周期不变量。对于incomingearlydata、selectorcontext和physicaltoken，成本与安全约束需要明说；对于mapping/closedpath修复，不必为了兼容目标把缺陷重新启用。
