# iroh / p2panda fork 修改规模、集中度与风险审计

日期：2026-10-12。审计代码基线：`5beb2c791a929e5343576dfc3e26d7a831e2087f`。本轮只整理现有代码与证据，不实现功能，不运行 Cargo、手机或网络测试，不创建任务，不修改现有决议。完整工程的模块、连接/同步流程及性能证据入口另见 [当前架构、流程与性能地图](2026-10-12-current-architecture-flow-and-performance-map.md)。

**当前已经是实质性的 iroh 连接机制 fork，不能继续概括成“几个小接口补丁”；p2panda 则仍是很窄的适配。** iroh 修改了 22 个 Rust 文件，新增 4,719 行、删除 189 行；约 68.1% 的行变动集中在四个文件。多数 MQTT、IRC、RTC、打洞策略与产品权限映射确实在根 crate，核心改动也集中于连接、路径和底层传输机制；但 iroh / noq 的准入、调度、收包、资源回收已形成需要共同维护的机制链。已存在明确 Rust 源码不兼容和默认行为改变；未见新增私有 QUIC frame 或重写 TLS 身份机制的差异，这不等于运行行为或与原始节点全面兼容。

| 组件与原始基线 | 运行或混合 Rust 修改文件；新增 / 删除 | 测试、构建与打包差异 | 改动集中程度与判断 |
| --- | ---: | --- | --- |
| iroh 1.2.0 | 22；+4,719 / −189 | 新增两个运行模块；manifest +2/−4，含 test hook 与仅构建 `lib`；删除 crate 自带 lock 5,114 行 | 95.9% Rust 行变动在 `endpoint*` / `socket*`；连接核心较重 |
| iroh-relay 1.2.0 | 0；0 / 0 | manifest +1/−4，仅构建 `lib`；文档与 provenance 差异 | relay Rust 源码完全相同 |
| noq 1.3.0 | 3；+180 / −5 | 删除 8 个测试/示例 Rust 文件 −1,123 行及一个 benchmark −169 行；删除自带 lock 2,839 行 | 主要在 `connection.rs`：收发 gate、生命周期状态与观测 |
| noq-proto 1.3.0 | 9；+594 / −18 | `tests/multipath.rs` +185/−11；删除 benchmark −8 行、自带 lock 2,176 行 | 普通数据 gate、Backup ACK 调度、CID 参数、观测；不只是 accessor |
| netwatch 0.19.3 | 1；+152 / −1 | manifest 新增 test hook；补许可证及说明 | 全部运行差异在 `udp.rs` 原始报文发送 |
| portmapper 0.19.3 | 2；+97 / −6 | 删除自带 lock 2,382 行；补许可证及说明 | 服务停止与 NAT-PMP 已拥有映射的回收 |
| p2panda-auth 固定官方 git 的 0.7.1 | 1；+25 / −6 | 补许可证及说明；manifest 未改 | CRDT 因果前驱处的应用策略入口与 Clone |
| p2panda-net 同一固定官方 git 的 0.7.1 | 5；+109 / −31 | 两个 test 文件 +9/−4；manifest 未改 | endpoint 适配与 discovery 退避；其余网络组件未改 |
| p2panda-sync 0.7.1 发布 crate | 0；0 / 0 | normal store dependency `default-features` 改 true → false；manifest +1/−1 | 同步协议、manager、scheduler Rust 源码完全相同 |

合计：43 个 `src` 下非独立测试的 Rust 差异文件，+5,876/−256。上表简称“运行或混合”，包括文件里的 inline test、comments、观测和格式差异，**不是 5,876 行新增产品逻辑**；未做精确 production/test LOC 拆分。全部文件差异为 87 个、+7,169/−14,213；其中 12,511 行删除来自四个 vendored `Cargo.lock`，不能据总删除量判断“删掉大量底层实现”。完整原始源码仍包含大量未改文件，也不应将整个 vendor 目录体积当成 fork 规模。逐文件统计与源码 hash 见 [审计证据说明](../evidence/fork-change-audit-2026-10-12/README.md)、[完整 inventory](../evidence/fork-change-audit-2026-10-12/inventory.json) 和 [统计摘要](../evidence/fork-change-audit-2026-10-12/summary.tsv)。

## 统计口径与可复核基线

使用原始发布 `.crate` 或经过校验的官方 git archive，对当前 `vendor/` 做 `git diff --no-index --numstat` 与逐文件源码对照。统计包含新增、删除文件；运行文件的 test 代码不做猜测式扣除。上表运行范围排除独立 `tests/`、`examples/`、`benches/`，这些差异另列。本报告没有用 vendoring 提交相对本仓库空目录的 diff 来代表上游修改。

