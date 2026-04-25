# Review fix design plan (2026-04-25)

> 状态：临时设计计划。  
> 目的：把 `review-current-go-code` 中“不是纯编码错误、需要先澄清设计/契约”的问题先说清楚，再决定后续是否进入 OpenSpec change 和代码修复。  
> 边界：本文件只讨论设计/接口契约问题，不修代码；nil deref、RandID error、TLS cleanup、TCP worker leak、gofmt 等纯实现问题另行处理。

## TL;DR

- 当前没有发现 Door-2/TCP/dataplane 主设计整体错误。
- 需要设计层澄清的是 3 个局部契约：
  - `wire.Dispatcher`：handler 生命周期、线程安全、发送错误语义。
  - `event.Emitter`：事件写失败是 best-effort 还是必须可观察。
  - `sh_attach` interactive CLI：远端/WebSocket 关闭时，本地 stdin 阻塞如何退出。
- 推荐后续修复采用“小设计澄清 + 小范围实现修复”，不要重做 control plane、event pipeline 或 shell protocol。

## Applied decision update (fix-review-design-contracts)

- `wire.Dispatcher` 采用“运行期安全注册”方案：handler registry 加同步保护，`Send` 仍是异步接受语义，底层读/写失败通过 dispatcher terminal error 暴露。
- `event.Emitter` 最终采用比原建议更直接的方案：`Emit` / `Start` / `OK` / `Fail` 直接返回 `error`，现有调用仍可选择忽略，关键路径可检查错误。
- `sh_attach` 采用“远端/WebSocket 关闭后 CLI 立即退出”方案：stdin reader 不再作为命令退出的阻塞条件。
- 验证中发现 `p2-05-tcp-spraying` 的失败不是 review 中三项设计契约本身导致，而是 TCP spraying lab/runtime 基线存在实现缺口：TCP punching 只发一轮 SYN、TCP sender/receiver 发包相位容易被 RST 锁死，且 `nat4-irregular` 注释承诺 45100 可达但缺少对应 DNAT。最终修复采用预算内限速重试、coordinator 下发 receiver 对齐延迟、补齐 lab profile DNAT，不放宽 lab 期望。

## 1. `wire.Dispatcher` 生命周期与发送语义

### 当前设计

- `Dispatcher` 封装一个 `io.ReadWriter`，通过 `Run()` 启动独立 `readLoop` 和 `sendLoop`。
- `RegisterHandler` / `RegisterDefaultHandler` 用于注册入站消息 handler。
- `Send` 当前实际行为是把消息放入 `sendCh`；真正的 `WriteMsg` 在后台 `sendLoop` 执行。
- `MessageTransporter.Send` 直接透传 `Dispatcher.Send`，上层 coordinator/peer 使用它发送 hello、SID、NatHole response 等控制消息。

### 设计出错点

- handler 注册时机没有被定义：当前代码既有 `Run()` 前注册，也有 `Run()` 后按角色补注册。
- `RegisterHandler` 写 map，`readLoop` 读 map；如果允许 `Run()` 后注册，就必须定义线程安全语义。
- `Send` 名字和返回值容易被理解为“消息已写入 wire”，但当前只能表示“消息已入队”。
- 因此这不是 NatHole/TCP 协议设计错误，而是 control-plane dispatcher 的 API 契约不完整。

### 方案 A：规定所有 handler 必须在 `Run()` 前注册

- 优点：契约简单；不需要锁；读路径最快；容易用测试锁住。
- 缺点：现有 hello 后按 peer role 注册 handler 的代码需要重排；运行中动态注册能力被禁止。
- 适用：如果我们决定 dispatcher 是严格的 build-then-run 对象。

### 方案 B：允许 `Run()` 后安全注册 handler

- 优点：贴合当前调用方式；改动较小；后续按角色动态安装 handler 也自然。
- 缺点：需要给 handler registry 加同步保护或 copy-on-write；handler 调用时不能持锁，否则容易阻塞注册和读循环。
- 适用：如果我们承认 dispatcher 是运行期可配置对象。

