# MioCube 原会话分批恢复

MIOPUNCH-204 基于 `ce54876c`。当前原生成员由 FUSED owner 负责恢复。

## 交付行为

原会话上的专用控制流先交换当前获准、去重后的候选。每帧受 4096 字节边界约束，
每个提交批次最多八项，后续批次持续递补。原会话的路径验证与候选发送并行，收到
首批可用候选即可验证。双方确认实际路径 ACK 并完成 COMMITTED 后保留胜出路径，
其他临时候选随本次尝试结束释放。

原控制尝试期间新学到的联系在实际 fallback 时读取，仍共用原截止时间。
独立联系保持同一 TLS 绑定和原会话身份。每个数据 relay、成员转发或直接联系
至多三秒，MQTT、IRC 旁信道可用原调用方剩余总期限。入口切换及后续批次共用
同一截止时间。原控制接收方也将请求方预算夹紧到宿主期限内，超时后结束自己的
交换及 reservation。取消和原会话终止沿用同一结算路径。

v1 初始请求及响应继续使用 EndTerminated，每个方向各一条。双方协商批次扩展后，
在同一保护联系上另开一个有界帧交换。新请求保留一个旧版可消费的初始 offer，
未协商的对端继续使用旧格式。旧字面请求获得历史单批及字节边界内的响应。

正式 FUSED 的显式 `MemberConnection::repair_path_until` 与隐式恢复 owner 均先用
原控制。旧 raw `Connection` 的任意业务 ALPN 仍由其调用方接收业务流，旧公开
repair API 通过独立保护联系恢复原路径，并使用新的逐入口期限。其边界由主线程
按父规格 195、ADR 0019 和 0020 确认。

## 真实入口

`/tmp/miopunch-204-entries.json` 记录首次八项定向结果。第九候选的原 red
`/tmp/miopunch-204-ninth-red.log` 返回 `invalid data path offer`，随后原入口通过。
原流在恢复前后均双向收发，保留两端稳定身份。

- `repair_uses_the_ninth_candidate_on_the_original_stream` 使用八个无法通过认证的
  UDP 目的及第九个真实绑定，保留原连接及业务流。
- `slow_signaling_repairs_from_a_second_batch_after_three_seconds` 经自有 MQTT broker
  传送第二批，延迟至少 3.3 秒后仍在原七秒期限内恢复。
- `fast_contact_yields_to_slow_signaling_without_restarting_the_repair_clock` 先让真实
  快入口静默到三秒，再经 MQTT 等待超过三秒的第二批，两阶段共用原七秒时钟。
- `repair_validates_a_usable_first_batch_before_the_delayed_second_batch` 在第二批
  发出前的一秒内收到实际路径验证 READY。
- `live_member_repairs_over_its_original_control_and_keeps_the_stream` 保持原 relay
  控制及两端原流，独立 before_connect 计数保持不增。夹具另发送 120 ms 预算并
  持住请求流，确认接收方到期结算，再成功发起下一次恢复。

后两项审查后的聚焦结果为
`/tmp/miopunch-204-reviewed-entries.json`。同批大候选集旧字面请求的前置错误由
重复默认 IPv4 bind 引起，已改为额外 socket 使用非默认 bind，原 wire 断言保留。
最终该入口及其余原用例纳入下面完整回归。

## Standards

独立 Standards 全审无发现。

## Spec

独立 Spec 首轮四项 P2 涉及全列表期限、原控制接收方预算、未协商响应边界及
旧 responder 的临时候选保留。四项均经同一调用链修复，原审查者已聚焦确认关闭。
随后新增边界断言、服务故障夹具，以及最新 learned 联系的读取时点经同一审查者
聚焦复核，无新增问题。服务正例的 MQTT 断言证明真实发起联系；不单独声称该联系
完成候选协调。该完整行为由独立的慢批次及真实 responder 用例覆盖。

## 构建与回归

最终 Native 库、network、service 和 CLI 使用
`/tmp/miopunch-204-settled-build/receipt.json`，466 Fresh、6 rebuilt、67.28 秒。
大候选集 literal 夹具修正只重编 path 目标，收据
`/tmp/miopunch-204-literal-fixture/receipt.json`，468 Fresh、1 rebuilt、5.65 秒。
实际产物固定为 `.artifacts/miopunch-204-delivery.{lib,path,network,service,cli}`。

WASM 共享源码检查通过，收据 `/tmp/miopunch-204-wasm-final/receipt.json`，
294 Fresh、2 rebuilt、13.57 秒。原恢复核心仍由浏览器共享，浏览器现用 active
业务控制协议及其 WebRTC 载体沿用既有入口。本票的宿主运行验收覆盖 Linux。

