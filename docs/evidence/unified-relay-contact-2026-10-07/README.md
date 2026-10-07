# Relay 起步和联系监听验证

MIOPUNCH-77 的 public relay、显式成员 data relay 起步和三个候选失败场景已通过真实 SDK 操作验证。MIOPUNCH-78 的非对称 member signaling、公共 MQTT 联系、断线撤回、代际隔离和禁用收尾已有实际通过记录。最终独立审核已关闭：Spec 0 个问题，Standards 的共享 Cargo 缓存入口问题已修正并确认解决，0 个剩余问题。

| 场景 | 实际结果 |
| --- | --- |
| Public relay 起步再改善 RTC | 原 M 和原 stream 保留；初始与改善操作的 60 秒期限后继续传输，完整观测约 114 秒。 |
| 显式成员 data relay 起步 | 三个独立成员实际授权、保存与重开、replica 同步；同 M 从 MID1 改善到 RTC，原 stream 继续 PONG。停用 MID1 后原 M 继续传输。 |
| 成员转发被禁用或未显式开启 | 数据转发被拒绝；独立正常 M 的 RTC 和 stream 保留。 |
| 丢失 fresh probe | Native 真实初始 probe 创建后丢包；候选失败并回收，健康 relay 上同 M、同 stream 继续 PONG。 |
| 候选 driver 发送失败 | 实际首次二进制 DataChannel.send 抛错；候选回收，原 relay stream 继续。 |
| RTC 实测 RTT 更差 | Browser RTC/relay 约 3.28/1.06 秒，Native 约 1.89/1.17 秒；两端保留 RTC 候选、选择 relay，原 stream 继续。 |

[results.json](results.json) 保存经过筛选的结果、实际运行 receipt 的路径与 hash、双端帧观测和独立收尾。成员 data relay receipt 的控制器 exit 字段缺省；实际 Root 执行均为 exit 0，结果表把这一执行记录与 receipt 字段区分。原始失败、完整运行记录和 SDK 构件保留在各自不可变的本机证据目录；完整私有记录不随本说明发布。

## 联系监听和回归

MIOPUNCH-78 的十份实际 case receipt 在结果表中逐项链接。覆盖 member signaling 优先、其不可用后的公共 MQTT fallback、对侧 relay 健康时的常驻受限 listener，以及断线撤回、现有退避重连和旧代回调拒绝。禁用时停止 lookup/replica 联系；保存操作在替换前失败保留原始配置，在替换后目录同步失败保持 fail closed，独立正常 M 的业务继续。

最终相容回归包含两个现有 Root 检查和四个实际共同提交边界检查。成功提交后的控制连接关闭不回滚 M；提交前控制关闭、原 M 关闭或当前资格失效都拒绝提交并清理候选。旧代回调检查明确是 private unit，并未写成公开 SDK 回调。

两个检查脚本现通过既有共享 cache lease、command 和 environment 编译，并在租约内固定 primary/focused 构件。原始 compiler JSON、Fresh/rebuilt 计数与失败记录保留。真实独立进程争用测试验证固定构件期间租约未释放；source/lock 改变在 Cargo 前拒绝、构建期间改变在固定构件前拒绝，旧构件不匹配也先于可选 typecheck 拒绝。这些是控制器边界检查，未启动 Rust 编译或 SDK 网络测试。

## 源码和构件

功能检查的 receipt 保留各自实际编译源码版本。最终格式整理仅由 rustfmt 修改布局，Native 真实 probe 的创建和确认日志保持原行为；Root 与 Browser 格式检查均通过。最终 Native 和 Browser SDK 使用共享 target 增量构建通过，耗时约 18.4 秒和 10.1 秒，分别重建 3 和 1 个构件。源码、锁图、工具和所选 .d 输入由构建驱动在 Cargo 前及缓存租约内校验。

最终源码 digest 为 `4eb990b87f1a288c468a85063ee543ebe0d87cf4ca8457f170118b193ca26255`。实际 SDK 输入计数为 Native 6576、Browser 3158。格式整理后的构件通过编译；功能运行证据属于其注明的格式整理前版本，不把构建成功写成新的运行结果。

## 测量范围

实际 public data relay 使用 APS1，联系设施使用已配置公共 MQTT；RTC 与受控 direct 路径位于本机 Host 端点。物理跨网络资格由后续独立任务验证。

Native 初始 probe 日志实际观测四类业务帧为零。Native 路径快照的持续 DATAGRAM 计数不可用，保留为 UNOBSERVED；driver 首次发送失败发生在路径形成前，该场景帧计数也保留为 UNOBSERVED。

拥有的 PC、DC、pump、timer、连接、临时进程和进程 socket 已自然回收。Browser SDK 结束后仍观测到 quiet 的 relay/MQTT CLOSING socket 尾部，100 毫秒内流量没有变化；全部 SDK 终止任务退出和 relay graceful close 未获完整证明。临时 Browser 进程自然退出后的 socket 为零，记录中没有强制终止。

## 独立审核

[Standards 首次报告](standards-review.md)、[修正关闭报告](standards-closure.md)和 [Spec 报告](spec-review.md)分别保留。共享缓存修正只改变 Python 检查脚本，未改变最终 SDK 的 555 个冻结源码输入；真实功能矩阵和 SDK 构件保持原有版本与结果。
