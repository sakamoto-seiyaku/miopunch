## Why

当前 `PeerSession.AcceptStream(ctx)` 为了支持 `context` 取消，在 `smux` 上通过 `SetDeadline + 轮询` 的方式实现。这引入了两个问题：

1. `SetDeadline` 是 session 级共享状态，未来一旦出现并发 `AcceptStream`，会有非确定性干扰风险。
2. 可观测性层面，`logical stream Close()` 的返回 err 很容易被误判为“失败事件”，造成噪声。

我们希望在 mux 库层面尽量减少“自己造取消/超时语义”的坑，并参考 `frp` 的现行实现做出更稳妥的选择。

## What Changes

- 将 TCP/TLS 与 KCP 会话的多路复用从 `smux` 切换为 `yamux`（与 `frp` 保持一致），并使用 `AcceptStreamWithContext(ctx)` 以原生方式支持取消/超时。
- 保持 QUIC 的 native streams 方案不变。
- 将 `logical stream close` 诊断事件改为总是 `info`，并在 `kvs.close_err` 中携带 close 失败原因（不再把 close err 直接打成 fail）。
- 在设计/规格中明确：inbound session 在完成 hello/auth 之前，`remote_peer_id` 可能未知（允许为空），可信 peer id 以“认证后”为准。
- 清理依赖：移除 `smux` 依赖，引入 `yamux` 依赖（采用 `frp` 的 pinned fork 以获得 `AcceptStreamWithContext`）。

## Capabilities

### New Capabilities
- （无）

### Modified Capabilities
- `miopunch-dataplane`: TCP/KCP peer transport session 的 mux 实现从 smux 调整为 yamux；并补齐与 `context` 取消、stream close 诊断噪声、inbound peer identity 时序相关的约束说明。

## Impact

- 代码：`dataplane/session_transport.go`（TLS/KCP mux 实现）、`dataplane/session.go`（stream close 事件）、相关单元测试。
- 依赖：`go.mod` / `go.sum`（新增/替换 yamux，移除 smux）。
- 文档/规格：本 change 的 design/spec/tasks；并为后续 sync 到主 specs 提供 delta spec。

