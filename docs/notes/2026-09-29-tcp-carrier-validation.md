# MIOPUNCH-18 · 真实 QUIC over TCP 载体验证

日期：2026-09-29。范围是两个明确可达的原生节点及单跳 TCP；不是 TCP 打洞、端口映射、消息旁信道或生产资源策略。代码入口为 [`src/tcp_transport.rs`](../../src/tcp_transport.rs)、[`src/main.rs`](../../src/main.rs) 和公开进程测试 [`tests/native_probe.rs`](../../tests/native_probe.rs)。

## 运行与结果

工具链：Rust 1.98.1；锁定 iroh 1.2.0、noq-proto/noq-udp 1.3.0、p2panda-net 0.7.1。`quic_config()` 沿用当前现成 BBRv3。服务端与客户端分别保存自己的 Ed25519 身份；内层连接仍是 iroh/noq 的 QUIC、TLS 和普通 `Connection::open_bi`/`accept_bi`。

在两个终端运行；先从服务端 `READY` 取身份与 TCP 地址，再填入客户端命令：

```sh
rtk cargo run --locked -- serve tcp /tmp/miopunch-tcp-server
rtk cargo run --locked -- send tcp /tmp/miopunch-tcp-client TARGET_ID TARGET_TCP_ADDR
```

客户端输出 `PONG`；双方退出时在 stderr 输出 `TCP_TRAFFIC`。本机集成检查曾观察到客户端双向各 14 个自定义载体包，发出 7161、收到 8044 个包体字节；这些数值取决于握手/关闭时序，不是固定断言。服务端公开 `READY` 和客户端公开 `PONG` 是可重跑入口；测试另检查连接公开的选中路径为 `Custom`。`Minimal` 不安装地址查询或 relay，`clear_ip_transports()` 与 `clear_relay_transports()` 明确移除 iroh 的 UDP 与 relay socket，目标只提供 TCP custom 地址；`TcpOnly` 选择 custom 路径承载应用流量。

```sh
rtk proxy cargo test --locked --test native_probe -- --test-threads=1
```

此测试在真实本地 TCP socket 上启动独立进程，验证握手/双向 stream、错误目标身份、截断帧、超长帧、写半关闭和突然断流。错误身份没有 `PONG`；故障帧计入 `failed`，服务端仍能为下一条合法连接回 `PONG`。截断帧及半关闭测试观察 `early eof`，超长帧观察 `invalid TCP frame length`。载体接口的另两项真实 socket 测试确认 Endpoint 丢弃后连接取消、入站队列不被消费时约 5 秒有界关闭。没有用模拟 TLS 或模拟 QUIC 成功替代握手检查。

## 报文、背压与生命周期

每条 TCP 连接先交换 6 字节 IPv4 监听地址前言；随后以 2 字节大端长度加完整 QUIC 数据报连续封帧。前言仅是回程路由提示，不证明节点身份；身份由内层 QUIC/TLS 验证。`read_exact` 处理任意 TCP 拆包与粘包。帧长限定为 1–2048 字节，在分配帧体前检查；收、发队列各为 32 项，满队列时发送端经 `poll_ready` 背压、接收端暂停 TCP 读取。并发入站及目标通道各有 64 个上限。单帧接收体、写入及接收端背压各超时 5 秒，帧头空闲 15 秒，TCP 连接和前言各超时 5 秒；进程级 connect、stream 往返和服务端等待也有超时。Endpoint 关闭会通知活跃 TCP 任务取消；读/写结束或取消均清除对应通道并记录结果。`TCP_TRAFFIC` 报告包数、包体字节数、TCP 帧边界正常/失败/取消关闭数、双向队列待处理包数及运行中峰值、容量配置；待处理数也包含正在读写的单个包，不计 TCP/IP 或 TLS 开销，关闭时的统计是该时刻快照。帧边界的正常 EOF 只表示 TCP 读方向完整结束，不证明 QUIC 业务成功；成功以 `PONG` 和内层连接检查为准。

这里验证的是单条明确地址 TCP 载体可承载真实 iroh 连接。`TcpOnly` 是这张验证票的应用路径选择器，未实现独立的控制/业务载体资格体系；后续旁信道隔离不能直接把本实验当成生产策略。`0.0.0.0` 监听时 `READY` 会显示监听通配地址；远端应使用服务端实际可达的 IPv4 和同一端口。仓库中未加入 IPv6、自动发现、重连、TCP 打洞或端口映射。
