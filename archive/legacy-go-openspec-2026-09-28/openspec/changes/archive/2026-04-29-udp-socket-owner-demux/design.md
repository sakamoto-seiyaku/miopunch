## Context

`P3` 传输层纲领已明确两条硬约束：

1. 对 UDP 路径，traversal（`gather/attempt/punching`）与数据面会话（`dataplane`）必须共享同一个本地 UDP socket/端口映射，不能靠拆 socket 绕过收包冲突（否则 traversal 观测到的映射与数据面实际使用的映射不一致，成功率与正确性都劣化）。
2. UDP socket 必须存在单一 owner：只有 owner 负责读包（`ReadFrom/recvmmsg`），并把报文分流给 punching 事务与 QUIC/KCP 会话层；同一端口必须支持多 peer 并发 accept 多条 peer transport session。

当前实现形态与上述约束冲突：

- `connectivity.Attempt` / `internal/punching.MakeHole` 在多个 goroutine 中直接 `ReadFromUDP` 同一个 `*net.UDPConn`。
- `dataplane` 的 QUIC/KCP 直接用 `quic.Listen` / `kcp.ServeConn` 接管同一个 `*net.UDPConn` 并读包。
- quic-go Transport 文档与实现明确：将 `net.PacketConn` 交给 Transport 后，再对该 conn 调用 `ReadFrom/WriteTo` 是无效用法（未定义行为），因此必须把 punching I/O 也纳入 Transport 侧的收发边界。

本设计的目标是：在保持对外选择面（`data-proto=kcp|quic` + `quic-cc=bbr|brutal`）不变的情况下，补齐 “UDP socket owner/demux” 架构，使 attempt/punching 与 dataplane 能在同一端口、同一 socket 上正确共存，并允许同端口并发多 peer session。

## Goals / Non-Goals

**Goals:**

- UDP path 具备单点收包 owner，并按明确规则分流：
  - punching/direct handshake 报文进入 traversal 事务处理；
  - QUIC / KCP 报文进入数据面会话层；
  - 不允许并发多个组件直接读同一 `*net.UDPConn`。
- QUIC 路径用 `quic.Transport` 作为 socket owner，并通过 `ReadNonQUICPacket` 承载 punching 收包。
- KCP 路径在同端口可并发 accept 多条 peer transport session（不再依赖“只 accept 第一条”或单槽位语义）。
- 保持 `--quic-cc bbr|brutal` 不回归（两种模式都要在新架构下可运行、可回归）。

**Non-Goals:**

- 明确不做 “每个 logical stream 都重新打洞 / 重建底层连接”。成本更高、成功率更差、也更难做并发与可观测。
- 不引入 relay / 中继服务器；打洞成功就直连，失败就失败。
- 不做自动协议切换或传输迁移（仍保持单次选择、单次建立、单次运行）。

## Decisions

### 1) Punching 报文增加明文 tag 前缀（5B）

**Decision**: 规定 punching（包括 direct handshake 的 `NatHoleSid`）在加密 payload 之前增加固定前缀：

- `00 4D 50 00 01`（`0x00 + "MP" + 0x00 + ver(=1)`）

**Rationale**:

- `quic.Transport.ReadNonQUICPacket` 的非 QUIC 判定非常严格：仅当“首字节前两位为 0”才视为 non-QUIC。使用 `0x00` 起始可确保 punching 报文稳定进入该分支。
- 明确区分 punching vs KCP/QUIC 数据面报文，避免靠 conv 过滤或“读到什么算什么”的不确定性。
- `ver` 为将来扩展预留（例如 tag 版本升级或增加校验）。

**Wire rule**:

- `punch-wire := tag(5B) || crypto.Encode( wire.WriteMsg(msg), key )`
- 接收端必须先检测 tag，再进入 crypto decode；未命中 tag 的报文不得尝试按 punching 解密。

### 2) QUIC：使用 `quic.Transport` 作为 UDP socket owner

**Decision**:

