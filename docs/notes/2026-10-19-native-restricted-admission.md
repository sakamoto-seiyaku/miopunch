# Native SDK 受限准入迁移

[M93](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/b6c70cbd-6ff1-4807-a8c6-d0e616b68423) 将 Native SDK、资源装配、ControlSession、CLI、probe 和 Native 测试迁入 M92 的独立受限接口。依据仍是 M91 与 ADR 0010、0011、0012；Browser 类型迁移和旧库接口恢复分别由 M94、M95 交付。[验收证据](../evidence/native-restricted-admission-2026-10-19/README.md)记录实际构建、失败修复、固定 counterpart 与资源关闭结果。

## 控制能力与既有协议

Native 端点绑定现在返回 `RestrictedEndpoint`。申请、precode、管理恢复、秘密轮换及控制历史使用控制入口；普通成员业务仍取得原来的 `Connection`。`ControlOwner` 保留控制会话的所有权，不提供普通连接转换。可复制的 `ControlHandle` 仅提供受限控制消息、只读连接信息和受约束的原会话建路操作；其 owner 被丢弃后，保留的 helper 不能继续使用会话。撤销使用不能升级的弱控制句柄。

M92 的默认受限协议增加库前缀和 Ready 帧，不能直接替换既有 Native 线协议。新增显式 `restricted_application` 选择，复用原来的 SCRAM、MUC1 offer/nonce、准入材料、控制历史和 RTC 消息；不会按 peer 或版本自动降级。应用通过定长、16/32 位长度前缀或 FIN 结束的完整消息交换操作私有 stream，调用者不能取得原始 SendStream/RecvStream。各既有协议仍使用自己的长度上限，读取 body 前检查长度。

应用交付协议由每次尝试独占的 `ControlConfirmation` 持有。缺少确认拒绝交付；取消、拒绝和丢弃 future 会释放其交换及候选资源。M92 默认的库帧模式保持原来的协议。

## 原会话建路与业务交付

Gate 和 coordinator 复用原来的认证、授权、协议协商和候选算法；共享模块通过 `AuthenticatedControl` 接收控制能力。Native adapter 仍使用原来的 TCP、物理 UDP、relay、RTC 和维修资源，不增加另一套网络或密码实现。控制-only 请求协商的数据权限保持为零。

成员交付先检查原连接的 TLS 身份、授权 revision 和 exporter binding，再在原会话上准备候选并取得 fresh ACK。`DataPathPermit` 绑定同一连接、PathId 和地址；旧普通 hook 只在这些条件成立后接收完整连接。确认原应用 nonce、保留真实路径、复查当前资格并退休控制交换之后，才向业务调用者返回原 `Connection`。`business_released` 将普通弱连接登记给后续维护；该登记不提前向应用暴露业务句柄。

已有 RTC 精确 seal 可以证明特定原连接及地址的资格，即使允许的 custom carrier 类型列表为空。actor 复用相同的当前资格和 packet permit 条件，仍检查实际资源策略；不会扩大为该 carrier 的所有地址。物理 UDP 描述符、LocalSocketId、真实源 IP、轮次、候选退休和最终关闭继续由原 socket pool、连接及路径 actor 管理。

调用者的绝对 deadline 贯穿建连、认证、控制、候选和交付；阶段切换不会重新计时。取消和对端关闭结束原尝试。成功交付后，setup deadline 不截断正常会话寿命。

## 包装端点与迁移边界

p2panda 的普通 `Builder::spawn`、endpoint getter 和 actor 生命周期保留原契约。新增 `spawn_restricted` 返回独立的受限 wrapper；私有 ownership 分支共用原 actor、address book、连接报告和 shutdown。普通消费者不必改成受限模式，受限 wrapper 不能借用或转换为普通端点。

Native `src/` 和 `tests/` 已移除 `before_data_release`、`AcceptData`、`AcceptRestricted` 和 `control_only_alpns` 消费。Browser 继续通过共享算法的普通接线运行，其类型迁移属于 M94。库中的旧枚举和过渡 hook 在 M95 恢复原契约时处理；本次 Native 迁移不宣称它们已经恢复。

关闭端点与释放端点 owner 是两个实际生命周期动作。夹具先关闭会话和端点，再丢弃保留的 endpoint/transport owner，最后等待驱动结束；没有将强制终止当作自然回收。完整公网与实体设备矩阵仍沿用已有独立验收范围。
