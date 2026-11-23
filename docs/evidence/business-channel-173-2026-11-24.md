# MIOPUNCH-173 有界等待与恢复收尾

本票让业务通道的本地读写寿命跨承载流保留。恢复期间仍接收有限应用字节，节点按发送和接收两个方向分别管理共享额度。原通道完成 Ready 及双向进度交换后结束恢复轮，拨号、Ready 和进度交换失败继续受同一截止约束。节点既有连接所有者负责连接复用，暂时连接失败复用现有 ContactState 退避。[MIOPUNCH-173](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4a5e7f8c-16b1-40a0-b704-4ddcc539e992) · [ADR 0021](../adr/0021-business-channel-continuity.md)

## 当前默认值及依据

`ResourceConfig.business_channel` 的 `ChannelPolicy` 默认恢复窗口为 60,000 ms，每通道每方向为 256 KiB，节点每方向为 2 MiB。配置在原节点资源文件保存并由原生节点启动使用。接收窗口随进度交换，允许两端配置不同额度。

在两节点、两个真实应用 socket、自有回环 MQTT 联系和 TCP 数据路径的 debug 测量中，双向各 4 MiB 共 8 MiB 数据逐字节核对通过，耗时 3.5215 秒，合计约 2.38 MB/s。发送保留峰值每节点为 262,144 字节，接收保留峰值为 65,563 字节。整个测试进程的 RSS 从 60,788 KiB 增至 78,600 KiB，后者包含两节点、独立发送/接收数据数组共 16 MiB 和传输运行时。记录在 `/tmp/miopunch-173-rx-green/measure.log`。

256 KiB 在这次吞吐测量中允许每方向约 0.22 秒的数据并行；节点每方向 2 MiB 可容纳八份满额通道，也让单条满流留下其他通道可用的额度。发送和接收各有自己的池，双向同时发送不会因共享同一份额度而阻止对端接管和确认。默认保留的有效负载总上界每节点为 4 MiB。每条活跃流另有最多一条 64 KiB 解析记录、64 KiB 本地读暂存和有限的传送副本，普通 socket 与 QUIC 的既有缓存另计。

黑洞后解封到双通道原数据交付为 4.724 秒，16 原 socket 的重连验收整体 48.96 秒，包含旧 QUIC 默认约 30 秒的终止等待。60 秒恢复窗口在承载流因网络故障结束后开始，覆盖本次实际冷恢复时间并保留继续尝试的余量。健康静默时间与 30 秒旧承载终止等待均不消耗这份恢复窗口。高铁轨迹的真实终端表现由后继 174、175 验收，当前取值及原始测量可供该轮核对。

## 真实入口结果

