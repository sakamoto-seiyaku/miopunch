# MIOPUNCH-170 原 TCP socket 跨成员连接继续收发

本票实现已建立原生 TCP 业务在无在途字节断点上的接回。业务通道保留原应用 socket，成员连接终止后通过节点现有联系、拨号和授权入口取得新承载。通道 ID 与已授权对端、原目标绑定。双向进度、在途重放和跨承载半关闭由 MIOPUNCH-171 继续完成，首次就绪回执丢失由 172 完成，恢复期限与资源额度由 173 完成。[MIOPUNCH-170](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/05c5f4c3-9be8-41c5-b80c-9235c54865d1) · [ADR 0021](../adr/0021-business-channel-continuity.md)

## 实施与验收

`BusinessChannel` 将应用读写对象与每次成员流分开持有。原生 TCP 监听默认使用该通道，旧的单次交换和来宾专用协议仍通过原有 Open/Ready 记录收发。正常 FIN 保留半关闭语义，目标本地 IO 失败会中止当前流，网络 `TimedOut` 或 `Reset` 才进入接回。实际业务容量仍为 16，保留通道提供有界的恢复头部接收容量。

| 入口 | 核对内容 | 原始记录 |
| --- | --- | --- |
| `native_tcp_channel_keeps_both_sockets_after_member_replacement` | 49.80 秒通过。16 条真实 TCP 业务各保留两端 socket。隔离双方 TCP 数据监听及联系数据，双方原 QUIC 均 `TimedOut` 且防火墙有丢包。恢复网络后在新 QUIC 上继续双向序号，通道 ID 保持，目标累计接受 16 次，没有第 17 次。 | `/tmp/miopunch-170-final-build/recovery16.log` |
| `native_tcp_channel_keeps_quic_and_stream_when_only_udp_path_is_cut` | 9.52 秒通过。先用原 socket 完成 UDP 往返并核对实际 STREAM 帧增长。在无业务在途字节时切 UDP，独立确认 relay 被选中且有故障后 ACK，再以原五秒期限双向收发新序号。原通道、QUIC 和流标识保持，relay 双向 STREAM 从 0 增至 2。 | `/tmp/miopunch-170-final-build/path.log` |
| `ended_tcp_target_is_never_opened_again_after_member_replacement` | 46.96 秒通过。原目标 RST 后结束业务，监听计数 `failed=1`、`completed=0`，目标没有第二次接受连接。 | `/tmp/miopunch-170-final-build/ended-target.log` |
| `forwarding_lifecycle` 四项 | 配置来宾重开、实际待处理调用取消及重开、16 流满额提供端结束、独立清理判定均通过。每例使用独立节点目录及自有回环 relay。 | `/tmp/miopunch-170-lifecycle/results.json` |
| WASM 库检查 | 共享 `forwarding.rs` 的可选通道记录通过 `miopunch-browser-probe` 的 WASM 检查。 | `/tmp/miopunch-170-wasm/receipt.json` |

最终 Native 构建通过，466 个 Cargo artifact 为 Fresh，4 个重编。命令与输出在 `/tmp/miopunch-170-final-build/receipt.json`。两轴独立审查的发现已修复并限定复核，Standards 与 Spec 均无未关闭发现，报告在 `/tmp/miopunch-170-review.md`。

## 复现

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio170-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test native_service_streams --test forwarding_lifecycle --no-run --jobs 2
```

前三项使用 Cargo 回执中的 `native_service_streams` 产物，在 `unshare -Urn --fork --pid --mount-proc --kill-child` 创建的一次性命名空间中启用 `lo`，用 `--ignored --exact <test-name> --nocapture` 运行。UDP 换路样本还建立了地址为 `10.170.0.1/24` 的 `mio170` dummy 接口。当前 QUIC 默认空闲期限为 30 秒，终止前置使用 36 秒上界。业务收发继续使用已有五秒期限。生命周期驱动为 `/tmp/miopunch-170-run-lifecycle.py`，提供既有测试所需的 `M118_FORWARD_PRIVATE` 与 `M118_LOCAL_RELAY`。

## 已知限制与后续

旧的边切路径边写入用例在五秒内未取得回复。`accepting_peer_alone_can_contact_and_repair_the_original_native_stream` 在 HEAD 原实现上也复现同样失败，记录在 `/tmp/miopunch-170-baseline-build/path-baseline.log`。BBRv3 与显式 Cubic 的对照均失败，记录在 `/tmp/miopunch-170-ab2-build/`。UDP 故障后 4.5 秒的诊断显示 relay 已选中并有新 ACK，旧 UDP 的在途 STREAM 尚未在 relay 重传。这些在途失败交给后续进度与重放工作核对，原 MIOPUNCH-154 历史证据保留在 [既有报告](native-iroh-network-switch-154.md)。

最终普通整文件运行得到 11 项通过、7 项失败、10 项按环境忽略，耗时 152.37 秒，完整结果在 `/tmp/miopunch-170-final-build/suite.log`。失败包括受限原流修复、交叉联系、取消修复、重开后的地址复用及静默 relay 竞速。其中仅接收侧原流修复已做 HEAD 对照；其余失败逐项根因尚未核实。三个新的隔离网络验收已独立运行通过。业务在途状态、首次打开重试及默认资源边界仍依后续票推进。

现有 immutable artifact store 超过其 32 GiB 门槛，固定测试产物的尝试被拒绝。共享 Cargo `target/` 的当前产物可用于接力，后续复现可按本提交与上述命令重编。