### 方案 C：拆成显式 lifecycle：`Register*` 阶段 + `Start` 后冻结

- 优点：长期语义最清晰；启动后误注册可以返回错误或 panic；设计上避免 data race。
- 缺点：需要调整调用方结构；对当前 POC 修复来说偏重。
- 适用：如果后续要把 control-plane wire 层做成更稳定的公共内部 API。

### 方案 D：重做 dispatcher 为同步 transport / request-response manager

- 优点：可以系统性解决写确认、错误传播、响应匹配、关闭语义。
- 缺点：牵动面大，容易把 review fix 变成 control-plane 重构。
- 适用：不建议作为本轮修复方案，只能作为后续架构演进方向。

### 建议

- 采用方案 B。
- 明确设计契约：
  - `RegisterHandler` / `RegisterDefaultHandler` 可在 `Run()` 前后调用，并且对 `readLoop` 并发安全。
  - handler registry 读取与更新必须同步；调用 handler 时不得持有 registry 锁。
  - `Send` 只表示“发送请求已被 dispatcher 接受”，不保证底层写成功。
  - 底层 `WriteMsg` 失败时，dispatcher 必须关闭并暴露失败状态，调用方至少能通过 `Done()` / error accessor / log 或事件观察到。
- 后续实现不应把 `Send` 改成阻塞等待 wire 写成功；那会改变现有异步发送模型并扩大风险。

## 2. `event.Emitter` 写入失败语义

### 当前设计

- `event.Emitter` 用于输出机器可读 JSONL 事件。
- `Emit` / `Start` / `OK` / `Fail` 都没有返回值。
- 多个 spec 把 diagnostics/event stream 作为可观测性和 lab 验收依据，例如 connectivity/dataplane 的阶段事件和 payload evidence。
- 当前 `Emit` 内部创建 `json.Encoder` 后直接丢弃 `Encode` 错误。

### 设计出错点

- 设计没有区分“普通日志事件 best-effort”与“关键证据事件必须可观察失败”。
- API 没有返回错误，调用方无法知道 stdout/pipe/file writer 已失败。
- 这会让 lab 或用户以为系统没有产生某些阶段证据，但真实原因可能是事件输出失败。
- 因此这不是 dataplane 业务设计错误，而是 observability API 的错误传播契约缺失。

### 方案 A：保持 `Emit` 无返回值，只记录内部 last error

- 优点：调用面改动最小；兼容所有现有调用。
- 缺点：调用方仍然容易忽略错误；如果没有统一检查点，错误只是被换个地方藏起来。
- 适用：仅适合快速补充调试信息，不足以支撑关键证据链。

### 方案 B：新增可检查错误路径，保留现有 best-effort 便捷方法

- 优点：兼容现有调用；关键路径可逐步改成检查错误；风险小。
- 缺点：API 变成双路径；需要约定哪些事件必须检查。
- 适用：最适合当前 POC 阶段和 review fix 范围。

### 方案 C：把 `Emit` / `Start` / `OK` / `Fail` 全部改为返回 `error`

- 优点：Go 风格最直接；所有调用方都会被迫面对错误。
- 缺点：调用面很大；容易把事件输出失败和业务失败混在一起；短期噪音大。
- 适用：如果未来 event package 成为稳定公共 API，可以考虑一次性收敛。

### 方案 D：引入异步事件管道和 fatal error channel

- 优点：适合高吞吐和统一 flush；可以集中处理 writer failure。
- 缺点：引入 goroutine 生命周期、缓冲、关闭、丢弃策略等新复杂度。
- 适用：当前不需要，且会制造新的并发审查面。

### 建议

- 采用方案 B。
- 明确设计契约：
  - 默认 `Emit` 仍是 best-effort 兼容包装，不 panic。
  - 新增可返回错误的方法，用于关键事件和测试。
  - 关键验收事件（例如 `transport.payload_exchanged`、阶段 fail、最终结果事件）应优先走可检查路径或在上层可观察地记录失败。
  - event writer 失败不应默认改变 connectivity/dataplane 业务结果，除非调用点明确把事件证据作为该流程的必需输出。