| 场景 | 实际观察 | 日志 |
| --- | --- | --- |
| 离线有限写入与单次等待取消 | 旧 QUIC 超时、网络仍封锁时 4096 字节进入发送缓存；取消一次 100 ms 读等待后，解封仍完整交付并继续新序号。 | `/tmp/miopunch-173-green/offline.log` |
| 双向发送额度与共享重连 | 每方向首通道 65,536 字节，另一通道使用 16,384 字节余额后，节点达到 98,304 字节。满写等待可被调用方取消，恢复后 360,448 字节及后续序号正确，两通道共用一个新成员连接。 | `/tmp/miopunch-173-final/limits.log` |
| 双向接收额度 | 两方向分别达到 98,304 字节。DATA 可以先接管剩余容量内的前缀，应用恢复读取后交付完整原字节并继续新序号。 | `/tmp/miopunch-173-rx-green/receive.log` |
| 健康静默与单向活跃 | 使用 100 ms 恢复配置，健康静默 300 ms 后继续往返，持续单向交付 400 ms 后仍保留原连接。 | `/tmp/miopunch-173-expiry/quiet.log` |
| 固定恢复截止 | 新 QUIC 在约 4.886 秒建立，但 Ready 被暂停。原通道在同轮约 12.101 秒终止，获得新连接并未重新开始 12 秒恢复期。 | `/tmp/miopunch-173-final/deadline.log` |
| 已确认未读尾部 | 接管 100 字节，应用读取 60 字节，余下 40 字节留在通道接收缓冲。1.2 秒恢复期满后先收到完整 40 字节，再读 EOF，公开状态为 `business_channel_recovery_expired`，目标只接受一次。 | `/tmp/miopunch-173-final/expiry.log` |
| 断网时显式取消 | 本端立即结束，仍不可达的对端在原 1.2 秒恢复窗口内回收，两端原 socket 最终结束，无第二次目标接受。 | `/tmp/miopunch-173-cancel/cancel.log` |
| 目标移除与显式取消 | 断网恢复期间移除一个目标，该原对象结束。其他目标保留并恢复，随后显式取消关闭其两端原 socket，目标均无迟到 accept。 | `/tmp/miopunch-173-limits/remove.log` |
| 撤销访问 | 恢复期间撤销 TCP 服务访问，两端原对象结束，目标无二次 accept，仍获准的 ping 可继续。 | `/tmp/miopunch-173-boundaries/revoke.log` |
| 来宾实际到期 | 使用现有来宾转发角色，在 relay 断路期间跨过真实授权截止，目标结束且无新 accept；普通成员业务仍获准。 | `/tmp/miopunch-173-boundaries/guest.log` |
| ACK 丢失及半关闭 | 复用 171 的真实原 socket 场景，恢复后尾部、FIN 与最后响应正确。 | `/tmp/miopunch-173-cancel/ack-fin.log` |
| 16 通道 | 复用 170 原五秒业务断言，16 条原 socket 在不同 QUIC 上恢复，目标共接受 16 次。 | `/tmp/miopunch-173-final/recovery16.log` |

生命周期四项通过，结果在 `/tmp/miopunch-173-lifecycle/results.json`。普通无 hook Native 库检查通过，回执 `/tmp/miopunch-173-native-final-check/receipt.json`。WASM 共享配置及线格式检查通过，回执 `/tmp/miopunch-173-wasm/receipt.json`。独立双轴审查及限定复核无未关闭发现，记录在 `/tmp/miopunch-173-review.md`。

## 复现入口

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio173-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test native_service_streams --test forwarding_lifecycle --no-run --jobs 2
```

使用回执中的测试产物。网络故障用例在 `unshare -Urn --fork --pid --mount-proc --kill-child` 一次性命名空间启用 `lo`，并以 `--ignored --exact <test-name> --nocapture` 运行。`receive_node_quota_is_shared_and_released_by_application_delivery`、健康静默及测量三项为普通测试。准确场景名在 `tests/native_service_streams.rs`，本票顺序驱动在 `/tmp/miopunch-173-run-final.py`、`/tmp/miopunch-173-run-rx-final.py`。Cargo 继续使用原共享缓存及锁，既有 artifact store 容量限制沿用。

## 限制

普通 TCP socket 以 EOF 表达最终中断，详细到期原因由已有 `forwarding_status().listeners[*].counters.last_error` 读取。网络完全不可达时，本端显式取消立即停止本地对象；无法传递取消的对端保留对象仍受既定恢复期限约束。正常半关闭继续保留相反方向的读取。

普通完整文件批次运行一次，17 项通过、6 项失败、22 项按环境忽略，耗时 108.72 秒，日志 `/tmp/miopunch-173-rx-green/suite.log`。最后增加的隔离网络取消用例另行运行通过。六个失败是受限原流修复、反向修复、静默首选备份联系、交叉联系、取消修复和静默 relay 竞速；它们均已出现在 172 的八项失败名单中，本票未逐项重新定位根因。原日志、操作和断言保留。已通过的 173 额度、截止、收尾和相关原 socket 真实入口独立列在上表。

原 16 通道五秒冷恢复在前几票存在时序失败与通过样本。本票同一原断言通过，仍保留旧拨号日志供真实终端回放对照；一次通过不能证明所有恢复延迟均小于五秒。
