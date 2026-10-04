# RTC 成员 PONG、非空 replica 与真实保存边界

对应 [WebRTC 第 9 步](2026-10-03-browser-webrtc-replan.md#9-真实成员业务和非空-replica)。真实 Chromium browser 与 Linux native 在自有正常 TLS/private NSS namespace 中完成成员 `PING`→`PONG`，随后通过独立的 REPLICA M 运行现有 p2panda LogSync，验证、持久保存一条带 36-byte 正文的原作者签名记录，并在同 origin/profile 的 IndexedDB 重开后经 SDK 读回完全相同的 header/body。[证据包](../evidence/browser-rtc-member-replica-2026-10-04/README.md)保留来源、范围、每 M 资格、保存回执、真实失败和重开结果。

本次资格是已保存当前材料的 browser QUIC dialer/RTC offerer→native answerer，ICE/STUN/TURN 列表为空。前置材料准备的 WSS relay 已关闭，测量 M 初始使用 MQTT signaling，普通 STREAM 只走获准 RTC；未把材料准备认作第 10 步首次入网资格，也未声称异网、IRC RTC 或成本资格。

## 普通业务与独立同步

有限成员消息的现有 `PING`/`PONG` handler 提取到 `member_message`，桌面处理器与已经通过当前成员检查的 RTC BUSINESS 处理器共享它。原 browser 任意字符串回显兼容仍保留；本次验收实际调用公开 `BrowserMemberConnection.exchange("PING")`，不是私密诊断消息或新 ALPN。

BUSINESS 与 REPLICA 分别拥有原 M、ALPN、binding/stable-id 检查、自己的初始 fresh ACK、放行前新的 exact-path probe/watermark/ACK，以及最终 route seal。先关闭 BUSINESS 后才运行 REPLICA；前者不借给后者连接或资格。同步实际 stream、两端 RTC STREAM/byte counters 和成功 DataChannel.send 提交及 message 事件独立记录，后者不被称为外层 UDP wire 计数。

原记录是已有支持的管理 `Add`，赋予一个新生成的惰性第三身份 `Access::read()`，正文为 `original-author browser replica body`。native 当前管理员通过 p2panda builder 签名，经真实 SDK ingest 验证、应用和保存。没有新增聊天、设备名或签名产品 API。source 与 author 本例相同，不能据此声称已经运行第三作者离线转发场景；现有 native offline-replica 测试提供相关回归证据。

目标在 browser 同步前经 `control_record(id)` 证明不存在。同步使用原作者 public header、原正文、依赖和来源 scope，不把 provider 身份重写成 record author。SDK 完整验证签名、正文长度/hash、因果依赖和组权限；最终 frozen scope 为 3 条记录，saved/applied scope 和 complete 一致，native 收到真实 durable Receipt。原 header/body 与接收后及重开后的 SDK 读回逐字节一致。

## 已保存记录不等于原 M 资格失效

初次真实非空同步曾在目标记录实际保存之后失败：全局 SavedNode snapshot revision 因合法新记录变化，原 RTC lease 随之退休，无法发送最后 Receipt。这份失败与前面的 PING 回显失败分别保留；它不是保存前失败。

现在 browser gate 继续发布真实 saved snapshot/source diagnostic revision，但 RTC 使用有界、属于原 M 的独立 current qualification source。保存新记录时重新检查原成员双方、proof scope、当前秘密世代/凭据、资源与生命周期；这些仍允许该 M 时，source revision 可以变化而该 M 的 permission revision 保持。真实丧失资格或凭据改变时，原 source 同步变更、唤醒、退休 exact route 并关闭原 M，旧 lease 不可复活。

CurrentFn 只读取独立 atomic 和弱 M 的开放状态，不在 Noq 回调内重入 gate/Connection lock。实现范围是此次实际 browser consumer；native 当前 qualification source 保持现状。最终正常同步观察 source revision 1→2、per-M permission revision 1→1、currentPermission true、same-session/binding true，随后正常关闭。

## 保存前的实际中断

失败故事使用另一条新 REPLICA M 和另一条新原作者记录。测试只在真实 IndexedDB readwrite transaction 的 proposed SavedNode 包含该新目标 public header/body 时，一次性调用该实际 transaction 的 `abort()`，发生在目标 `put`/commit 之前。其余事务不触发 fault，原方法在 finally 恢复。

SDK 返回实际存储失败，未返回 complete，native 未接受 durable Receipt。关闭失败 M/RTC、BrowserNetwork 并释放锁后，在同 origin/profile 重开：失败目标仍经 `control_record` 证明不存在，先前成功记录逐字节保留，原 identity、Complete、authorization_saved 与 secret_saved 均保持。此资格是 storage-commit interruption；未声称进程 crash、网络 outage 或整个 rotation 产品。

browser connect 和 receive 共享原 12 秒 absolute caller deadline；native provider 的 20 秒内部上限只缩短 caller remaining，不重开时钟。公开 native 回归先消耗 400 ms caller 的 300 ms，再建立真实非空 manifest 并令 client 停顿，证明 provider 在原剩余时间内失败并关闭原 M、保持存储。每个独立新 M 有自己的有限操作时钟。

## 运行和兼容范围

```sh
rtk proxy python3 scripts/check_browser_paths.py --case rtc-member-replica --output /tmp/mio-rtc-member-replica
```

入口默认先构建 native/WASM/bindgen，单独记录构建时间。实验从 owned namespace/trust/fixture 准备前开始，工作最多 105 秒，保留 15 秒清理，总计最多 120 秒；产品 12/10/2 秒预算未增加。最终含 normal、abort 与两次重开共 24.284 秒，源冻结 60 文件，实际执行 5 个产物。

同一源/产物的四个第 8 步 STANDARD 故事均通过：cancel 后 distinct 新 M 成功、真实签名成员移除并 browser SDK 应用、真实 durable native 当前秘密世代安装、实际已安装 RTC carrier 禁用。正常 WSS 兼容回归为 41.666 秒，涵盖原较短 caller remaining、取消、超过 setup deadline 的存活 M 和原 empty replica smoke。该 empty smoke 不承担本步非空资格。此前 stopped 的 held-PC/notification-delay 诊断未重试或改造成通过。

109 项当前相关 native 测试通过，包括完整 library、管理/存储/native network/offline replica/daily management targets 和 RTC guard/release/invalidation targets；native all-targets、WASM rtc/default typecheck、format 与 driver syntax 通过。未重跑未改动慢 NAT 矩阵。具体命令、计数和源码 hash 在回执，worker/focused 重复不额外累加。

三条测量原 M 均关闭，browser PC/channel 和 SDK/观测 callbacks 拆除，native final queues/pumps/associations/routes 归零，socket count 回到实际 baseline 6。该结果是 measured quiescence，不声称所有资源瞬间归零。各 fixture 退出 0；五个自有清理命令均成功并查询确认 container/network 不存在。

最终审阅、提交及其来源身份以证据包为准；第 10、11 步不因本回执自动完成。同步、修复 wire、crypto、默认 backend/resource、UI/IWA 和 TURN 产品选择未新增。
