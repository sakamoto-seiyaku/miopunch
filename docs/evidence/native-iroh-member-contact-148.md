# MIOPUNCH-148 受限联系上的首条原生数据路径

正式原生业务可通过获准的成员转发、MQTT 或 IRC 建立受保护联系。两端在同一条 QUIC 连接上完成成员准入，复用 147 的原生 QNT 流程交换当前地址并验证实际 UDP 承载。业务交付要求当前成员资格、获准数据路径和该路径上的新鲜 ACK，受限载体在交付时退出连接。共享拨号沿用原绝对期限，并在实际启动受限联系时占用已有节点联系名额。明确拒绝准入结束本轮。

## 双端实测

- MQTT 冷启动只提供获准 MQTT 路由。红组到达真实 TCP 目标时仍使用专用业务连接。绿组从正式 `NativeClient` 转发入口进入，双端读回融合流和相互对应的实际 UDP 路径，目标核对 `contact-only request` 并返回 `qualified data reply`。另组在接收端撤销成员后，从仍未收到撤销信息的节点发起相同联系，收到 `PermissionDenied`，目标未收到连接。红组见 `/tmp/miopunch-148-mqtt-red/`，最终日志见 `/tmp/miopunch-148-candidate-build/native-service.log`。
- 成员转发通过真实第三成员的两条已准入转发链路。发起端没有数据 relay，接收端的自有数据 relay 在测试前后均在线。只提供成员转发地址，两端读回相同 TLS 导出 attempt、各自原连接、实际 UDP 路径及 ACK，完成请求和回复的双向字节核对。接收端禁用 UDP 的独立组在 8 秒原期限内返回 `NotConnected`，接收入口报告同一缺少数据路径的结果。最终回执见 `/tmp/miopunch-148-final-member/`。
- IRC 复用仓库已固定的真实 Ergo 镜像和配置。只提供获准 IRC 路由，首拨在 1.191 秒交付融合连接，双端的正式转发流分别读回实际 UDP 路径，目标和客户端核对完整请求与回复。接收端禁用 UDP 时，调用在约 1.26 秒返回缺少获准数据路径，目标未收到连接。日志见 `/tmp/miopunch-148-candidate-build/irc-fusion.log`。

同一 Ergo 上，融合入口和既有专用 IRC 入口在修复前都于首个合法的 512 字节 IRC 行后断开。`LinesCodec` 在找到 LF 前也计算 CR，因此原生读端允许扫描 511 字节的行体和 CR，完整线路上限仍由现有 `parse_line` 执行。线长诊断见 `/tmp/miopunch-148-irc-frame.log` 与 `/tmp/miopunch-148-irc-old-frame.log`。

## 兼容回归

正式 NativeClient 默认组 8 项、融合成员 8 项、普通端点 2 项和受限端点 19 项通过。文件专用 ALPN 和相同连接的大流、小请求对照包含在 NativeClient 组。构建回执和运行日志位于 `/tmp/miopunch-148-candidate-build/`。

定向回归沿用 147 的真实 relay 和原业务流。两端各自的策略只允许另一端已实际绑定的 UDP 地址和端口。修复前在 10 秒内无法取得两端新鲜 ACK。修复后双方验证了原连接上的实际对端元组，并在撤销 UDP 许可后经 relay 完成原流字节往返。本端广告取自实际绑定，远端许可约束实际远端路径和收发。红组见 `/tmp/miopunch-148-peer-policy-red/`，绿组和受限成员转发复测见 `/tmp/miopunch-148-peer-policy-green/`。

既有 IRC 无数据路径回归观察到实际 MIR2 片段，双端保持联系路由并拒绝业务，见 `/tmp/miopunch-148-irc-control-final/`。既有测试自定义 ALPN 的正组在分片修复后到达传统准入。该专用准入需要已有数据路径，组内只提供了 IRC，日志见 `/tmp/miopunch-148-candidate-build/irc-existing.log`。

浏览器 WASM 检查复现 147 的 `noq_proto` 引用及 `improve_existing` 调用参数两处错误，错误码、消息、路径和行号一致。回执见 `/tmp/miopunch-148-candidate-wasm/`。额外的物理 UDP 过滤场景在施加过滤前卡于私有命名空间的手动入网阶段。本票的无数据路径实测采用真实联系和接收端禁用 UDP。
