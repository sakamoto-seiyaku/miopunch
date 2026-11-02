# 保留原版 early-data 接口，受限模式使用独立接口

2026-10-12，用户确认此接口方向：保留 iroh 原版 early-data 接口及其原有使用方式，新增受限准入模式通过独立接口表达权限与 early-data 限制；新增 API 参照既有 Builder、Incoming、Accepting 的命名、类型、返回结果、异步和所有权设计。当前 fork 把原入站 `into_0rtt` 改成 `Result`，影响了未使用新准入模式的原版调用者；通过接口隔离兼顾原版源码兼容和受限端点的安全要求。

受限接口必须在身份、准入及业务放行条件成立前阻止早期业务句柄，不能通过 panic、已关闭的句柄或要求调用者自行避免发送来假称保持正常兼容。此决定不重新设计 TLS early-data 算法，也不恢复 ADR 0009 已取消的旧 miopunch 应用连接协议。

2026-10-13，用户在 grill-with-docs 访谈中确认以独立受限端点类型作为设计起点：普通端点继续原 `Endpoint` 接口；显式启用受限准入时，通过另一种端点类型提供受限建连与接入接口，避免调用者绕回旧 early-data 入口取得早期业务句柄。两种类型共享底层实现。接受当前 demo 端点接线适配新类型的代价，以保留普通旧调用方的原用法。具体公开类型名尚未选定。

同日用户进一步确认连接交付方向：控制阶段使用独立受限句柄；身份、准入和数据路径检查通过后，向业务层返回原 `Connection`，使现有 stream 业务代码可复用。后续撤销与路径许可继续由底层检查。受限句柄的具体能力与放行入口尚待收敛。

同日用户确认将新增准入能力从旧 `EndpointHooks` 拆为独立接口。旧 hooks 保留原有握手检查及 `Accept`／`Reject` 结果；新增接口表达受限范围、允许的数据路径与放行条件，具体产品策略由 miopunch 提供。当前 demo 的准入 hooks 迁接新接口，旧调用者保留原枚举的穷尽匹配。

同日用户确认旧 `after_handshake` 在受限端点中的复用顺序：先完成新接口的准入与数据路径检查，再调用旧 hook，最后向业务层交付连接。旧 hook 返回 `Reject` 时仍关闭连接；普通端点保持原调用时机。这样旧 hook 接收完整 `Connection` 引用时，新的放行条件已经成立。交付仍受取消、撤销与资源许可约束。

用户同时要求具体设计符合既有原则与 API 设计思路。名称、Builder 接线、异步和所有权等细节沿用现有设计；整体规格需覆盖 Router／IntoFuture／accept_with 和 demo 的适配，不把逐个方法命名当成独立产品决策。

本轮仍是设计阶段，运行代码未修改。检查的推进方式、受限句柄的具体能力、Router／IntoFuture／accept_with 接线和 demo 迁移细节尚待收敛；不能只新增一个 fallible 方法就视为完成隔离。依据见 [iroh 兼容可行性评估](../notes/2026-10-12-iroh-upstream-interface-compatibility-feasibility.md) 与 [p2panda／demo 依赖评估](../notes/2026-10-12-p2panda-and-demo-compatibility-feasibility.md)。
