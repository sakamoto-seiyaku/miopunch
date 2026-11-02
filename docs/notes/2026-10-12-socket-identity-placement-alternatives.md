# 物理 UDP socket 身份放置位置：新增候选与最小接口设计

2026-10-12。使用 research 与 codebase-design 技能，只读当前源码和当前 ADR；本轮没有修改运行源码、编译、执行测试或操作网络。以下是**候选设计和静态推断，不是产品决定，也不是 demo 已通过的证明**。此前[兼容可行性报告](2026-10-12-iroh-upstream-interface-compatibility-feasibility.md)的方案不是穷尽列表。

**内部 socket ID 可以移到别处。更完整的选择是：把“公开地址”与“内部路由身份”分开，而不是继续要求一个 `FourTuple.local_ip` 同时表示两者。** 当前 token 不只是 socket 查找键；它也替 Noq 区分路径、替发送报文携带来源、替限制回调识别物理路径。移动存储必须同时补齐这些作用。

## 用户纠正后的评估方向：底层原生 socket 模型

用户明确指出：“socket 身份应是底层已有概念，不该靠地址或额外适配绕行”。后面的候选表保留研究过程，但原先优先 B 的建议应按这个方向重新收敛：**先补足底层多 socket 模型，以现有 PathData 引用实际 socket；再讨论必要的接口兼容接线。** 本轮确认的是评估方向，尚未实施或将具体类型设计记录为 ADR。

用户随后再次明确目标：兼容以其他现有程序尽量不修改代码即可重新编译、正常运行为准；本项目允许新增 API，但继续按既有接口设计思路谨慎讨论。核对范围因此包含旧签名、旧必需 trait 方法、可穷尽 enum、公开 struct 构造，以及未选择新能力时的默认行为。并不要求旧地址接口自动表达新增多 socket 的所有细节；新能力可以通过明确的新入口使用，内部算法与资源对象仍共享。

