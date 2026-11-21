# MIOPUNCH-164 UDP 起步后的新 TCP 恢复

融合端点为 UDP 起步的原连接保留直接候选和获准 relay 的学习入口。是否仍有合格直连由既有路径 actor 的健康、许可、绑定和代际过滤结果通知候选 owner。旧路径留在列表、曾经收到过 ACK 都不能单独阻止下一轮。

原连接仍能传控制消息时，TCP 使用原有控制流、监听端口观察、双方 prime、新鲜探测及双端提交。所有数据路径都失去控制能力时，既有修复 owner 经获准的 MQTT 联系建立受保护控制。请求沿用原 TLS exporter 绑定，并显式请求 TCP 扩展。TCP 协调使用独立控制 exchange，最终验证和保留仍在同一原 QUIC 连接完成。既有单次 3 秒修复期限、节点联系额度、名额、权限和取消规则继续约束操作。

普通修复请求省略新增的 `traverse_tcp` 字段。对端明确不支持 TCP 时，调用方可在原剩余期限内沿用已有地址修复。首拨与修复的 TCP 物理连接和普通 iroh 消费者保持各自的契约。

## 正式入口

`scripts/check_member_tcp_recovery.py` 复用 161 的两个真实 NAT 网关和正式 NativeClient。首笔经获准信令找到对端并在 UDP 上读回。初期 TCP 观察点不可用，原流持续运行且没有保留 TCP 路径。观察资源可用后制造双向 UDP 数据故障，旧 UDP 路径仍列在原连接中。

- relay 模式允许数据 relay 备路，先在原流经 relay 读回新字节，再经新 TCP 继续。已通过日志 `/tmp/miopunch-164-cells-relay/`，首笔 TCP 数据约 1933 ms。
- signaling 模式始终阻断数据 relay，原连接的控制消息不能通过原路径送达。实际独立控制连接绑定该原连接后，双方观察、互拨、取得新鲜 ACK 并最终保留 TCP。已通过日志 `/tmp/miopunch-164-signal-first-tcp/`，首笔 TCP 数据约 960 ms，修复 owner 的结果为成功。

两组都核对原连接标识、原流、双方最终 TCP 提交和独立对端的业务字节。最终驱动还核对对端同流的 TCP STREAM 接收帧。

## 受影响检查

普通端点 2 项、受限端点 19 项、原生融合 TCP 6 项通过，日志 `/tmp/miopunch-164-contracts.log`。WASM 检查通过，回执 `/tmp/miopunch-164-wasm/receipt.json`。

完整根库单次结果为 63 通过、4 失败、4 忽略，日志 `/tmp/miopunch-164-full-lib.log`。原 active budget 和 first-contact 失败沿用 162、163 的前置产物对照证据。RTC 用例在隔离网络中报告 `No such device`。额外的旧 SDK `production_native_active_prepares_udp_on_the_original_member` 在 1070 行的路径 ID 断言失败，当前 exact 与现存 17:30 前置库测试产物在相同入口均复现 `[PathId(1)]` 与 `[PathId(0)]` 的差异，日志 `/tmp/miopunch-164-active-exact.log` 和 `/tmp/miopunch-164-active-pre164-test.log`。完整套件全绿尚未成立。

## 已观察限制

另一个实验先让 TCP 物理互拨在网络中失败，再立即复用同一四元组。故障前 socket 快照发现一端仍有无当前租约的 `SYN-SENT`，另一端随后留下同元组 `SYN-RECV`，新的 connect 报 `EADDRNOTAVAIL`。独立修复在原 3 秒期限内结束。证据 `/tmp/miopunch-164-signal-tuples/` 保留该失败，未将它解释为单纯等待期限不足。此次正式成功场景在初期让观察点不可用，未先创建这种失败 TCP 四元组。

本轮没有可用的现场公网双端 TCP 运行结果。既有 Windows 现场入口 `u22sz` 的跳板连接受阻，160 的独立实机双 NAT 验收仍待可用实验环境。当前重复成功证据来自受控双 NAT。
