# MIOPUNCH-160 Windows 原生成员 TCP 首拨

Windows 产品资源配置默认绑定临时 IPv4 TCP 监听端口。显式禁用仍过滤安装，提供获准 TCP 观察资源时启用现有打洞流程。Windows 用 SO_REUSEADDR 将观察连接和互拨连接绑定到实际监听端口；SO_REUSEPORT 继续只用于已有 Linux／Android 平台。现有 `TcpTransport::listen` 普通调用方的默认参数保持由调用方选择。

## Windows 实机结果

真实 Windows 11 宿主通过 SSH 控制本次独立临时测试目录，执行本轮构建的 Windows 程序。受保护 MQTT 控制、TCP STUN、成员准入、路径验证和服务流均由正式 `NativeClient` 入口执行。

- 红测日志 `/tmp/miopunch-160-windows-default-red.log` 在未安装默认 TCP 时因 `no valid address available` 失败。正式客户端红测 `/tmp/miopunch-160-windows-native-red.log` 完成入网后发现监听缺失。
- 修复后 `/tmp/miopunch-160-windows-green.log` 两个测试通过。默认监听保留调用方 250 毫秒 QUIC idle 设置，并在原两秒期限内关闭。正式客户端回环实验关闭 UDP 数据和数据 relay，只提供 MQTT 信令，双方从各自默认通配监听端口观察，`MIOTCP01` 协调互拨并取得双端新鲜 ACK。提供端独立读出 `first request`，调用端读回 `first reply`。双端原连接标识、同一业务流、选中 TCP 与 STREAM 字节计数均记录在日志。
- 显式禁用组没有 TCP 监听，也没有观察请求，受限信令未交付业务。调用在原有界期限内结束。
- 红／绿 Windows 构建回执分别为 `/tmp/miopunch-160-windows-red-build/receipt.json` 与 `/tmp/miopunch-160-windows-green-build/receipt.json`。绿构建复用 527 项、重建 4 项，沿用共享 target、稳定 Windows 编译器镜像和构建锁。

## 尚待验收

本轮 Windows 正式业务结果使用回环路径。可控双 NAT 的 Windows 真实首拨尚未完成，MIOPUNCH-160 仍待这一验收。现有 159 双 NAT 驱动依赖 Linux namespace，已有 Windows 主机的 Sandbox 正在核实能否提供隔离 Windows NAT 节点。159 的 Linux 双 NAT 和既有普通消费者结果继续作为相应平台及共享实现的证据。

完整 root 运行的已有未全绿限制见 [159 证据](native-iroh-tcp-first-159.md)。本轮未重复与 Windows 配置变更无关的完整运行。
