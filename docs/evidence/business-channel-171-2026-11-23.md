# MIOPUNCH-171 在途字节与半关闭续接

可靠业务通道按两个方向分别保留未确认字节和接收进度。接收缓冲取得连续字节后确认，向应用成功写入的部分单独推进交付位置。重接先交换进度，再从对端尚未接管的位置发送。每次 `carry` 独占承载读写 future，旧承载全部结束后才能交给下一承载，已接管数据与部分应用写入进度保留在通道中。[MIOPUNCH-171](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/654c535f-e46c-4a58-8244-c2a70e3b02fc) · [ADR 0021](../adr/0021-business-channel-continuity.md)

FIN 携带该方向的最终字节位置，接收端先交付全部前置数据，再半关闭应用写端。另一个方向可以继续发送最终响应。发送侧按对端已交付位置限制窗口，接收侧最多保留一个既有 64 KiB 记录额度。节点总额度与具体恢复期限仍由 173 实测确定。首开就绪回执恢复仍由 172 实施。

## 真实入口证据

| 场景 | 核对结果 | 原始记录 |
| --- | --- | --- |
| 双向在途字节 | 原实现红灯在恢复后十秒未读齐。新实现 46.99 秒通过。真实黑洞期间，两端原 TCP socket 各提交 256 KiB 独立序列，双方旧 QUIC 超时后，经新 QUIC 读齐两组序列及恢复后新标记，目标只接受一次连接。 | `/tmp/miopunch-171-red/inflight.log`、`/tmp/miopunch-171-green/inflight.log` |
| ACK 丢失与半关闭交叠 | 47.81 秒通过。目标先独立读到 4,096 字节，测试功能丢掉对应通道进度回执。在黑洞中提交两个方向各 128 KiB 尾部并半关闭发送端。恢复后目标只读到余下尾部再收到 EOF，原目标可发送最终响应，客户端回显完整且无重复。退休连接的迟到关闭和失败开流不影响新通道。 | `/tmp/miopunch-171-boundaries3/ack-fin.log` |
| 文件服务续传 | 17.50 秒通过。真实 32 MiB 文件在目标保存部分字节后中断，文件服务按已存 65,536 字节续传，共两次尝试。双方保存回执一致，独立读取目标文件与原字节序列一致。文件仍使用原 `Ready.offset` 与保存回执协议。 | `/tmp/miopunch-171-regression/file.log` |
| UDP 报文与丢弃 | 6.19 秒通过。真实 IPv6 socket 独立确认 65,508 字节宿主报文合法，转发监听与目标回程分别丢弃该超长报文一次。同一关联随后继续完整收发 0、17、65,000、65,507 字节报文。UDP 产品代码保持原入口。 | `/tmp/miopunch-171-udp-fixed/udp.log` |
| 16 通道恢复 | 48.14 秒通过。单独运行原五秒业务断言，16 条原 socket 均在新 QUIC 上双向续接，通道 ID 保持且没有额外目标接受。 | `/tmp/miopunch-171-udp-fixed/recovery16.log` |
| 同 QUIC 换路 | 12.79 秒通过。原通道、原 QUIC、原流在 UDP 路径失效后经获准 relay 继续双向收发。 | `/tmp/miopunch-171-final/path.log` |
| 目标业务失败 | 48.53 秒通过。目标 RST 结束原业务，未再次打开目标。 | `/tmp/miopunch-171-final/ended-target.log` |
| 生命周期 | 四项通过，覆盖来宾配置、取消与重开、容量满额退出、独立清理。 | `/tmp/miopunch-171-lifecycle/results.json` |
| 普通 Native 库 | 无测试功能的 `cargo check --lib` 通过，517 Fresh、4 rebuilt。 | `/tmp/miopunch-171-default/receipt.json` |

## 复现入口

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio171-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test native_service_streams --test forwarding_lifecycle --no-run --jobs 2
```

从 Cargo 回执取测试产物。`original_tcp_sockets_replay_bidirectional_inflight_bytes_once` 与 `lost_channel_ack_and_interrupted_half_close_preserve_exact_tail` 使用 `--ignored --exact <name> --nocapture`，在启用 `lo` 的 `unshare -Urn --fork --pid --mount-proc --kill-child` 一次性命名空间中运行。文件与 UDP 两项通过普通测试入口运行。ACK 丢失的注入只在 `nat-lab-test-hooks` 功能下编译，默认实现没有该开关。

## 审查与剩余观测

独立 Standards 审查无发现。Spec 审查发现初版 UDP 夹具把超长宿主报文误写为 wire 记录，已改为上述真实 IPv6 宿主入口。既有 UDP wire 记录拒绝逻辑保留。两轴报告与限定复核见 `/tmp/miopunch-171-review.md`。

最终整文件运行一次，结果为 11 通过、9 失败、12 按环境忽略，记录在 `/tmp/miopunch-171-final/suite.log`。七个失败名称与 [170 报告](business-channel-170-2026-11-23.md) 相同，另外失败为 `native_service_parallel::udp_first_reply_keeps_the_started_tcp_round` 和已修正的初版 UDP 夹具。新 UDP 用例随后定向通过，整文件未重复运行。旧路径修复等失败仍未逐项核实根因。

一次 16 通道恢复复核在新成员连接准入阶段触发原五秒业务期限，记录在 `/tmp/miopunch-171-final/recovery16.log`。此时尚未进入新业务通道握手。待其他运行终态后按同一操作、同一五秒断言单独复核通过，保留两个运行的拨号时间线供对照。