发布 crate 的 `.cargo_vcs_info.json` 记录：iroh / iroh-relay 源于 `n0-computer/iroh` 的 `17c0612f80f78f5288e97b818b1360ae6ea0a51a`；noq / noq-proto 源于 `n0-computer/noq` 的 `c1f411562e6078852749b8bcf1190523096a107f`；netwatch / portmapper 源于 `n0-computer/net-tools` 的 `0030e57fc70981895bb86212ff0f059cc41c8a4e`。p2panda-auth / net 是官方 `p2panda/p2panda` 的 `083b48215964e92a564f3c83a1c607b00e94aa64` 固定源码，不把根 workspace 的继承 manifest 展开误记成源码修改；sync 则直接对发布 crate。原始 archive SHA-256 如下，供后续重建同一比对材料。

| 原始材料 | SHA-256 |
| --- | --- |
| iroh 1.2.0 | `b2f8d1cfffc83efe39a1031aab423ce09cb8048071baba550508931e9a81ce46` |
| iroh-relay 1.2.0 | `beb2294a9749d6a25fd7cd8bcf0fccd932f20716d4f967d85d3f23c7135ae6a2` |
| noq 1.3.0 | `b78be567e796cfa74bb9bdc4117790af2d505eb887019cf8803244353eb09d89` |
| noq-proto 1.3.0 | `7c1e5b6fe668491eca022f745a0a9402585626c73a7b839b3424ace15d6a9c8f` |
| netwatch 0.19.3 | `39da9cad5f23aa43401f09497d3b280e51b5feba115d9ffbf38ca35d6ff95e96` |
| portmapper 0.19.3 | `ca97242f016e090a25330613bc2382aab65eb22f9149dbe84d8f8e711d49a530` |
| p2panda-auth / net 官方 git archive | `bf5cbf9d6bc03b420b0ffe060a75eff164fb5e11241e33bb91347889292803c5` |
| p2panda-sync 0.7.1 | `ab2dfec4540ec2021949ae1e90788948528f51d76e50827895d821ffda76272e` |

本次机器可复核清单已保存于 [fork-change-audit-2026-10-12](../evidence/fork-change-audit-2026-10-12/README.md)。本机 `/tmp/miopunch-fork-audit-20261012/` 还含原始 `baselines/`、`diffs/*.patch` 与 `source-diffs/`，供当前研究复核；临时目录不是持久交付入口或运行验证回执。当前选择关系以 [根 manifest](../../Cargo.toml) 的 `[patch.crates-io]` 与 [根 lockfile](../../Cargo.lock) 为准；vendor 内说明文件仅是辅助材料。local patch 保留原 package 版本号，不能据版本号相同推断可无缝替换上游库。

## iroh 的改动集中在哪里

以下列出全部 22 个运行或混合 Rust 差异文件；每个数值是与 iroh 1.2.0 发布源码的逐文件差异。

| 当前文件 | 新增 / 删除 | 实际职责 |
| --- | ---: | --- |
| [socket.rs](../../vendor/iroh/src/socket.rs) | +1,395/−23 | 同 socket 观察、raw traversal registration/lease、多 socket 接入、epoch、路径接口、shutdown |
| [remote_state.rs](../../vendor/iroh/src/socket/remote_map/remote_state.rs) | +951/−52 | connection 所属路径、控制/数据阶段、permit、提交/退休、质量重评估与候选过滤 |
| [endpoint.rs](../../vendor/iroh/src/endpoint.rs) | +528/−9 | Builder 配置、资源策略、observation/traversal/path API、caller deadline/cancellation |
| [ip/pool.rs](../../vendor/iroh/src/socket/transports/ip/pool.rs) | +384/0，新文件 | 物理 UDP socket 身份、ownership、池与释放 |
| [stun.rs](../../vendor/iroh/src/socket/stun.rs) | +373/0，新文件 | 原 socket 上的 STUN 请求与严格解析、候选/序列观察 |
| [endpoint/connection.rs](../../vendor/iroh/src/endpoint/connection.rs) | +249/−30 | handshake 后 admission 和 data release、取消 guard、0-RTT 返回约束 |
| [transports/custom.rs](../../vendor/iroh/src/socket/transports/custom.rs) | +139/0 | 自定义 transport 的 resource、connection qualification、shutdown 接口与 wrapper |
| [transports.rs](../../vendor/iroh/src/socket/transports.rs) | +138/−28 | physical binding 元数据、最终 custom send 检查、connection qualification |
| [relay/actor.rs](../../vendor/iroh/src/socket/transports/relay/actor.rs) | +130/−26 | relay/resource 用途许可在连接、重连与发送位置生效 |
| [endpoint/hooks.rs](../../vendor/iroh/src/endpoint/hooks.rs) | +87/−3 | 受限/普通 scope、多个 hook 的交集与 before-data-release 阶段 |
| [net_report.rs](../../vendor/iroh/src/net_report.rs) | +65/0 | 独立观察资源、用途许可、受约束请求 |
| [address_lookup.rs](../../vendor/iroh/src/address_lookup.rs) | +61/−2 | lookup facility 声明、publish/resolve 用途许可 |
| [path_watcher.rs](../../vendor/iroh/src/socket/remote_map/remote_state/path_watcher.rs) | +53/−8 | 原连接的物理 binding 保留与最终 path disposal |
| [remote_map.rs](../../vendor/iroh/src/socket/remote_map.rs) | +46/−2 | 传递连接阶段、精确 connection/path 请求 |
| [transports/ip.rs](../../vendor/iroh/src/socket/transports/ip.rs) | +36/0 | UDP pool 的接入、来源元数据与发送 |
| [portmapper.rs](../../vendor/iroh/src/portmapper.rs) | +23/−3 | Disabled 默认、仅 NAT-PMP 模式与等待回收 |
| [tls.rs](../../vendor/iroh/src/tls.rs) | +20/−2 | 受限或 gated endpoint 禁止 TLS early data |
| [address_lookup/pkarr.rs](../../vendor/iroh/src/address_lookup/pkarr.rs) | +16/−1 | facility 声明与请求限制 |
| [endpoint/quic.rs](../../vendor/iroh/src/endpoint/quic.rs) | +14/0 | 暴露 CID 参数及 fresh path probe |
| [address_lookup/dns.rs](../../vendor/iroh/src/address_lookup/dns.rs) | +7/0 | facility 声明 |
| [net_report/reportgen.rs](../../vendor/iroh/src/net_report/reportgen.rs) | +2/0 | 传递独立观察配置 |
| [transports/relay.rs](../../vendor/iroh/src/socket/transports/relay.rs) | +2/0 | 传递资源策略 |