新增接口应沿原 socket/路径机制扩展，保留通用资源与生命周期职责，NAT 调度、旁信道及 RTC 协商继续在既有 adapter/协调模块。当前 demo 已有 `open_data_path_from(connection, LocalSocketBinding, remote)`，应先评估保留该入口、在底层落实真实 socket 绑定；不能因为模型变化就要求所有业务调用者再学习一套 route 对象。[现有入口:1247](../../vendor/iroh/src/endpoint.rs#L1247)、[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)

最先需要讨论的旧契约是 `rebind`：它原来替换当前 abstract socket 并更新全部连接的 sender；旧单 socket 使用必须保留这项行为。新增显式绑定路径如何响应 rebind，以及未完成发送如何保存原 socket 寿命，应在确定接口前讲清楚。注册到 Endpoint、获准用于某条连接、验证完成、长期 retain 是不同状态；ID 有效不自动授予数据路径权限。[rebind:280](../../vendor/noq/src/endpoint.rs#L280)、[winner lease:684](../../vendor/iroh/src/socket.rs#L684)

当前 Noq Endpoint 的 State 保存一个当前 abstract socket、一个 sender 和迁移过渡的 prev_socket；每条连接从该 endpoint socket 创建 sender。这个 abstract socket 在 iroh 内部可以聚合多个物理 transport，但 Noq 不具有显式物理 socket 身份。FourTuple 注释也明确假定发送只能选择 source IP，不能选择 local port。当前 pool 的 token 正在补这个模型缺口。[Endpoint State:567](../../vendor/noq/src/endpoint.rs#L567)、[connection sender:258](../../vendor/noq/src/endpoint.rs#L258)、[FourTuple 假设:373](../../vendor/noq-proto/src/lib.rs#L373)

建议先评估如下原生关系，名称仅作示意：

```text
Endpoint 管理本地 socket 集合
Connection M 管理已有 paths
每个 PathData = 真实 network_path + LocalSocketId + 已有 generation
```

具体描述符和既有 sender 由 I/O 层管理，Sans-I/O proto 仅引用不透明的 LocalSocketId。可以将现有 pool 接成统一 socket 集合，避免重复增加一套物理 owner。原 socket 的 round lease、raw traversal、pin、retain 和关闭仍沿同一个对象运行。socket 是资源身份，local IP/port 是通信属性，PathId 是 QUIC 路径标识；三者职责应保持清楚。

正常发送已经在 `build_transmit` 中选定 PathData，可以直接取得该路径绑定的 socket，再由 driver 使用它的 sender。接收 socket 自己提供来源身份。现有 PathData、PathState 的 current/previous 和 generation 可以继续承载迁移状态；没有必要先引入通用 PathKey 框架或把增强 UDP 变为另一种 transport。[发送位置:1290](../../vendor/noq-proto/src/connection/mod.rs#L1290)、[PathData:141](../../vendor/noq-proto/src/connection/paths.rs#L141)、[generation:280](../../vendor/noq-proto/src/connection/paths.rs#L280)

仍有必要的底层信息传递：发送结果及 buffered transmit 保存当时选中的 socket ID，不能稍后只凭 PathId 重查，因为该 PathId 可能已经迁移；尚无 PathData 的 Initial/Retry/refuse/reset 回复从真实接收 socket 保存来源，off-path 回复从原 challenge 记录取得来源。这些可作为原生收发接口的一部分。公开 Transmit 的 struct literal 兼容问题仍存在，应保留旧 struct，以内部发送结果或新增带 socket 的入口处理，不静默丢 ID。[buffered send:1720](../../vendor/noq/src/connection.rs#L1720)、[stateless response:150](../../vendor/noq-proto/src/endpoint.rs#L150)

旧单 socket 创建/连接方法可自动使用默认 socket；新增注册 socket、在指定 socket 上开路径的能力。主要修改面因此集中到 Endpoint 的 socket 假设、PathData、收发及缓存寿命。具体公开方法、错误类型和 fd owner 分工还须逐项核对，不能从模型合理就推断未知旧程序或 demo 已通过。

一个相关的静态检查点：当前 fresh probe 保存 PathId + watermark，ACK 判断没有同时比较 PathData generation。若 socket/路径 generation 改变，物理路径证明须失效或核对对应 generation；这是该设计需要处理的寿命条件，本轮没有复现产品故障。[probe:2835](../../vendor/noq-proto/src/connection/mod.rs#L2835)

## 当前源码为何使用 token

- `LocalSocketId` 是不复用的诊断身份；pool 把它编码为 `fd6d:696f:3631:0:…`。接收保存真实 IP、port 到 `RecvInfo.ip_binding`，随后把 `RecvMeta.dst_ip` 替换为 token。[pool.rs:27](../../vendor/iroh/src/socket/transports/ip/pool.rs#L27)、[pool.rs:173](../../vendor/iroh/src/socket/transports/ip/pool.rs#L173)
- Noq 接收入口只用 `meta.addr`、`meta.dst_ip` 构造 `FourTuple`。公开 `FourTuple` 只有 remote socket address 与 local IP，`Hash/Eq` 包含这两个字段；它明确假设发送时只能选择 source IP，不能选择 source port。[endpoint.rs:946](../../vendor/noq/src/endpoint.rs#L946)、[lib.rs:364](../../vendor/noq-proto/src/lib.rs#L364)
- Noq 的路径验证、路径迁移、pending PATH_RESPONSE 都保存 `FourTuple`；并非只有 iroh 的 pool 字典需要区分 socket。[connection/mod.rs:925](../../vendor/noq-proto/src/connection/mod.rs#L925)、[paths.rs:853](../../vendor/noq-proto/src/connection/paths.rs#L853)
- 发出报文只携带 `destination`、`src_ip` 等，没有物理 socket ID 或 PathId。iroh 从 `src_ip` 恢复 token，交给 pool，再用 pinned 真实 IP 发出，token 不进入内核 source address。[lib.rs:335](../../vendor/noq-proto/src/lib.rs#L335)、[transports.rs:1086](../../vendor/iroh/src/socket/transports.rs#L1086)、[pool.rs:149](../../vendor/iroh/src/socket/transports/ip/pool.rs#L149)
- 现有连接限制在 QUIC connection lock 内检查 incoming datagram 和 buffered transmit 的 tuple；改变载体后必须让这些检查获得同样准确的身份。[connection.rs:349](../../vendor/noq/src/connection.rs#L349)、[connection.rs:1742](../../vendor/noq/src/connection.rs#L1742)、[connection.rs:1832](../../vendor/noq/src/connection.rs#L1832)

当前 pool 实际绑定 `port: 0`，活跃 socket 通常具有不同本地端口。但 Noq 的 FourTuple 不保存 local port，因此同一 local IP、同一 remote 即使本地端口不同也会失去区分。端口关闭重绑后的数值复用也不能代表原 descriptor 的寿命。**真实地址不是物理 socket 身份**；同 IP/port 的不同 socket 或同端口新旧 generation 更不能仅由地址字典解决。[pool.rs:255](../../vendor/iroh/src/socket/transports/ip/pool.rs#L255)

## 候选总览

| 候选 | ID 实际存放处 | 可保留的公开契约 | 主要代价与限制 |
| --- | --- | --- | --- |
| A. FourTuple 增 private binding 字段 | Noq FourTuple 内部 | 旧签名、旧构造、Copy、真实 local_ip getter | 扩展 tuple 的 Eq/Hash 不再只是地址；旧程序重构 tuple 丢 ID。收发仍需新增 metadata 通道。 |
| B. private PathKey + 独立收发 context | Noq 私有 path key 与 route metadata | FourTuple 的地址意义、Eq/Hash、旧 public struct/enum | 改动深入 Noq driver/proto，但身份作用集中、语义最清楚。优先研究。 |
| C. connection-local registry | `(connection, PathId) → binding`，同时给 datagram/transmit 携带 context | 公开地址与诊断方法 | PathId 还不存在的初始/迁移/off-path 报文仍需 context；只加 registry 不足。可作为 B 的实现。 |
| D. iroh-only side table | iroh registry，Noq 完全看真实旧 tuple | 若只读 observer，公开契约最好保留 | Noq 内部同地址路径仍合并，发送又没有 PathId；一般情形不足。只有明确限制并发/同tuple路径时才成立。 |
| E. 复用 mapped remote 地址架构 | iroh 私有 `alias → real remote + binding` | 无需 Noq runtime 新字段；iroh facade 可还原地址 | 仍然编码身份到地址，移到 remote；映射必须覆盖所有 ingress/egress、地址分类、限制、退休与网络变化。是值得评估的低层改动较少路线。 |
| F. 原生 UDP 作为私有 custom carrier | iroh custom local/remote route context | 用既有 CustomTransport seam，可不改 Noq | UDP 被公开看作 Custom，现有 direct/IP 分类、策略与 demo 条件需调整；若要保留 Ip view 仍需 facade。 |
| G. token 保留内部、公开 facade 还原 | Noq 内继续 token，iroh 私有 RoutedPath 与公开 address view | 改动小，可先恢复旧 local_addr 的真实 IP | token 技债仍在；所有外部回调也必须投影，selector 精确身份需另存。适合过渡，不能称为内部已拆分。 |

上述 A/B/C 与 E/F/G 是不同 seam placement。把 ID 从一个 hashmap 移到另一个 hashmap，仍不给 Noq 收发传递上下文，不构成完整候选。

## 最小接口设计：B 的地址与身份分离

内部示意，名称未定：

```rust
// FourTuple 继续只有原地址意义和原 Hash/Eq。
struct PathKey {
    addresses: FourTuple,
    binding: Option<BindingKey>,
}
struct BindingKey { endpoint_scope: u64, id: u64, generation: u64 }
struct RoutedTransmit {
    transmit: Transmit, // 原公开 struct 不加字段
    route: RouteContext,
}
// RecvMeta 也保留；另一个按 batch index 配对的 context 数组。
```

业务调用方只需在既有 round lease / winner handoff 上使用一至三个入口：

```rust
let binding = round.retain_winner(winner.id, winner.local_addr).await?;
let path = connection.open_bound_path(winner.remote, &binding).await?;
let id = path.local_socket_id();
```

这里展示 interface 的范围，不表示应当公开新增这三个同名方法；当前 retain_winner、local_socket_id 已存在，最小新入口可能只剩绑定路径的创建。普通 caller 继续既有 `open_path(FourTuple)`，其中 binding 为 None。binding lease 必须属于原 Connection；跨连接、关闭、未 pin、过期 generation 都返回错误，不能默默从 primary 发送。实现隐藏 pool、真实 port/source IP、RouteContext、CID 关联、buffered transmit、PATH_RESPONSE 与取消清理，形成一个深 module。

**新增收发通道不能只覆盖业务 data packet。** Initial、Retry、refuse、stateless reset、path challenge/response、off-path response 和网络变化后的恢复也必须保留相应 routing context。Endpoint 处理 datagram 时尚未有 Connection，因此最早 context 的归属是 endpoint/socket；connection-local registry 是后续关联，而非最早入口。[endpoint.rs:153](../../vendor/noq-proto/src/endpoint.rs#L153)、[endpoint.rs:804](../../vendor/noq-proto/src/endpoint.rs#L804)、[connection/mod.rs:2178](../../vendor/noq-proto/src/connection/mod.rs#L2178)

Noq `AsyncUdpSocket` 和 `UdpSender` 是现有 transport seam，适合提供带默认实现的新 routed 方法或平行扩展入口。旧 adapter 默认接收 binding=None、发送无绑定报文仍走旧方法；有绑定而 adapter 不支持时明确失败。不要新增必需 trait 方法强迫全部实现改动，也不要让默认实现忽略 binding。[runtime/mod.rs:44](../../vendor/noq/src/runtime/mod.rs#L44)、[runtime/mod.rs:86](../../vendor/noq/src/runtime/mod.rs#L86)

最小 I/O 扩展示意是两个带默认实现的方法：`poll_recv_routed(..., contexts)` 和 `poll_send_routed(transmit, context, ...)`。接收默认清空 sidecar 为旧模式后委托原方法，发送默认只对无绑定模式委托原方法；注册绑定能力时先确认 adapter 支持，避免首次发送才遇到永久 I/O 错误。GRO 的每个 metadata entry 对应一个 context，GSO batch 不混 binding；已缓冲的 bytes/transmit/context 一起保存，重试复查寿命，不从后来选中的路径反查原包该用哪个 socket。[receive:946](../../vendor/noq/src/endpoint.rs#L946)、[buffered send:1720](../../vendor/noq/src/connection.rs#L1720)

保留旧 proto `poll_transmit` 也有一个必须解决的契约：不能将新 `RoutedTransmit` 简单剥掉 context 后交给旧入口发送。一个候选是新增 route-aware proto facade，独占带绑定状态的驱动所有权，不暴露可由旧 polling 入口驱动的裸状态；旧类型继续走无绑定模式，内部共用 QUIC 算法。另一种保护方式也须显式拒绝能力丢失，不能静默退回 primary。同理，旧 `DatagramEvent` 是可穷尽 enum，应另设带 route 的新返回类型，而非往旧 enum 塞 variant。[poll_transmit:1030](../../vendor/noq-proto/src/connection/mod.rs#L1030)、[DatagramEvent:1237](../../vendor/noq-proto/src/endpoint.rs#L1237)

**`Transmit` 的 public fields 是具体兼容陷阱。** 无论加 public 字段还是 private 字段都会使已有外部 struct literal 无法原样构造。应新增 wrapper/context 返回或内部 routed polling 入口，保留原 `Transmit`。旧 public iroh `FourTuple::Ip { remote, local }` 是 enum variant，也不应直接加字段或新增一个必须处理的 variant。[noq-proto lib.rs:335](../../vendor/noq-proto/src/lib.rs#L335)、[iroh transports.rs:875](../../vendor/iroh/src/socket/transports.rs#L875)

依赖策略：path registry、Eq/Hash、寿命与 scope 检查是 in-process；物理 UDP adapter 是 local-substitutable，但只有真实 OS socket 能验证 pktinfo/source IP/port 和收包归属。Noq 不应依赖 iroh 的 LocalSocketId 类型，而接收库自身的 opaque route key；iroh 负责把该 key 映射到 pool descriptor。未来测试应通过 binding handoff 与 path interface 验证选中的 descriptor、fresh ACK、退休后的拒绝；不把内部 hashmap 当外部测试 interface。本轮没有执行这些验证。

## A 可以少改类型，但 Eq/Hash 不是免费兼容

Noq 的 FourTuple 字段私有，可以加 `binding: Option<opaque key>`、新增构造方法，保留旧 `new`/`from_remote`，默认 None。若 key 是 Copy 小值，原 Copy 性质也能保留；源码兼容成本相对低。[lib.rs:372](../../vendor/noq-proto/src/lib.rs#L372)

但必须准确描述其行为：

```rust
let address_copy = FourTuple::new(path.remote(), path.local_ip());
// 对扩展绑定路径，address_copy != path，若 private binding 参与 Eq/Hash。
```

旧 caller 未用绑定路径时可以保持原 equality/default；扩展 caller 或观察扩展路径的旧代码不能再假设“两地址相同即相等”。如果故意令隐藏字段不参与 Eq/Hash，同地址 socket 的身份又失效，等于绕回原问题。B 用 private PathKey 把新 identity equality 放在内部、把旧 FourTuple equality 留为地址 equality，避免这个语义成本。B 变更多于 A，不能在未评估路径匹配覆盖率前宣称是小修补。

## selector 可保留更多现有 interface

不必因为身份分离就立即新增一整套 selector trait。现有 `PathSelectionData` 的私有 source 已存 `(conn, path_id)`，`PathSelection` 字段也私有；`PathSelection::set(&data)` 目前仅 clone tuple。可以让 set 内部捕获 exact path handle/key，旧 `select` 签名和 `network_path()` 返回的真实地址仍保持。[remote_state.rs:2248](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2248)、[remote_state.rs:2336](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2336)

这样，即使两个候选公开的地址相同，caller 将某个候选传给 set 仍能精确选该路径。新 caller 可通过新增 `binding_id()` 或 `current_path()` accessor 获取身份；旧 caller 若自行按 FourTuple 去重，仍只得到地址语义，无法自动获得扩展区分能力。`ctx.current()` 的原地址视图不能完整表示 exact current，需如实保留这个限制，不能靠隐藏改 Eq/Hash 偷换契约。

内部 selection 应保存可校验的连接/路径身份，而非为选择结果长期持有一个强 Connection 引用；应用选择时复查 owner、generation 和 path liveness。peer hint 的公开地址偏好也不能直接变成另一连接的 binding capability。因此这项局部优化保留 selector 的公开签名，却仍需要调整内部 apply/retain 的精确匹配；不是只改 `set` 一行即可完成。

普通全局 peer hint selector 与原 Connection 的绑定能力仍须分别管理。一个 Connection 的 binding 不应被复制到另一个 Connection 的 peer preference 中。当前 owns_udp_binding 正在执行这项限制；B/C 需要移到私有 key/registry 而非删除它。[remote_state.rs:2058](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2058)

## CID、fresh ACK 与退休不会自动解决 placement

非空 local CID 的当前 endpoint index 能定位 `(ConnectionHandle, PathId)`；因此 CID 可辅助已建路径的关联。但 Initial、无 CID 路由、reset 与 off-path 处理并不总有这个信息。当前 zero-CID incoming index 用 FourTuple，outgoing index 甚至只用 remote；不能假定在所有配置下仅 CID 足以选择物理 socket。把 CID 当唯一内部 route key 也要处理 rotation 与退休，不能只解析报文头反查一张 map。[endpoint.rs:1030](../../vendor/noq-proto/src/endpoint.rs#L1030)、[endpoint.rs:1143](../../vendor/noq-proto/src/endpoint.rs#L1143)

Fresh ACK probe 已保存原 Connection/path handle 与 watermark；重构后应继续证明该 PathKey 的新 traffic 得到认证 ACK，而不是同地址其它 socket 的旧 ACK。身份 registry 的 stale entry 不得被新 socket 复用；网络变化、round drop、path close、connection close 都须清理并使已缓冲收发失效。[path.rs:279](../../vendor/noq/src/path.rs#L279)、[pool.rs:330](../../vendor/iroh/src/socket/transports/ip/pool.rs#L330)、[remote_state.rs:2040](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2040)

这里要区分 ACK 确认的发送路径与 ACK 自身的接收路径：确认的是在目标 binding/路径上新发出的 packet；合法 ACK 经另一条现存路径返回，并不因此自动无效。route/epoch 改变后旧 proof 不应直接用于新 socket，不能把这个条件误写成所有 ACK 必须从同 socket 返回。PATH_RESPONSE 也要继续按原 challenge 的路径语义核对。[challenge validation:503](../../vendor/noq-proto/src/connection/paths.rs#L503)

此处 generation/retirement epoch 是候选内部手段，不是建议新增产品协议字段。当前 ID 不复用是一种已有抵抗 stale association 的手段；保留它比只用 fd/port 更简单。

## 对其他位置的判断

**E 值得独立试算。** iroh 已经有 EndpointId、Relay、Custom 的映射 IPv6 地址和私有 `MultipathMappedAddr`，不是完全新造 overlay。给 `(real remote, binding key)` 分配私有 remote alias，接收改 `meta.addr` 为 alias、`dst_ip` 保持真实 IP，发送从 alias 恢复 descriptor，可以让 Noq 原 Eq/Hash 与 Transmit 自然携带区分信息。[mapped_addrs.rs:84](../../vendor/iroh/src/socket/mapped_addrs.rs#L84)、[remote_map.rs:76](../../vendor/iroh/src/socket/remote_map.rs#L76)、[transports.rs:1344](../../vendor/iroh/src/socket/transports.rs#L1344)

它把变动集中在 iroh，可能比 B 的 Noq driver 改动少，但没有彻底消除 token：只是将私有名字放到 remote address namespace。需要新增 private mapped variant/registry，并核对所有 mapped 地址解析、IP/Custom 分类、scope/IPv4-mapped normalization、network-change hint、限制回调、stateless response。不能直接复用 public CustomAddr 暴露成 Custom 后声称原 IP 语义不变。此方案 feasibility 仍是静态推断。

还有具体的限制：Noq 内置 NAT traversal 把收到的 `network_path.remote` 与原先保存的真实候选 IP/port 比较；改成 alias 后须提供翻译或隔离该显式扩展路径的内置探测。Retry token 也直接绑定 `incoming.network_path.remote`，reset 索引还涉及 remote。若先限定 alias 只用于已认证原 M 的显式路径改善，可以缩小初始握手的改动范围，但仍须检查未知 Initial、off-path 回复和旧探测状态，不能宣称“只加一张 map，Noq 的所有原功能就不受影响”。[n0_nat_traversal.rs:614](../../vendor/noq-proto/src/n0_nat_traversal.rs#L614)、[n0_nat_traversal.rs:857](../../vendor/noq-proto/src/n0_nat_traversal.rs#L857)、[Retry:777](../../vendor/noq-proto/src/endpoint.rs#L777)、[index:1143](../../vendor/noq-proto/src/endpoint.rs#L1143)

**F 是 adapter 方案。** enhanced UDP 经私有 CustomEndpoint/CustomSender 携带 socket route，不要求 Noq 认识 pool。既有 custom 本身也是通过 mapped address 进入 Noq，因此它更容易复用现有载体机制。不过当前 demo 用 `path.is_ip()`、remote `TransportAddr::Ip` 和 `local_socket_id()` 检验胜者；直接 custom 化会改变这些行为，并影响 direct preference 与 PathStats 分类。要保留 demo 行为需真实 IP facade 和 metadata accessors；或者明确决定改变产品接线，但本轮不作此决定。[nat_punch.rs:829](../../src/nat_punch.rs#L829)、[path_watcher.rs:489](../../vendor/iroh/src/socket/remote_map/remote_state/path_watcher.rs#L489)

**G 是最小过渡方案。** 保留 token 于 Noq 内，iroh Path/selector/callback 向外返回真实 IP，将 binding id 存在私有 PathData；PathSelection 仍按 exact handle。它能先消除公开 local_addr 的 token 暴露，成本较低，但每次投影都必须有 connection/route context；按真实地址逆向重构内部 token不应是隐式允许的通道。不是“ID 已离开 local_ip”的最终方案。

**D 单独使用不够。** 当前 `RecvInfo.ip_binding` 已是 side metadata：iroh 知道实际 binding，Noq 仍只看经过转换后的 RecvMeta。若删除 token而不增加载体，信息在进入 Noq 的瞬间消失；同地址 path matching、validation 与发送选择无法再得到它。这是当前源码的限制，不是说 side table 原理上不可用；B/C 的 routed context 正是让 side table 成为完整解法的补充。

## 推荐研究顺序与 demo 约束

若用户的首要目标是**旧公开地址语义严格保留并清除 local_ip 编码**，优先 B：private PathKey + routed context；C 可作为其 registry 实现，selector 尽量复用现有 set interface。若首要目标是**减少 Noq fork 改动**，应把 E 与 F 单独设计比较；如果需要尽快清除 iroh 外部 token view，G 有过渡价值。A 是源码破坏较少的候选，但隐藏 Eq/Hash 变化需明确接受，不能称为严格原语义兼容。

所有候选继续保留：原 QUIC session/stream、真实 winner 的物理 descriptor 与 pinned source IP、原 Connection 所有权、绑定 ID 不复用、fresh authenticated ACK、取消/权限撤销/资源退休后不得再发，以及普通 caller 的旧构造与默认行为。demo 当前胜者 handoff 明确在 round 结束后把 winner socket 交给原 Connection，并在业务提交后再次检查 selected path 的 socket ID；这些约束不能因为换存储位置而省略。[nat_punch.rs:1086](../../src/nat_punch.rs#L1086)、[nat_punch.rs:829](../../src/nat_punch.rs#L829)、[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)

范围可以只限物理 UDP binding：既有 relay/custom 的映射机制不必一起重写，TLS、拥塞算法也不因这项身份拆分而需要重新实现。所谓成本应按收发 context、私有路径状态、资格/退休与公开投影的具体改动面判断。少了地址编码不代表没有 CPU、metadata 或 registry 成本；本轮没有测量性能。

下一步若获准实现，应先选择一种 route representation，再验证同 remote/local IP 的两个 socket、关闭重绑、buffered send 与退休竞争、迁移 PATH_RESPONSE、stateless 拒绝响应、普通 selector 与扩展 selector、fresh ACK 不被旧路径满足，最后执行现有相关 demo。这里仅列出验证范围，没有运行它们，也没有把实现候选升级为已接受架构。