## 3. `sh_attach` interactive CLI 退出语义

### 当前设计

- `miopunch sh ...` 创建 `sh_attach` task 后连接 LocalAPI WebSocket。
- CLI 有 websocket writer goroutine、stdin reader goroutine、主 goroutine websocket read loop。
- WebSocket 读循环结束后调用 `cancel()`，然后等待 `wg.Wait()`。
- stdin reader 阻塞在 `os.Stdin.Read`；普通 context cancellation 无法打断这个阻塞 read。

### 设计出错点

- `miopunch-poc-shell-v0` 定义了 WebSocket frame 语义、tmux anchoring、single-writer lock，但没有定义 interactive CLI 在远端关闭/网络断开/任务结束时的退出语义。
- 当前实现隐含假设：所有 goroutine 都能响应 context 取消；这个假设对 terminal stdin 不成立。
- 这不是 shell wire frame 设计错误，而是 CLI 本地生命周期和 terminal I/O 取消语义没有设计清楚。

### 方案 A：远端/WebSocket 关闭后，CLI 立即恢复终端并退出，不等待新的 stdin 输入

- 优点：用户体验符合预期；修复 hang；不需要复杂 terminal 控制。
- 缺点：stdin reader 不能作为必须等待的 goroutine；测试里要避免把它变成泄漏检查噪音。
- 适用：最适合当前交互式 CLI 修复。

### 方案 B：主动关闭或替换 stdin 来打断 `Read`

- 优点：理论上 goroutine 生命周期最完整。
- 缺点：不能安全关闭真实 `os.Stdin`；跨平台行为复杂；可能破坏用户终端。
- 适用：不建议用于产品 CLI。

### 方案 C：改用非阻塞/轮询 terminal read

- 优点：可以让 stdin read 响应 context。
- 缺点：Unix/Windows terminal 细节复杂；会让 POC shell 变重。
- 适用：如果未来做完整 terminal runtime，可重新评估。

### 方案 D：抽象 interactive I/O 为可关闭 reader/writer

- 优点：测试友好；长期可维护；有利于 Windows/非 TTY 支持。
- 缺点：当前修复范围偏大；需要重构 CLI I/O 入口。
- 适用：后续 shell 子系统成熟化时考虑。

### 建议

- 采用方案 A。
- 明确设计契约：
  - 当 WebSocket 关闭、远端任务结束或网络错误发生时，interactive CLI 应恢复 terminal 状态并尽快退出。
  - CLI 不应等待用户再敲一次键才能结束。
  - stdin reader 不得成为进程退出的阻塞条件。
  - 退出后仍应尽力获取最终 task 状态并执行 `--report` 导出；获取失败时不要再次造成交互式 hang。

## 后续落地建议

- 下一步创建独立 OpenSpec fix change，范围可以命名为 `fix-review-design-contracts` 或类似名称。
- 该 change 同时包含：
  - 上述 3 个设计契约的 spec/design 更新。
  - 对应最小代码修复和测试。
- 纯编码错误可以作为同一个修复 change 的第二组任务，也可以拆成更小的 `fix-review-code-issues`；如果追求快速降低风险，建议同一个 change 内分组处理，但 review 时分开验收。

## 验收建议

- `wire.Dispatcher`：race-sensitive 单测覆盖 `Run()` 后注册 handler；写失败时 dispatcher 关闭且错误可观察。
- `event.Emitter`：failing writer 单测覆盖可返回错误路径；兼容方法不 panic。
- `sh_attach` CLI：fake websocket / fake stdin 场景覆盖远端关闭且 stdin idle 时不会卡住。
- 代码修复进入 mainline 前按 `$dev` 执行完整 host gates；如触及 lab/runtime 行为，再按要求执行 lab gates。