按新增加删除 4,908 行作为“行变动”口径，`socket.rs` 与 `remote_state.rs` 占 49.3%，加入 `endpoint.rs` 后占 60.3%；加入新 `ip/pool.rs` 与 `stun.rs` 后占 75.7%。其中前四个文件（不计 `stun.rs`）合计 3,342 行，占 68.1%；`endpoint*` 与 `socket*` 合计占 95.9%。这说明**按模块位置集中，按机制责任已不薄**。`socket.rs` 的变动量相当于其原始 2,866 行的 49.5%，`remote_state.rs` 相当于原始 1,523 行的 65.9%；这些比例是 churn 相对原文件大小，不表示相同比例的旧代码被重写。

53 个原始 Rust 文件保持字节相同，原始 73 个 Rust 文件中的 20 个被改，另新增两个。集中度不能说明上游升级容易：socket owner、peer actor、handshake hook、QUIC driver、packet scheduler 的组合边界都有修改，跨文件不变量比文件数更决定合并成本。

## 核心机制与根 crate 策略的边界

[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md) 已取消旧连接协议兼容要求，并要求核心集中在控制阶段、传输注册、路径验证/保留/退休，具体旁信道、增强打洞和 RTC 协商由适配器及共同模块承担。当前源码总体沿这个方向，不能把根 crate 新增模块算成“改坏上游的代码”。

| 根 crate 拥有的规则或流程 | vendor 提供的共同机制 |
| --- | --- |
| [connection_coordinator.rs](../../src/connection_coordinator.rs)、[connection_coordinator_native.rs](../../src/connection_coordinator_native.rs)：统一 ALPN、能力交换、用途、尝试预算、角色分工 | endpoint hook、阶段 gate、原 connection 的路径接口与 cancellation/deadline 传递 |
| [native_path_adapter.rs](../../src/native_path_adapter.rs)、[nat_connection.rs](../../src/nat_connection.rs)、[tcp_punch.rs](../../src/tcp_punch.rs)：具体建路、打洞策略及尝试顺序 | 同 descriptor 的 raw send、观察与 physical socket lease、QUIC path validation |
| [mqtt_transport.rs](../../src/mqtt_transport.rs)、[irc_transport.rs](../../src/irc_transport.rs)、[message_bootstrap.rs](../../src/message_bootstrap.rs)：载体与受保护联系流程 | 通用 custom transport 注册、facility 许可、control/data packet gate |
| [rtc_transport.rs](../../src/rtc_transport.rs)、[rtc_association.rs](../../src/rtc_association.rs)、[rtc_ice_profile.rs](../../src/rtc_ice_profile.rs)：RTC association、ICE 配置及 negotiation | connection-owned custom 地址、普通数据 qualification 与 probe；没有把完整 RTC 栈放进 iroh |
| [resources.rs](../../src/resources.rs)、[path_selection.rs](../../src/path_selection.rs)、[network_maintenance.rs](../../src/network_maintenance.rs)：资源配置、质量偏好、维护预算/退避 | resource callback 的调用位置、selector 接口、actor 的有界 reevaluation timer |
| [management_auth.rs](../../src/management_auth.rs)：genesis/管理员/成员权限映射、signed record 验证 | p2panda-auth 的因果前驱回调；未在 fork 中写死产品角色 |

