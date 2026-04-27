## Context

当前 `dataplane` 在 TCP/TLS 与 KCP 会话上使用 `smux` 作为逻辑流 multiplex。由于 `smux.Session.AcceptStream()` 不支持 `context.Context`，我们为了支持 `PeerSession.AcceptStream(ctx)` 的取消/超时，采用了 `SetDeadline(1s)+轮询` 的实现方式。

该实现方式的核心风险在于：`SetDeadline` 是 session 级共享状态。如果未来出现并发 `AcceptStream`（哪怕是误用），不同 goroutine 会互相覆盖 deadline，导致非确定行为与排障困难。

参考 `frp` 的 TCPMux 实现：其采用 `yamux`，并通过（fork 后提供的）`AcceptStreamWithContext(ctx)` 原生支持取消语义，从根上避免 deadline 轮询。

## Goals / Non-Goals

**Goals:**

- 让 TCP/TLS 与 KCP 的 mux 层具备原生 `context` 取消语义，移除 `SetDeadline+轮询` 的实现风险。
- 在 mux 库选择上尽量贴近 `frp` 的现行做法，减少“踩坑面积”。
- 不改变 `stream-open(kind+metadata)` 的 wire 语义，不改变 QUIC native streams 的实现。
- 降低可观测性噪声：logical stream `Close()` 返回 err 不再直接打成 fail 事件。
- 明确 inbound session 的 `remote_peer_id` 取得时序：hello/auth 前可为空，可信 identity 以认证后为准。

**Non-Goals:**

- 不在本轮引入新的 wire 握手字段来把 peer id 融进 TLS pinning（保持现有 pinned identity 机制）。
- 不实现 “session re-key / verified identity 后重建 SessionKey” 之类的复杂缓存迁移。
- 不改 `CloseReason` 的枚举体系与语义范围（保持现状）。
- 不引入 FRP 风格 `keepTunnelOpenWorker` 等额外长连接保活策略（仅保留 mux 自身 keepalive 默认行为）。

## Decisions

### 1) 使用 yamux（FRP pinned fork）替换 smux

- TCP/TLS 与 KCP multiplex 从 `smux` 切换到 `yamux`。
- 采用 `github.com/hashicorp/yamux` 作为 import path，但在 `go.mod` 中 `replace` 到 `github.com/fatedier/yamux` 的固定版本（与 `frp` 一致），以获得 `AcceptStreamWithContext(ctx)`。

**Rationale:**

- `AcceptStreamWithContext` 能直接消除 deadline 轮询与共享状态干扰风险。
- 保持 hashicorp import path，未来上游合并后移除 `replace` 的成本更低。

### 2) AcceptStream(ctx) 使用原生 context 取消

- `PeerSession.AcceptStream(ctx)` 在 yamux 上直接调用 `AcceptStreamWithContext(ctx)`。
- `ctx.Done()` 返回时直接返回 `ctx.Err()`，不触发额外的 session close reason 污染（上层已经在 shutdown/取消时主动关闭 session）。

### 3) yamux 配置最小化，并抑制库级 stderr 日志

- 使用 `yamux.DefaultConfig()`，并将 `LogOutput` 设为 `io.Discard`，避免库内部日志直接污染 stderr。
- 可选对齐 `frp`：把 `MaxStreamWindowSize` 提升到 `6MiB`，减少大 payload 下的窗口阻塞风险。

### 4) inbound remote_peer_id 的时序约束

- `SessionKey.RemotePeerID` 在 inbound（acceptor/serve）侧允许为空：建立 session 时并不知道可信 peer id。
- `stream-open.metadata.peer_id` 属于 declared identity；只有在 hello/auth 校验通过后，才认为该 peer id “可信”。
- 本轮不做 session re-key；诊断事件中 `remote_peer_id` 可能为空是允许的。

### 5) logical stream close 事件降噪

- `transport.stream_close` / `transport.stream_accept_close` 事件总是 `info`。
- 若 `Close()` 返回 err，把错误放进 `kvs.close_err`；Fail 事件保留给明确的协议/传输致命路径（例如 stream-open 解析失败、底层 session fatal）。

## Risks / Trade-offs

- [Risk] 新依赖引入兼容性差异（smux vs yamux）。  
  → Mitigation: 保持 wire 协议不变；维持现有单元测试覆盖；执行全套 host + lab gates。

- [Risk] yamux 默认 keepalive / window 等参数与历史行为不同，可能影响极端网络下表现。  
  → Mitigation: 初始以默认配置为基线，必要时对齐 `frp` 的窗口配置；参数变更单独开 change 评估。

- [Risk] 采用 fork 版本依赖（通过 replace）。  
  → Mitigation: 固定到与 `frp` 一致的 pseudo-version；后续跟踪上游合并后再移除 replace。

## Migration Plan

1. 切换 mux 实现并更新依赖。
2. 跑单元测试与必要 lab gate，确认三种数据面（TLS/KCP/QUIC）均可建立 session/stream。
3. 回滚策略：直接 revert 该 change 的提交即可（不涉及持久化状态迁移）。

## Open Questions

- 未来是否需要在 hello/auth 成功后补充一个显式的 `peer_verified` 诊断事件（用于把 “declared” 与 “verified” 分开记录）？
- 是否需要将 yamux 的关键参数（窗口/keepalive）纳入 Config（或保持内部常量）？

