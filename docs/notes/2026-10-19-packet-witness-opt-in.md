# Packet witness 的启用边界与实际工作

2026-10-19，[MIOPUNCH-104](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/2ed5ec95-0117-4e33-9f4b-29aba2653680)
核对既有 Noq／Noq-proto 私有 packet witness。依据 [ADR 0012](../adr/0012-preserve-library-contracts-and-defaults.md)，
保留普通消费者的默认行为，以及既有显式诊断入口。本项没有修改诊断生产 Rust
逻辑、权限、密码、调度或 features；补充实际公开接口回归与契约说明。

实施选择是继续使用现有的 `install`／`finish` 执行时 opt-in，不另加诊断 Cargo
feature。模块上的 `doc(hidden)` 只隐藏文档，不是 `cfg(test)` 或编译隔离。
实际 Native、WASM 和普通 Noq consumer 的 depfiles 均包含 `packet_witness.rs`；
encoder hash 观察另有既有 `ring` cfg，两种边界不能混称。这里没有全 provider／
全 target 的诊断覆盖承诺，也未调整原库默认 features。

## 所有权、关闭与 callback

observer 是进程级 `OnceLock`，初始关闭；必须在自有 endpoint 启动前显式安装。
同一进程只能成功安装一次，所有使用该 crate 实例的连接共享它。`finish` 关闭
接收并读取累计 status，不 reset、不允许重装，也不等待 producer 或正在执行的
callback。调用者先结束自有 endpoint／pump 等 producer，再 finish；注册的
closure 仍由进程级槽保留到进程结束。

callback 同步执行，Noq driver 的调用点已经持有 connection lock。它只能观察，
不能执行 I/O、异步等待、等待另一个 SDK task，或调用会重入该锁的 Connection
方法。本项回归只用 `try_lock` 处理有界本地计数，并调用 WeakConnectionHandle
的原子 `is_open`。递归 `emit` 会记 loss 并停用；它不检测任意 Connection API
重入。callback panic 在 unwind 配置下被捕获、记 loss 并停用；不承诺 panic=abort
的捕获。自定义 callback 遵守上述约束是调用者的责任。

核心预算是最多 16,384 条 record、16 MiB 的字段计费量；每条至多 32 个字段，
单个 Text 至多 256 KiB、Bytes 至多 64 KiB。scalar 按 8 字节计费，字段名也
计费。这不是整个 heap、Vec capacity、callback 自有状态或文件的内存上限。
字节预留早于 record 预留，因此 status.bytes 也不能当作已经回调／写入文件的
总字节。越界、递归或 callback panic 会记 loss 并停止接受，已有计数不清零。
并发 producer 不能据此推断严格的最后事件顺序；finish 仍要求先回收 producer。

Native 的既有 PrivatePacketCaptureFixture 另有 1,024 项有界 writer channel、
每个 proof map 512 项及 24 MiB 文件上限。回调使用非阻塞提交，文件 I/O 在独立
writer thread；finish 在 producer 结束后送终止标记并 join。Unix 私有文件权限
为 0600；该 Native 文件适配器不代表 Browser 已有对应 JS 安装接口。核心预算、
observer／writer 丢失和 writer 结束必须一起核验，loss 不能当完整抓取证据。

## 按启用状态归属工作

| 路径 | 未安装或已关闭 | 已启用 |
| --- | --- | --- |
| Noq drive context | 计算 stable id、读取 enabled、普通分支；不设置诊断 TLS | 设置／恢复线程本地连接上下文 |
| ring STREAM encoder | 仍保存 offset／buffer length 标量并读 enabled；正常编码照常 | 对实际已追加 payload 计算 SHA-256，构造文本与字段 Vec |
| ring builder／加密完成 | 读取 enabled；正常 frame／加密工作照常 | 构造 header 观察、摘要、格式化文本与字段 Vec |
| async sender 边界 | 读取 enabled；packet／application／socket 许可照常 | 有界复制实际 ciphertext 到 Vec，再发观察记录 |
| Native PONG／RTC pump | enabled 检查与普通传输工作 | 摘要、格式化、字段复制和观察 |
| 核心 emit | 原子状态读取后返回 | TLS 递归标志、字段检查、原子预算预留、同步 callback |
| 自有 Native writer | 未启动便无该 writer | proof map／同步元数据、try-send、JSON 序列化与文件写入 |

以上是源代码工作分类，不是指令、分配次数或 CPU 测量。没有诊断专属 hash／
ciphertext-copy／字段 Vec 构造的生产分支，仍不等于整个网络路径没有分配、复制、
锁或诊断状态检查。直接调用 `emit(stage, fields)` 时，Rust 已先计算实参；调用者
要避免停用状态下自己的字段构造，应先检查 enabled。观察所含 bytes／digest
字段数也不是整个程序的 allocator 字节或 hash 调用计数。

## 实际验收与证明边界

首次仓库改动前，原版和当前 fork 的固定 Iroh／Noq consumer 在相同受控环境
重跑：IP、自有 HTTP relay、旧 custom sender、单路与健康多路的真实 stream
均正常结束并释放 UDP／relay 端口；没有改动固定消费者来配合新能力。

`tests/diagnostic_work.rs` 通过现有公开 observer 入口，在十个独立进程运行
同一 32,768 字节实际 UDP／TLS stream：未安装、启用、finish 后、递归 callback、
panic callback、record／byte／field／Bytes／Text 上限。每例都另使实际 packet
gate 拒绝一个新 stream，确认 observer 不使它进入 carrier；endpoint、UDP
端口均在断言最终资格前正常回收。递归和 panic 在实际 Noq drive poll 发生，
capture 记 loss 并停用后，同一连接的普通 stream 仍完成。没有通过调用会重入
connection lock 的 API 来制造挂死并把它当成可用行为。

未安装和 finish 后的 records、callbacks、字段计费、copy 与 digest 观察均为零；
启用场景的实际计数与各失败边界保存于[工作量记录](../evidence/packet-witness-opt-in-2026-10-19/diagnostic-work-measurements.json)。
这说明对应观察是否发生，不是启停后的包数、时序或性能完全相同。诊断增加 hash、
copy、allocation、callback 和 writer 工作，可能影响背压与时序，不能将其运行
时长直接纳入没有诊断标记的性能比较，也不声明吞吐、延迟或 SLA。

`ordinary-stream-declaration` 只声明写入意图；`actual-payload-appended` 和
`actual-packet-encrypted` 证明对应编码阶段。`before-actual-carrier-call` 位于
真正 sender 调用之前，可能接着 Pending；`actual-sender-returned-ok` 也不是
物理发送证明。Iroh carrier 可以按原语义抑制失败，多个 packet 可共用一次
coalesced／GSO transmit。RTC pump 的 dequeued／try-send／accepted 仍只是对应
边界，必须与真实 carrier、对端接收及业务结果联合判断，记录本身不授予权限。

本项生产 Rust 输入没有变化，Native／WASM SDK 沿用逐项核对实际 selected inputs
与 raw pins 的 M103 构件。新增回归只编译自身，锁的完整外部行、来源、checksum、
profile、工具链及共享 target 不变。CID、fresh probe、迁移、本地 socket 与既有
最终 guards 另作针对性运行；完整当前 SDK 验证默认关闭诊断的准入、持久化、
双方同身份重开、成员 PONG 与自然回收。最终回执、构件来源和双轴审查见
[验收证据](../evidence/packet-witness-opt-in-2026-10-19/README.md)。公网、实体设备和
全平台矩阵保留原有独立范围。
