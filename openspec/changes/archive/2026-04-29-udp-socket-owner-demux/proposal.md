## Why

当前主线产品路径在 UDP 场景存在结构性不完备：`connectivity`（direct handshake / punching）与 `dataplane`（QUIC / KCP session）都在直接读写同一条 `*net.UDPConn`，缺少“UDP socket owner / demux”的单点收包边界，导致：

- QUIC 一旦接管 `net.PacketConn`，对同一个 conn 再做 `ReadFrom/WriteTo` 属于未定义行为，punching/direct handshake 的收包会与 QUIC 抢占并互相干扰。
- KCP 当前路径依赖 `conv=1` 的过滤来避免把 punching 误判为 KCP session，本质是脆弱的 heuristic；同时也没有明确保证“同端口并发多 peer session accept”。
- 在 mesh / 多 peer 的主线路径里，同一被访问端必须能在同一 UDP 端口上并行服务多个 peer transport session；当前缺少能支撑该语义的 socket ownership 设计。

本 change 需要把 “UDP 同端口复用 + 单点收包分流 + 多 peer 并发 accept” 收敛成可验证的 spec 合约与后续实现任务，作为进入更复杂主线网络测试（MNT-02/后续）前的根因修复与设计固化。

## What Changes

- 引入 UDP punching 报文的明文 tag 前缀（固定 5B：`00 4D 50 00 01`），用于：
  - 明确区分 punching vs QUIC/KCP 数据面报文；
  - 使 quic-go `Transport.ReadNonQUICPacket` 能稳定识别 punching（首字节前两位为 0）。
- QUIC：切换到 `quic.Transport` 作为 UDP socket owner：
  - QUIC 连接收包由 Transport 统一负责；
  - punching/direct handshake 收包从 `ReadNonQUICPacket` 读取；
  - punching 发包使用 Transport 的写包路径（避免对底层 `PacketConn` 再调用 `WriteTo`）。
- KCP：引入 UDP socket owner/demux 的实现边界：
  - 单点收包循环读取 UDP；
  - 按 tag 将 punching 报文分流到 punching 事务；
  - 将非 punching 报文分流给 KCP（以 `net.PacketConn` 视图喂给 kcp-go），从而支持同端口并发多会话 accept。
- 明确约束：对 UDP 路径，traversal（gather/attempt/punching）与 dataplane 必须共享同一 UDP socket/端口映射；不得通过拆分 socket 规避收包冲突。
- 保持对外选择面不变：`--data-proto kcp|quic` 与 `--quic-cc bbr|brutal` 的语义不变，仅修复其在同端口复用与多 peer 并发下的正确性基础。

## Capabilities

### New Capabilities
- `miopunch-udp-socket-owner-demux`: 约束并实现 UDP 同端口的 socket owner / 报文分流边界（punching 与 QUIC/KCP 共存，多 peer 并发可成立）。

### Modified Capabilities
- `miopunch-dataplane`: 追加并固化 UDP 路径的 “socket owner / demux” 约束与同端口多 peer 并发 accept 的 requirement（覆盖 QUIC/KCP）。

## Impact

- Affected code (expected):
  - `connectivity`：attempt/direct handshake 的 UDP I/O 边界将改为依赖 socket owner/demux，而不是直接读写 `*net.UDPConn`
  - `internal/punching`：punching 编解码增加 tag 前缀，并调整收发接口以适配 demux
  - `dataplane`：QUIC 改用 `quic.Transport`；KCP 增加 demux packetconn 适配；server listener 语义需要可并发 accept 多 peer session
  - `internal/pocacceptor`：数据面入口将逐步迁移到 “listener + accept loop + 多 session” 形态（避免单槽位）
- Backwards compatibility:
  - punching 报文 wire 发生变化；POC/主线未发布阶段允许 breaking，不要求与旧版本互通。
- Dependencies:
  - 不新增新的外部服务依赖；复用现有 QUIC/KCP 依赖栈，仅调整其接入方式与边界。

