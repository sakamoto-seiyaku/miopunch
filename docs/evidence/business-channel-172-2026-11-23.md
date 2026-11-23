# MIOPUNCH-172 首次打开应答丢失后的结果

原生 TCP 转发监听现在保留首次打开的请求身份。目标已接受连接而首个 Ready 随网络中断丢失时，重连接回同一目标 socket 并补发原就绪结果。调用方继续使用原 socket。若原打开记录已丢失，公共 `forwarding_status()` 的 `listeners[*].counters.last_error` 为 `open_outcome_unknown`，原本地 socket 关闭，提供端不会自动再连目标。[MIOPUNCH-172](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/0ecf2285-705a-4443-91c3-5ce3b522bc04) · [ADR 0021](../adr/0021-business-channel-continuity.md)

## 请求与对象

提供端在连接目标之前原子登记请求与原业务所有者，重复请求读取原连接结果。已连接的重复请求获得 `Connected { attached: false }` 并结束该复制流；原通道继续拥有唯一的 IO 承载。恢复请求的代际递增，旧代际无法接管新承载。

首次创建绑定原 QUIC 的现有 TLS exporter 上下文与原服务流 ID。缺少原记录时，只有原始服务流可产生第一次目标连接。换流或换连接的原始请求副本返回未知结果。原记录存在时核对对端、目标和首次打开来源，再认回同一对象。这里使用现有 TLS exporter，未引入新的哈希或依赖。

## 真实入口与结果

最终运行均使用默认拥塞控制、真实 TCP 监听与目标、独立目标 accept、原两端 socket。一次性 Linux user/network namespace 中封锁双方 TCP 数据端口，旧双方 QUIC 均以 `TimedOut` 结束并记录实际丢包。每次业务收发保留已有五秒断言。首开未知结果在节点已有六十秒重连预算内观察。

| 用例 | 实际结果 | 最终记录 |
| --- | --- | --- |
| `lost_initial_ready_rejoins_the_once_accepted_tcp_target` | 46.17 秒通过。目标独立 accept 后暂停首 Ready，强制旧 QUIC 超时；恢复后原 socket 双向新序号正确，新 QUIC、原通道 ID、目标累计 accept=1。 | `/tmp/miopunch-172-final/ready.log` |
| `duplicate_and_delayed_open_leave_one_effective_channel` | 45.33 秒通过。首 Ready 仍暂停时复制原 Open，恢复后再送达旧代际原 Open 和相同恢复请求，均得到原连接结果并结束复制流；原通道可继续双向收发，业务所有者和目标 accept 均为1。 | `/tmp/miopunch-172-final/duplicate.log` |
| `lost_open_record_reports_unknown_without_opening_another_target` | 46.88 秒通过。在原 QUIC 上丢弃记录后复制原 `resume=false` 请求，明确返回未知且目标接受次数不增；再经真实断网重连，公开监听状态为 `open_outcome_unknown`、原本地 socket EOF、目标累计 accept=1。 | `/tmp/miopunch-172-final/unknown.log` |
| MIOPUNCH-171 ACK 丢失与半关闭 | 43.35 秒通过。独立接管4096字节后丢 ACK，断网期间两向131072字节尾部和 FIN，恢复后的精确尾部、原半关闭、最终响应与旧承载迟到关闭均正确。 | `/tmp/miopunch-172-final/ack-fin.log` |
| `forwarding_lifecycle` 四项 | 4/4通过，覆盖来宾重开、取消待处理业务、满额提供端结束和独立清理。 | `/tmp/miopunch-172-lifecycle/results.json` |
| 构建 | 最终 Native 测试构建469 Fresh、1 rebuilt。无 hook Native lib check及共享协议的 WASM lib check均通过。 | `/tmp/miopunch-172-final/receipt.json`、`/tmp/miopunch-172-origin-default/receipt.json`、`/tmp/miopunch-172-origin-wasm/receipt.json` |

独立 Standards/Spec 审查和限定复核完成，生产实现无未关闭发现。Spec 指出的缺记录后迟到原 Open 二次连接已修复，并由上述未知结果用例覆盖。审查报告为 `/tmp/miopunch-172-review.md`。

红灯 `/tmp/miopunch-172-red/ready.log` 在相同首 Ready 丢失操作下读到原 socket 的 early EOF，38.72秒失败。最终三个首开场景的运行索引为 `/tmp/miopunch-172-final/real-results.json`。

## 仍开放的恢复时序

16 通道复核仍在原五秒首回复期限内超时，日志 `/tmp/miopunch-172-final/recovery16.log`。解封前既有路由已开始拨号，第二候选在约43143ms发起、43873ms才完成准入并报告 route-dial succeeded，随后业务期限到达。MIOPUNCH-171 曾出现同类启动迟滞，也有保持原操作与原五秒断言的通过记录。本轮日志无法独立排除接回开销，保留为跨票开放问题，交173在固定恢复窗口与多通道共用重连的验收中核查。[171 记录](business-channel-171-2026-11-23.md)

最终 `native_service_streams` 整文件运行一次，12项通过、8项失败、15项按环境忽略，65.15秒。记录为 `/tmp/miopunch-172-final/suite.log`。七个失败名称与170的路径/联系/拨号失败相同，另一个为 `membership_revocation_closes_the_original_even_when_path_repair_has_started`：原目标已读到EOF且提供端流已移除，随后调用已撤权对端的 ping 在五秒内尚未结算。171曾失败的 `native_service_parallel::udp_first_reply_keeps_the_started_tcp_round` 本轮通过。上述失败未逐项核实根因，不能据名称直接归为已证明基线。

恢复时长与每通道/节点缓存默认值仍由173依据测量确定；远程终端和最终高铁回放继续由174、175验证。既有文件偏移续传和 UDP 报文入口未改变，复用171仍有效的对应证据。

## 复现

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio172-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test native_service_streams --test forwarding_lifecycle --no-run --jobs 2
```

从 Cargo 回执取得 `native_service_streams` 产物。前三项及 ACK/FIN 用例在 `unshare -Urn --fork --pid --mount-proc --kill-child` 中启用 `lo`，以 `--ignored --exact <name> --nocapture` 运行。最终顺序运行脚本为 `/tmp/miopunch-172-run-real.py`，生命周期复用驱动为 `/tmp/miopunch-172-run-lifecycle.py`。共享 target 后续可重编覆盖，按本提交和回执复现。