完整受影响回归顺序运行 lib、path_maintenance、network_maintenance 和
native_service_streams，使用两个自有 MQTT broker 及同一批 Native 固定物。
结果和原始日志保存在 `/tmp/miopunch-204-regression-runtime/`。这是末次聚焦修正前
的完整运行，保留失败；不将其写成全部通过。

| 集合 | 通过 | 失败 | 忽略 |
| --- | --- | --- | --- |
| lib | 67 | 2 | 8 |
| path_maintenance | 23 | 5 | 1 |
| network_maintenance | 28 | 1 | 9 |
| native_service_streams | 17 | 4 | 33 |

lib 两项沿用前票预算／first-contact 失败；network 唯一失败仍是已有 TCP 换网
观察入口，交 MIOPUNCH-205。service 的 ping 重开与 UDP-first-reply 重开在
`initial_data_path` helper 等值断言失败，沿用 203 的未精确归因限制。

path 的大候选 literal 失败属于重复默认绑定的夹具前置条件，已修正并保持原 wire
断言，末次聚焦通过。其余四项分别停在初始 bounded connection、两项对端 closed
等待和物理 idle/lifetime 等待，尚未完成各自核心恢复断言，未证明为本票回归。
本票正式 FUSED 终止后的拒绝及新 ID 双向收发由新增的原控制真实入口直接验证。

service 两项 MQTT／撤权入口最初只切断 proxy，备直连可继续原流，不能保证触发
旧 MQTT 前置。夹具现于自有网络 namespace 内阻断两端实际 TCP 监听并核对真实
DROP，保留原五秒 I/O、联系次数、稳定 ID、新路径 ACK／STREAM 接收及撤权断言。
这进一步暴露原控制优先引入的联系快照时序：同一 stable ID 在 6.432 秒学到 MQTT，
9.438 秒 fallback 仍使用原先空列表。诊断位于 `/tmp/miopunch-204-learning-service.log`。
修复只把联系快照移动到实际 fallback，不重启总时钟；临时日志已移除。

末次定向结果另列，原完整失败日志保持不变。


## 末次定向验收

前五项结果及原始日志来自 `/tmp/miopunch-204-complete-results.json` 和
`/tmp/miopunch-204-diagnosis-results.json`。慢 responder 初次失败是测试打开新 QUIC
流后先等接收、未写首字节，实际 handler 还无法 accept；改为先发真实首批，
保持延迟和全部完成断言，原入口通过。末次 owner 联系快照修正不改变这些显式入口。

| 实际入口 | 结果 | 核心断言 |
| --- | --- | --- |
| 全 fast 双联系 | 通过，8.28 秒 | 首次静默三秒后第二次尝试仍在原调用方时钟；原 ID、旧流双向数据 |
| 真实慢 responder | 通过，10.43 秒 | 第 3.3 秒后发送唯一可达批；实际 handler 完成 Ready/Committed、原流双向数据 |
| 旧 literal 大候选集合 | 通过，9.38 秒 | 实际候选超过八个；旧响应仍限八项及消息字节边界 |
| 第九候选原流 | 通过，4.12 秒 | 退休其他路径后双向数据，两端胜出路径 STREAM 接收计数增长 |
| 正式 FUSED 原控制 | 通过，6.63 秒 | 原控制优先无外拨、120 ms 接收预算、取消结算、终止后拒绝 repair，既有入口新 stable ID 双向收发 |

最终 Native path/network/service 同图编译收据为
`/tmp/miopunch-204-current-contacts-final/receipt.json`，463 Fresh、8 rebuilt、61.14 秒。
最终可复用固定物为 `.artifacts/miopunch-204-final.{path,network,service}`。
两项原服务故障用例均使用这个最终 service：

- `restricted_contact_repairs_the_live_native_forwarding_stream` 通过，总耗时 32.33 秒；
  原流首次回复为 4.0438 秒，仍受原五秒断言约束。实际发起一个 MQTT 联系，保留
  两端 connection／stream ID，新路径具备 ACK 及业务 STREAM 接收增长。
- `membership_revocation_closes_the_original_even_when_path_repair_has_started` 通过，
  总耗时 23.51 秒；真正阻断期间 MQTT 已发起，再执行撤权，旧流 EOF、成员移除及
  后续 ping 拒绝均通过。

结果在 `/tmp/miopunch-204-final-service-results.json`。总耗时含初始化及清理，不等于
恢复耗时。未重新运行完整集合；沿用有效的原回归和新增聚焦证据。

回收前读取宿主可见执行及文件引用：本票首批 13 个无后续消费者的中间固定物
释放 2,789,605,376 字节，末批 6 个诊断／替代固定物释放 1,308,626,944 字节。
明细在 `/tmp/miopunch-204-pin-cleanup.json` 与
`/tmp/miopunch-204-final-pin-cleanup.json`。保留最终三个入口固定物及用于未决失败
复现的原完整回归固定物，未删除共享依赖或工具。
