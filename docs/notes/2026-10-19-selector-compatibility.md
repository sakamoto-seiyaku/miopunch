# 保留 peer selector 契约并显式注册连接级选择

依据 MIOPUNCH-101、父规格 M91 与 ADR 0010／0011／0012，旧
`Builder::path_selector` 只按 peer 聚合调用一次；应用选择不再对每条连接
重复调用该用户 selector。新 `ConnectionPathSelector` 通过
`connection_path_selector` 显式注册，只接收当前原连接的 live qualified
paths。最后一次显式注册生效，产品的 `default_connection_path_selector`
保留任一显式注册。Native／Browser PathPolicy 共用原质量评分，改接新入口。

`current()` 仍返回真实地址；`is_current(candidate)` 包含实际 local socket
身份。Native 候选可观察 `local_socket_id()`，连接级 context 可观察当前
`connection_id()`。它们都是本地元数据，不改变地址语义或授予权限。
selector 保留上次结果也不能越过原连接当前的所有权、live path 和权限检查。
peer hint 仍需当前连接自己的 custom／association／physical socket 许可。
普通旧 selector 返回空结果时保持旧的“不创造选择”语义；受限连接或不能
持有 peer preference 的连接使用原 biased-RTT scorer 评估自己的获准路径。

首次产品改动前已冻结 `tests/path_selection_consumers/ordinary_stateful.rs`。
原版 published iroh 1.2.0 与当前 fork 使用同一份旧 API 源码，在隔离容器
中创建两条同 peer 原 QUIC 连接，实际交换 PING/PONG，记录回调与最终选择。
合格基准只绑定单个 IPv4 transport，避免把同一连接的 IPv6 候选当成第二条
连接。原版两次 peer 回调；未修复 fork 五次，包含额外逐连接回调；初次修复
恢复两次。三者均先正常关闭再断言结果，固定消费者未为修复更改。
首次默认 IPv6 测试判准不合格的记录保留，不当成产品失败或合格原版基准。

最终同消费者仍保持两次 peer 回调，两条真实原连接均选中正常 IPv4 路径。
新增连接级入口的三项真实双连接检查证明 callback 仅接收当前连接候选、
定时重评估仍按连接运行，最后注册及产品 default 的优先级均保留显式选择。
十二项 physical socket 测试、十五项统一连接测试、六项共同评分检查及四项
当前 Noq socket／generation／fresh ACK 边界检查全部通过，共四十项外层
测试；FD 限制夹具另启动一个内部子测试，不重复计入总数。

新增实际同址双 socket 用例观察到 `current()` 地址匹配两候选，而
`is_current` 仅匹配一个。切换后重播已退休的 PathSelection 不会接管当前
路径；随后同原 Connection 完成 fresh 数据交付、真实请求与 PONG，两个
物理描述符最终均释放。既有跨原连接拒绝、256 监听器取消、source IP pin、
丢弃与 network generation 失效用例保留并通过。Noq 生产代码未改动。

完整 Native／Browser SDK 使用本次冻结的实际编译输入和不可变产物：受限
入网请求约 681 ms 返回 AwaitingApproval，一次实际审批后 membership 与
admission secret 均 durable 保存，双方重开同一 persistent identity，成员
relay stream 返回 PONG。浏览器／WebSocket／Native actor／relay 均正常
退出，容器移除，生成的 TLS key 和私有 NSS fixture 随后清理。七组图在 SDK
执行前后保持相同 guard，运行期间没有重编译或扩展图；全根测试 typecheck
通过。二十六份既有原消费者源文件与原 lock／profile 未变；资源接线除
selector 注册方法外与 M100 相同。

首次完整统一连接测试有十四项通过，一项 WebRTC constructor 在
`network=none` 缺少组播路由时返回 ENODEV，尚未进入 selector。独立系统
socket 重现相同 mDNS join 错误；仅加入 `224.0.0.0/4 dev lo` 后恢复，
无需修改 interface flags、默认 mDNS 或产品代码。修正后的隔离环境没有
外部 interface 或 default route，完整十五项均通过。原始失败及系统证据
保留。新夹具前期误用 ControlSession.paths 和接入端开路的准备失败也保留，
分别改用既有 control_handle 观察与真正的拨号端，不添加旁路能力。

证据见 [selector 兼容性验收](../evidence/selector-compatibility-2026-10-19/README.md)。
这些受影响检查不扩展为全公网、实体设备或全部平台认证；对应矩阵仍属于
M69／M84 等既有独立验收范围。