边界仍有值得后续单独整理的部分：iroh 中有 STUN 解析和 observation freshness、raw traversal registration/framing、每轮 256 与 endpoint 1,024 socket 上限、gate 的阶段状态机；这些是为本项目扩展出的机制而非单纯透传接口。判定哪些必须留在共享 socket/QUIC owner，哪些可做独立 helper，需要逐个不变量讨论，不能只按“代码在 vendor”便断言违背 ADR。现有 MQTT/IRC/RTC 名称、broker/server 选择、NAT 策略分类和产品 permission policy 没有被整体移入 iroh。

## p2panda 的全部实际差异

### p2panda-auth

唯一 Rust 修改是 [group/crdt/mod.rs](../../vendor/p2panda-auth/src/group/crdt/mod.rs)。与 [固定官方源码](https://github.com/p2panda/p2panda/blob/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-auth/src/group/crdt/mod.rs) 对照，有四项：

1. `GroupCrdtError` 新增 `PolicyRejected(OP)`。
2. `GroupCrdtState` 及其 inner state 从仅 test/test_utils 下 Clone，改为正式构建也 Clone；成员和序列化形状没有随此变化。
3. 新 `process_checked(state, operation, callback)`；原 `process` 调用它并传永远 true 的回调，旧入口仍保留上游 CRDT 的验证路径。
4. `validate` 在必要的临时因果 predecessor rebuild 完成后、`apply_action` 和 operation 入 graph 前调用策略。回调看到的是该 operation 的因果前驱 group，而不是任意最新全局 view；拒绝直接返回独立错误。

StrongRemove resolver、graph rules、state transitions 和独立的其他 Rust 文件没有修改。产品调用者 [management_auth.rs](../../src/management_auth.rs) 先验证 signed header、body、依赖与 log 连贯性，克隆旧 state，再以产品 `policy(anchor, causal, op)` 尝试处理，成功后才替换 state / 更新 retained history。因而“未改 CRDT resolver”与“产品接受的 operation 集合变窄”同时成立；不同策略的调用者不能仅凭用同一 CRDT 就推断有同一收敛输入集。

风险是 callback 及 Clone 的契约、策略的一致性与历史重放成本，不是由 25 行增加推断的 CRDT 已损坏。源码可确认新 error variant 的穷尽 match 兼容问题；没有在本轮重跑 CRDT suite 或对 arbitrary policy 做收敛证明。

### p2panda-net

与 [固定官方 crate 目录](https://github.com/p2panda/p2panda/tree/083b48215964e92a564f3c83a1c607b00e94aa64/p2panda-net) 对照，运行差异完全落在以下五个文件。该 crate 的 manifest 字节相同，但 `workspace.dependencies` 的最终版本和 features 由根 workspace 决定，不能据此说整个依赖图与官方 workspace 相同。

| 文件 | 实际差异及调用者影响 |
| --- | --- |
| [iroh_endpoint/builder.rs](../../vendor/p2panda-net/src/iroh_endpoint/builder.rs) | 新 public `endpoint_builder_hook` 保存 `Arc<Fn(Builder) -> Builder>`，每次 actor bind 复用；hook 的 `Send + Sync + 'static` 与闭包所有权成为应用契约 |
| [iroh_endpoint/actors/endpoint.rs](../../vendor/p2panda-net/src/iroh_endpoint/actors/endpoint.rs) | 内部 actor 参数 tuple 多一个 hook；配置基础 builder 后应用 hook，再按 p2panda config 绑定；address book 没有合适 transport entry 时从“立即 TransportInfoMissing”变为仅 identity 的 iroh lookup |
| [iroh_endpoint/actors/connection.rs](../../vendor/p2panda-net/src/iroh_endpoint/actors/connection.rs) | 区分 connect 与 connecting 的 `LocallyRejected`，避免本地准入拒绝被上报成 reachability failed；其他失败仍报告，caller 仍收到 error |
| [discovery/backoff.rs](../../vendor/p2panda-net/src/discovery/backoff.rs) | 新 `Config::bounded`、`Backoff::duration`，increment 现在当次便 clamp 到 max；旧 upstream 允许一次 overshoot 再下一次 clamp |
| [discovery/mod.rs](../../vendor/p2panda-net/src/discovery/mod.rs) | 对外重导出 `DiscoveryBackoff` 与 `DiscoveryBackoffConfig`，供应用自己的 coordination timer 复用 |

两项测试差异：[iroh_endpoint/tests.rs](../../vendor/p2panda-net/src/iroh_endpoint/tests.rs) 适配内部五元 → 六元 tuple；[test_utils.rs](../../vendor/p2panda-net/src/test_utils.rs) 把随机 listener port 改为 `20000..30000`，避免上游 `49152..65535` 与本机 ephemeral client ports 重叠。生产 bind port 没有因此改变。`build_args` 是 `pub(crate)`，actor 类型沿 private module，**这项 tuple 修改不应列成外部公开 API 已破坏的证据**。

Address-book fallback 是实际行为改变：缺条目时可能开始 lookup/network activity，错误种类、耗时与谁负责限制请求都会变化；原先依赖即时 `TransportInfoMissing` 的上层逻辑需重新确认。hook 每次 bind 都运行，因此不能假定只执行一次，也不能假定它返回的 secret key、lookup 或 transport 配置仍符合 p2panda actor 的上层身份预期。actor 的 accept、report 和 shutdown ownership 仍由 p2panda 保留。

有一个静态 API 边界反例：`bounded` 比较原始 `Duration`，但 `random_increment` 将边界 `as_millis()` 后交给 `random_range`。例如 min=1ns、max=2ns 通过现有校验，却变成 `0..0` 空 range；该实现不会实现声明的有效有界随机增量。极大 Duration 转成 `u64` 毫秒以及 `value + increment` 也值得单独做输入约束核对。本轮没有运行 Rust 复现；当前根 [network_maintenance.rs](../../src/network_maintenance.rs) 使用明确毫秒范围，这不是已证实的现行产品故障。

`p2panda-net/UPSTREAM.md` 只描述 builder hook、fallback 和 test ports，漏掉 connection actor 的 policy rejection 过滤，以及 discovery 的新增 API/clamp；“runtime local delta is limited to…” 已与源码不符。auth 的说明与实际单文件 delta 相符。此处只记录证据漂移，未顺手改 vendor 文档。

### p2panda-sync 与未 patch 的依赖

sync 全部 11 个 Rust 文件与发布 crate 字节相同。唯一 manifest 行修改是 [Cargo.toml](../../vendor/p2panda-sync/Cargo.toml) 中普通 `p2panda-store` dependency 的 default features true → false；dev dependency 仍可另开 features，[Cargo.toml.orig](../../vendor/p2panda-sync/Cargo.toml.orig) 保留上游形式。作用是避免仅用 storage traits 的路径隐式拉 SQLite/SQLx 的 native networking；调用者需要自行启用实际存储 features。这是 dependency feature 行为变化，不是 sync wire format、log reconciliation 或 scheduler 的修改。

根 [Cargo.toml](../../Cargo.toml) 未 patch `p2panda-core`、`p2panda-store`、`p2panda-discovery`；根 [Cargo.lock](../../Cargo.lock) 三者仍为 registry 0.7.1，checksum 分别 `a3d46f52382b5e1352454fca91d846e23354f9189c913fd0d4a4b2ba57f13d61`、`2efce57a23952d06691f83c11ec05d3218431bac7367dc3a15168b20e57c00df`、`5d7c6a61d53a54bdcd5315d136f4c58ee7ba2f2b18abc0a7d99d2393184d122d`。这里指使用未修改的上游源码，不承诺应用启用的 features、存储 schema 或业务 record 内容与其他 p2panda 应用相同。

## 明确的不兼容与需要分开的兼容层

| 层次 | 已由源码确认的事实 | 调用者应理解的边界 |
| --- | --- | --- |
| Rust source API | `Accepting::into_0rtt()` 从 `IncomingZeroRttConnection` 改为 `Result<IncomingZeroRttConnection, Accepting>` | 是明确 source-breaking；即便 caller 没用 admission gate，也要适配返回值。对照 [上游](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/connection.rs) 与 [当前](../../vendor/iroh/src/endpoint/connection.rs) |
| Rust enum | 非 `non_exhaustive` 的 `AfterHandshakeOutcome` 新 `AcceptData` / `AcceptRestricted`；auth `GroupCrdtError` 新 `PolicyRejected` | 外部旧穷尽 match 会不再覆盖全部 variants；[hooks](../../vendor/iroh/src/endpoint/hooks.rs)、[auth](../../vendor/p2panda-auth/src/group/crdt/mod.rs) |
| Rust trait | `AddressLookup::resource`、`EndpointHooks::before_data_release`、custom transport/sender qualification、`PathSelector::reevaluation_interval`、`NetworkChangeHint::local_ip_for_recovery` 都有默认实现 | 旧 impl 不因缺少新 method 就当然编译失败；但默认 Local / unguarded / no cleanup 也不等于满足产品资源和隔离语义 |
| 已有 non-exhaustive enum | `PortmapperConfig::NatPmp`、新 connecting errors | 新 variant 不能直接作为外部穷尽 match source-breaking 例子；需要检查 callers 的 fallback 含义 |
| 默认 runtime | Portmapper default Enabled → Disabled | 原启用 portmapper feature、依赖默认自动探测/映射的消费者会改变行为；本产品显式选择与其他 raw consumers 分开评估 |
| TLS early data | control-only 或 admission-gated endpoint 禁用 early data，拒绝 0-RTT/0.5-RTT handle | 常规 TLS 握手仍可用；resumption/early-data 行为与原 fork 不同。不是 TLS wire encoding 被重写 |
| 应用线协议 | 根 crate 的 `miopunch/session/*/1`、能力帧和产品 admission policy | 有意要求新版协议、拒绝不满足 policy 的节点；ADR 0009 已取消旧接线兼容目标。不能以此单独证明“QUIC 与上游不兼容” |
| QUIC 参数与调度 | 可配置 active CID limit 2–5，默认 5；实际 transport parameter 与 receive window 改用此配置；普通 frame gate 与 Backup ACK scheduling 有修改 | 没有新 frame encoding，但启用配置会改变交换的参数值，gate/scheduler 会改变何时及何处收发报文，不能宣称所有 wire 行为完全相同 |
| artifact/feature | iroh/relay 不再构建 `cdylib`；sync 不再隐式开 store default features | Rust library 使用与 DLL/C ABI 构建、存储 feature 自动开启是不同兼容面 |

逐文件 hash 对照还确认：iroh 的 `src/tls/**/*` 四个 verifier/支持文件，noq-proto 的 `congestion/**/*` 四个实现、`frame.rs`、`packet.rs`、`coding.rs` 及 `crypto/**/*` 两个文件均与原始源码相同。可据此说明现有 TLS 验证支持、密码实现、QUIC frame/packet 定义和拥塞控制实现保留；[tls.rs](../../vendor/iroh/src/tls.rs) 本身仍改了 early-data 配置，`connection/mod.rs` 仍改了收发调度与 gate。源码不变的这些文件不构成全面互操作测试。

新 trait method 的默认实现保留了较多原始调用方式，不应一概记成破坏；同样，不应从默认实现存在反推整套 fork 可无修改替换原始 iroh。源级、行为级、产品协议级、QUIC/TLS encoding、artifact 五种结论必须分别写。

## 2026-10-12：用户补充的兼容性取向

用户明确希望新增能力尽量通过新增接口提供，已有接口尽量不改，其他基于原库的程序重新编译后仍能正常工作；遇到具体约束时因地制宜。这是用户确认的工程取向，不是本轮研究自行推导的决定，也不是无条件零变更承诺。

后续整理同时考虑源码和行为兼容：保留旧签名、旧枚举分支、原默认值与已有功能；产品的准入、资源许可和选路策略优先在 miopunch 接线处显式配置。确实必须改变底层公共契约时，逐项说明原因、受影响调用者和可用迁移方式。新增 trait 方法有默认实现只能说明较多旧 impl 可继续编译，还须核对其原有运行行为。

端口映射是一个具体分层例子。MIOPUNCH-39 为保持原先未启用 portmapper feature 的 miopunch 节点不自动开始网关探测，在打开该 feature 后同时把 fork 的库默认值改成 Disabled；详见 [当时的实现记录](2026-10-01-udp-portmap-validation.md)。当前产品 [ResourceConfig 接线](../../src/resources.rs) 已显式把 Disabled / NatPmp 传给 builder，因此保护产品旧配置不探测，并不要求所有原始 iroh 调用者也改用 Disabled 默认。按本次兼容性取向，更合适的整理候选是保留 iroh 的原默认值，让 miopunch 的本地网关选择继续由产品接线显式决定。这是待核对的候选；本轮未改默认值或运行代码，也未决定普通用户界面的端口映射默认体验。

该取向不自动恢复 ADR 0009 已取消的旧 miopunch 连接协议兼容目标：上游库调用接口的兼容与本产品旧版本应用协议的兼容是不同范围。

用户随后要求启用子代理全面评估，并明确纠正为“不是直接改动，而是先评估”。后续 [iroh 兼容可行性](2026-10-12-iroh-upstream-interface-compatibility-feasibility.md) 与 [p2panda / demo 接线评估](2026-10-12-p2panda-and-demo-compatibility-feasibility.md) 逐项列出保旧接口、新扩展、独立修错的候选及验证范围；包括端口映射在内，当前运行源码和默认值均未因此修改。

## noq、netwatch、portmapper 的底层差异与风险

noq 的三个运行文件为 [connection.rs](../../vendor/noq/src/connection.rs) +143/−2、[lib.rs](../../vendor/noq/src/lib.rs) +4/−3、[path.rs](../../vendor/noq/src/path.rs) +33/0。connection driver lock 下执行 receive-before-QUIC 与 send-before-carrier 的完整 four-tuple predicate；`restrict_network_paths` 组合既有 predicate，并撤掉已经 buffered、现在 forbidden 的 transmit。另增加普通应用 frame gate 的转接、`WeakConnectionHandle::is_open` 的 atomic terminal 状态、全部 protocol retained network paths 读取和 original-Path 绑定的 fresh probe。

noq-proto 的九个运行文件为：`cid_queue.rs` +36/−3、`config/transport.rs` +16/−1、`connection/mod.rs` +199/−12、`connection/packet_builder.rs` +96/0、`connection/streams/state.rs` +18/0、`lib.rs` +2/0、新 `packet_witness.rs` +209/0、`shared.rs` +17/0、`transport_parameters.rs` +1/−2。当前差异不止其 [UPSTREAM.md](../../vendor/noq-proto/UPSTREAM.md) 开头的 packet-source accessor：

- [connection/mod.rs](../../vendor/noq-proto/src/connection/mod.rs) 的 dynamic application-data permit 排除未 qualified path 的普通数据 fallback；收到含普通 frame 的 authenticated packet 在 dedup/ACK 前整体丢弃，避免 ACK 已丢掉的 STREAM；验证与 ACK frame 仍可走。
- 同文件让 validated Backup path 优先返回自己的 pending ACK，避免仍为 Available 的黑洞主路径带走全部 ACK；普通调度使用 `validation_only` 与 `own_acks_only`，`path_status` 对已 abandoned path 也改为 ClosedPath。这是对旧消费者也可能生效的调度/状态语义变化。
- [cid_queue.rs](../../vendor/noq-proto/src/cid_queue.rs) 与 [transport_parameters.rs](../../vendor/noq-proto/src/transport_parameters.rs) 用新配置限制 CID 接收窗口及退休范围；默认保持五，不表示配置二时也无 wire-visible 差异。
- [packet_witness.rs](../../vendor/noq-proto/src/packet_witness.rs) 及 packet builder / streams / noq driver 中的观测点默认没有 observer，部分点受 `ring` 控制；模块是 `doc(hidden)` public，**不是 `cfg(test)` 或独立 test-only feature 隔离**。安装 observer 后会同步产生记录、hash/复制 bytes，并在实质 packet/driver 调用路径运行；边界为 16,384 records / 16 MiB，callback 必须不 I/O、不等待、不重入 QUIC。它不主动改 policy，但启用时会影响 CPU、allocation 与 lock hold time，应与 production 和实验性能结果区分。

全部新增 permit callback 的正确性依赖不重入 connection lock、及时反映原 session/association 的失效、metadata 与物理 descriptor 身份相符。gate 故意丢包会影响 loss recovery，验证 frame whitelist 与普通 frame 分类也影响互操作。这里没有证明死锁、误丢包或泄露已经发生；这是新进入底层收发路径的高影响不变量。

[netwatch/udp.rs](../../vendor/netwatch/src/udp.rs) 的新增 `poll_send_datagram` / `poll_send_datagram_from` 仅用于 raw traversal；Linux/Android 通过 per-datagram ancillary TTL/IP_PKTINFO 保留真实 source descriptor/IP，不改 socket default TTL，缺支持及其他平台走 explicit degraded path。旧 QUIC sender 不因这些新增 API 自动换实现。但 Linux 路径证据不能证明非 Linux TTL/source 行为等效；这类 platform branch 是维护面的新增。

[portmapper/lib.rs](../../vendor/portmapper/src/lib.rs) 新 `deactivate_and_wait` 管理 pending/active mapping cleanup acknowledgment；[nat_pmp.rs](../../vendor/portmapper/src/nat_pmp.rs) 在 allocation 已 acknowledged 后保有清理 guard，后续 public address 查询失败或取消也不丢失 ownership，completed Mapping 保存 TCP/UDP protocol 并用正确 protocol 删除。不是重写 NAT-PMP encoding，也不枚举删除别人的 gateway mapping。pending creation、lease renewal、shutdown timeout 和 abort 的相互关系仍属需要验证的资源生命周期面。

## 行为与性能风险的优先级

这里的优先级是**后续代码整理/核验建议**，不是新产品需求、决定或已创建的修复任务。没有用本轮静态阅读估算公网成功率、手机耗电或吞吐 SLA。

| 优先核对范围 | 原因与潜在影响 | 当前证据能说明什么 |
| --- | --- | --- |
| connection / path / socket ownership 与 gate 生命周期 | 原连接、原 association、UDP physical token、退休 path、取消、网络 epoch、descriptor final disposal 必须一致；错误可跨 session 误用资源或永远挡住数据 | 代码有明确 ownership/gate/epoch/guard；高风险不等于已证实 bug |
| noq-proto 普通 frame gate 与 Backup ACK 调度 | 修改 receive/encode/scheduling；含多个 frame 的 packet 整体拒绝、跨 path ACK 与 blocked primary 的交互具有较大影响 | [新增 multipath tests](../../vendor/noq-proto/src/tests/multipath.rs) 包括 fresh ACK、closed path、黑洞主路 backup probe；未因此证明全部 QUIC feature组合 |
| 全局 callback 与动态资源 policy | packet filter 在 connection lock 路径；custom 最终 send、lookup publish/resolve、relay reconnect 都加入许可判断 | 新检查位置可静态确认；callback cost、锁顺序与第三方实现是否正确声明 resource 未做全覆盖 |
| socket 池与 shutdown | 每轮最多 256、endpoint pool 最多 1,024 physical sockets；close 每个 custom handler 给一秒回收机会 | 有硬上限和释放 owner；FD/内存、并发轮与 shutdown 总延迟仍需量测，不把“每个有界”当“整个恒定一秒” |
| 原始消费者默认行为 | Portmapper Disabled；Backup ACK/path_status 变化；identity-only fallback 从即刻错误变为 lookup | 不使用根产品策略也可能受影响，是最需要保留原始调用者对照的区域 |
| p2panda auth Clone/回放 | root 每条 operation 克隆 full CRDT state；上游 validate 原已有 graph/state snapshots 与并发 predecessor rebuild | 随 retained history 变大增加时间与瞬时内存的机制可见；没有测增长曲线，不能报告数值 |
| 周期 path reevaluation 与 observation | actor interval clamp 250ms–30s；产品 Quality 默认一秒；同 socket observation/raw probe 有独立活动与 freshness | [既有路径维护说明](2026-10-02-connection-path-maintenance.md) 记录实现初值，明确不构成 throughput / power SLA |
| witness 观测与 test hooks | runtime-installed observer 会同步 hash/copy、增加 lock hold；test hooks 有 feature gating 与 dev feature 合并 | 两类应分别标记；禁用 observer 不等于所有观测代码从 binary 消失 |
| vendor 独立打包完整性 | noq 删去 examples/tests/bench，noq-proto 删 bench，但 normalized manifest 仍声明对应路径 | 路径不匹配是静态事实；不应由“作为 dependency 能编译”推断 vendored crate 的 all-targets/package 入口完整 |

对 performance 最可靠的现有结论是“有哪些额外工作进入了哪个热路径”，不是“目前已无性能问题”。noq predicate 的次数与成本、frame gate 的额外 parse、witness 启用后的 bytes/hash、path reevaluation 的 wake/probe、socket pool 数量，以及 CRDT state clone 的历史规模，各有不同计量单位，不宜合成单个“性能损耗百分比”。

## 既有验证的证据边界

本轮没有重跑已有验证。现有 [同 connection 维护说明](2026-10-02-connection-path-maintenance.md) 与 [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md) 已明确区分：controlled carrier/fault 下保持原 QUIC connection、某个 Linux NAT/socket/TTL 场景、浏览器编译或某个真实 RTC 通道，分别只覆盖其声明边界。单次或若干 PASS 不能推导全平台、所有 NAT、任意原始 iroh consumer 或全成员/同步流程无破坏。

当前源码的测试也不能仅按存在数量推导保证强度。noq-proto 新增 multipath tests 对 probe/ACK 和 reduced CID pool 有针对性；auth resolver 原测试没改，不能因此视为已覆盖 arbitrary application callback；p2panda-net 改 tuple 的测试是接线适配，不能当新 builder hook 及 identity-only lookup 的完整兼容矩阵。vendor 的删除测试/benchmark 应从“当前还能运行哪些原始 checks”角度单独确认。

## 建议的下一步整理顺序

1. 固定并保存可重复生成的“上游 → vendor”差异清单，修正过时的 delta 说明。保留源码/manifest/provenance/测试四种范围，避免上游升级时依据旧说明漏掉 gate、scheduler 或 cleanup。
2. 先把跨 iroh/noq/netwatch 的共同不变量写成简短机制图或接口契约：谁拥有 original connection、path、association、physical socket、mapping；每个阶段何时允许 frame，取消/关闭/epoch 怎样传播。以现有代码为对象，不借此新增产品功能。
3. 为明确 source-breaking 和 default-breaking 项列出实际调用者，分别确认要适配、要保留的原始 raw usage，以及有意停止支持的旧产品协议。不要重新引入 ADR 已取消的旧接线分支作为默认任务。
4. 按机制拆分候选补丁边界：observation/raw socket、physical pool、admission/data gate、path retirement/selection/probe、resource policy、mapping cleanup、CID 参数、witness。拆分目的是独立理解和回归，不意味着现在立即抽模块、改代码或决定 upstreaming。
5. 性能阅读与以后测量从具体热路径开始：禁用/启用 witness 对照、gate callback/额外 parse、并发 pool FD/内存与 shutdown、不同 history 大小下 auth clone/replay、维护 wake/probe 次数。先使用已有 raw receipts 的明确单位和场景，再判断哪些未知值得补实验。
6. 将 net backoff 的时间粒度输入契约和 vendor dangling target declarations 放进后续低成本核对清单；保持本轮整理范围，不顺带改运行代码。

本报告判断的是当前 fork 的实际规模、架构集中度、兼容边界与需核验风险。它不替代 ADR，也没有把研究建议升级成产品决定。
