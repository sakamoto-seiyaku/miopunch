# MIOPUNCH-162 多个已验证地址的重启首拨

正式 NativeClient 在已有 `member.contacts.json` 中学习获准路径。UDP、原生 TCP 和原有 relay 记录保留各自地址类型。只有路径上的认证 ACK 提供验证时间，读取旧路径本身不会刷新这个时间。已验证且尚未选中承载业务的路径也能学习。原字段与 300 秒近期窗口、节点及每成员记录上限继续生效。

原生客户端复用现有一秒维护 tick 和关闭前的刷新，读取 AdmissionGate 发布的有效业务弱连接，同时保留旧维护消费者。每条新选定成员连接的既有业务接收任务还订阅一次路径事件，通过退役事件的 final stats 保存两次快照之间消失的已验证路径。发起与接受的共享连接均由同一 selected 通道送入该任务，复用连接不会重复订阅。文件更新继续由原存储锁和原子写负责。应用只提供近期、获准的记录；融合拨号 owner 在同一绝对期限内并发起拨所有合格 TCP／UDP 缓存候选及获准 relay 或受限联系。每条新连接照常确认 TLS 身份、准入和真实数据路径，沿用共享连接裁决及对明确准入拒绝的终止处理。

## 正式入口验证

- `native_service_parallel::udp_first_reply_keeps_the_started_tcp_round` 先通过 MQTT 保护联系，在 UDP 上读回首笔转发业务。放开 TCP 观察后，同一原连接验证 TCP。该 TCP 路径当时未选中，双向 STREAM 帧计数均为零。最终定向用例立即退役该备路，核对它已退出活跃路径列表，并在客户端保持运行时读回已验证的持久记录，原 UDP 业务流仍继续往返。关闭并重启同一身份且禁用 UDP，仅输入对端身份时，已保存的 TCP 地址交付真实 PONG。
- 将已实际验证的旧 UDP、TCP 端口分别交给独立静默 socket 与 listener。重启目标与拨号方后，两种旧缓存都实际收到首拨；健康 relay 在约 0.49 秒交付 PONG。只有 MQTT 受限联系时也不等旧地址超时，约 0.97 秒交付 PONG。第二笔请求复用各自首笔的连接标识。
- 原有 `native_ping_reuses_a_verified_member_address_after_reopen` 通过，覆盖已有 UDP／relay 文件、较新与较旧记录、仅公告、过期、禁用、错 TLS 身份、SCRAM 中断及目标明确拒绝。其既有 debug 栈限制使用前序已确认的 8 MiB 定位环境运行，耗时约 63 秒。
- 原联系存储 3 项通过，2 项独立远端实验保持忽略。普通端点 2 项、受限端点 19 项和存储期限／取消 1 项通过。浏览器 WASM 检查通过。

## 复现与证据

共享构建回执为 `/tmp/miopunch-162-focused-build/receipt.json`、`/tmp/miopunch-162-unused-build/receipt.json` 、`/tmp/miopunch-162-retire-build2/receipt.json` 和 `/tmp/miopunch-162-review-wasm/receipt.json`。未新增依赖、协议消息或 Cargo 图缓存。

TCP 未保存的初始红测为 `/tmp/miopunch-162-learning-red.log`。学习与 TCP-only 重启绿测为 `/tmp/miopunch-162-learning-green2.log`，双协议缓存竞速为 `/tmp/miopunch-162-multiple-final.log`，未承载路径断言为 `/tmp/miopunch-162-unused-final.log`，立即退役和完整重启竞速为 `/tmp/miopunch-162-retire-final.log`。旧入口验证及许可／身份对照为 `/tmp/miopunch-162-legacy-reopen.log`。普通、受限、存储和联系文件的结果分别为 `/tmp/miopunch-162-ordinary.log`、`restricted.log`、`store.log`、`contacts.log`，这些尾名均使用相同的 `/tmp/miopunch-162-` 前缀。

前序 159 的 raw allowlist closed-path 失败与 debug 默认栈问题，以及 161 的换网停止观察断言限制继续见各票证据。完整仓库运行全绿仍未成立。

完整根库单元测试一次运行得到 65 通过、2 失败、4 忽略，日志为 `/tmp/miopunch-162-full-lib.log`。两项失败的同宿主、同 exact 入口也在现成前置产物上复现。`active_native_budget_deferral_preserves_another_members_original_stream` 在路径掩码后立即断言全为 relay 失败，旧产物为 11 月 21 日 15:54 的 `miopunch_native_probe-226b931e26f19520`，日志 `/tmp/miopunch-162-active-pre162.log`。RTC silent observer 的 150 毫秒 caller 与 500 毫秒上限断言失败，旧产物为 11 月 18 日 11:12 的 `miopunch_native_probe-0c43e1db2b04653d`，日志 `/tmp/miopunch-162-rtc-pre162.log`。两项测试和核心断言保留原样。