- QUIC 侧不再使用 `quic.Listen` / `quic.Dial` helper 直接接管 `*net.UDPConn`。
- 统一创建 `quic.Transport{Conn: udpConn}`，由 Transport 单点收包。
- traversal/punching 的收包改为 `Transport.ReadNonQUICPacket(ctx, buf)`。
- traversal/punching 的发包改为 `Transport.WriteTo(payload, addr)`（不再对 `udpConn.WriteTo` 直写）。

**Rationale**:

- Transport 明确声明：把 `PacketConn` 交给 Transport 后，调用 `ReadFrom/WriteTo` 是无效用法；继续直读直写会引发非确定性冲突与丢包。
- Transport 支持同一 socket 上同时作为 server（accept inbound）与 client（dial outbound），并可并行多连接，满足“同端口多 peer 并发 session”。

### 3) KCP：引入 UDP owner/demux + PacketConn 视图

**Decision**:

- KCP 路径不尝试让 kcp-go 与 punching 直接共享一个裸 `*net.UDPConn` 的读循环。
- 引入一个 UDP owner：
  - 唯一 goroutine 对底层 `*net.UDPConn` 做 `ReadFromUDP`
  - 若报文命中 punching tag，则送入 punching 事务 channel（供 attempt/punching 消费）
  - 否则送入 KCP 的 inbound queue，并通过一个实现 `net.PacketConn` 的 wrapper 暴露给 kcp-go（kcp-go 只会从该 wrapper “读到 KCP 包”）
  - 写包同样经由 owner 的单出口写到 UDP（保证写路径也能统一观测/限流/统计）

**Rationale**:

- 在不依赖 kcp-go 内部识别规则的前提下，保证 punching 报文不会污染 KCP 的 accept 逻辑。
- owner 模式为后续可观测性（drop 统计、queue 长度、延迟）提供稳定边界。

### 4) Attempt / Punching I/O 抽象：不再直接依赖裸 UDPConn 收包

**Decision**:

- `connectivity.Attempt` / `internal/punching.MakeHole` 不再自己起 `ReadFromUDP` 轮询抢包。
- 抽象出 “UDP demux endpoints”：
  - punching side：`Recv(ctx) (payload []byte, from net.Addr)` / `SendTo(payload, to net.Addr)`
  - dataplane side：由 QUIC Transport 或 KCP packetconn wrapper 自己消费

**Rationale**:

- 只有这样才能从架构上保证 “任何 UDP 收包只发生在 owner”。
- 也为未来“并发 attempt（按 dial_id 分桶）”打基础：多个事务可以共享 owner，而不是抢同一个 UDPConn。

## Risks / Trade-offs

- **[Risk] wire breaking**：punching 增加 tag 前缀导致旧版本互通失败。→ **Mitigation**：明确作为主线 POC 期 breaking；在 spec 与 release notes 标注不向后兼容。
- **[Risk] QUIC 接入改动幅度**：从 helper 迁移到 Transport 需要梳理 listener/dial 生命周期与 Close 语义。→ **Mitigation**：先以最小可回归路径落地（同端口单 listener + 支持 dial），并补充多 peer 并发用例。
- **[Risk] KCP demux 复杂度**：需要实现 PacketConn wrapper 与队列，稍有不慎会引入死锁/丢包。→ **Mitigation**：owner loop 单 goroutine + 有界队列 + 明确 drop 策略；先写单测覆盖 tag 分流与并发 accept。

## Migration Plan

1. 先落地 punching tag 与 QUIC Transport 接入（QUIC 路径最容易被 owner 语义“强制正确”）。
2. 再落地 KCP owner/demux 与 PacketConn wrapper，并移除依赖 `conv==1` 的“误判过滤”作为正确性基础。
3. 最后重构 `connectivity.Attempt`/`punching.MakeHole` 的 I/O 形态，使其通过 demux endpoint 工作，而不是直接读 UDPConn。

## Open Questions

- KCP owner/demux 的队列容量与 drop 策略：默认采用固定小队列（例如 64/128）还是按 MTU/内存预算动态？本 change 先采用固定有界队列并记录 drop 计数（避免无限内存）。

