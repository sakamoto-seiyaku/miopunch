# MIOPUNCH-154 切网与原流恢复

融合端点根据原连接的路径 ACK、实际绑定和当前许可判断能否继续传输。有健康备份时，原业务流继续在获准数据路径上收发。活跃原连接失去可用数据路径时，掌握获准独立联系地址的一端可以交换当前候选。双方同时发起时，已有的原连接维护占用决定谁继续，QUIC 的原拨号端仍负责开路径。受限控制连接通过原 TLS 导出绑定关联到原业务连接。替代路径在原连接上验证，旧流终止后由后续业务请求建立新连接。

## 受控对照

从 `694f004d` 提取当时的源码，沿用共享 Cargo `target/` 和原有 `path_maintenance` 入口。真实 TCP→UDP 换路、关闭后新建连接、独立 MQTT 协调原连接换路、双向物理失联后旧流终止四项通过。旧物理失联用例验证已有独立控制仍存活、原流排队的 65,536 字节失败，并在 16 秒测试窗口内观察到原 QUIC `TimedOut`。该样本的终止耗时 10.686 秒。原始记录在 `/tmp/miopunch-154-baseline-build/`。

同一快照也运行了 demo 的 `sampler_wakes_once_on_a_same_ip_route_change`。原夹具在切换前误把旧维护器的周期发布算成地址变化触发。仅在临时快照将该夹具的 `maintenance.exploration_interval_ms` 设为 60,000，保持原来的操作、断言和 45 秒总期限。先切实际 relay 和 QAD 映射，物理网络代际保持。之后在独立命名空间换默认 veth 路由并保留两边地址，旧代际失效，维护器恰好唤醒一次。记录在 `/tmp/miopunch-154-baseline-sampler/route-switch.log` 和 `/tmp/miopunch-154-baseline-sampler-bounded/route-switch.log`。

融合红灯从实际 NativeClient 转发监听启动，在 TCP 代理断开后，原流未能在固定的五秒业务期限内交付下一段字节。第一版绿灯在约 1.92 秒取得回复。后续精确证据还核对接收侧地址元组相同的旧、新路径分别使用 PathId 0、1，双方 PathId 1 有断路后 ACK 及新增 STREAM 帧。独立 MQTT 控制完成双端确认，原业务连接标识和流标识保持。红灯及首次绿灯在 `/tmp/miopunch-154-red/run-host.log`、`/tmp/miopunch-154-green2/run-host.log`。

| 融合验收 | 实际结果 |
| --- | --- |
| 切断选中的 UDP，备用 relay 仍通 | 复用 MIOPUNCH-147 的 25/350 毫秒 relay 链路及持续写入/只写一次四组。每组在六秒内从原流收到正确序号，选中 relay。UDP 恢复后，原 UDP 绑定只有在收到新鲜 ACK 后再次成为选中路径。首份原流回复在 1.13–3.04 秒。 |
| 真实接口地址变更 | 在独立网络命名空间把 veth 地址从 `10.154.0.1/24` 改为 `10.154.0.2/24`。旧 UDP PathId 1 停用，原流经获准 relay 收发序号 3。新地址替代旧公告后，双方在新物理元组的 PathId 5 收到本次 ACK，并增加原流的 STREAM 帧。两端连接和流标识保持。样本从添加地址到读到再次直连的字节为 6.75 秒。 |
| 仅原拨号端或仅接收端有独立联系地址 | 两组都经获准 MQTT 协调，分开记录控制与原业务连接，双方分别读到 PathId 1、新鲜 ACK 及原流后续序号 2–8。第一份回复分别为 1.55、1.92 秒。 |
| 双方同时发起受限联系 | MQTT 夹具先确认两端各发出一次独立拨号，再放行报文。原接受端的出站占用让行，每端原 QUIC 仅新增一条获选路径，排队的双向序号均送达，约 3.22 秒收到同一流的序号 4。 |
| 首选独立联系静默，第二地址可用 | 在同一已就绪、获准的 MQTT 设施内给出一个旧地址和一个有当前对端订阅的地址。旧实现只发首选 Initial，五秒内无业务回复。新实现对两地址各发一次 Initial，整个受限恢复轮次为 2.182 秒。断路到原流序号 2 回复为 3.326 秒，双方选中新 PathId 后，序号 3 在该路径上增加 STREAM 帧。 |
| 三秒联系期限及所有数据路径耗尽 | MQTT 订阅保持就绪但丢弃受限数据包。尝试在 3,000 毫秒结算，原连接仍存活。切掉 TCP 代理并隔离其反向监听端口，防火墙计数独立记录 10 份回程包被丢弃。旧连接约 10 秒终止，旧转发计数报告 `member_connection_terminated`。恢复可用路径后，新拨在约 0.90 秒收到序号 3，双方读到新连接标识。 |
| 取消与撤权 | 受限联系发出首个 QUIC Initial 后取消最后一项原流业务。实际恢复轮次在样本中 3 毫秒结束，MQTT 夹具按 QUIC Initial 源连接标识统计，后续无新尝试、旧目标无业务字节。撤权组在修复进行期间撤销成员，接收端关闭旧流，后续业务请求被拒。 |

双端原始记录见 `/tmp/miopunch-154-reverse-matrix/` 的 `address-switch.log`、`native-repair.log`、`native-reverse.log`、`native-terminal.log`、`native-cancel.log` 和 `native-revoke.log`。交叉与取消复核在 `/tmp/miopunch-154-review-build/`，147 四组记录在 `/tmp/miopunch-154-matrix-build/carrier-blackhole.log`。备用联系的红绿日志分别为 `/tmp/miopunch-154-fallback-red/native-fallback.log` 和 `/tmp/miopunch-154-fallback-final/native-fallback.log`，相关复测记录在 `/tmp/miopunch-154-fallback-green/`。

普通端点 2 项、受限端点 19 项通过。现有受保护恢复入口另有 6 项通过，包括原接收侧发起、双端确认、实际断路、旧请求线格式及调用期限。WASM 使用上票同一条 Cargo 命令，仍有两项基线错误，`connection_coordinator.rs:1043` 引用了未链接的 `noq_proto`，`browser-probe/src/member_connection.rs:145` 少传 `seal_hello`。回执在 `/tmp/miopunch-154-fallback-wasm/receipt.json`。

## 2026-11-24 换路回归复测

当旧路径失去资格、替代路径仍在验证时，选路可能暂时为空。原实现此时丢失旧路径标识，使未确认的 STREAM 帧无法在替代路径成为可用后移交。修复保留该标识，在选中新承载后将待确认帧交回发送队列。宿主原流往返实测约 0.58 秒，原五秒断言保持。

原有七项失败均通过复验。四项需控制直连备路的用例在独立命名空间运行，先制造故障再登记 MQTT 联系，避免在计数基线前起拨。宿主整文件 19 通过、0 失败、33 跳过。UDP 黑洞四种组合和恢复到期先读尽确认字节的回归也通过。原始记录在 `/tmp/mio-repair-fix.LXoLzs/`。 [HSRNet 全量复测](hsrnet-terminal-replay-2026-11-23.md#2026-11-24-全量复测)

## 复现入口

```sh
rtk proxy python3 scripts/cargo_cache.py run --output /tmp/mio154-build -- \
  test --locked --offline -p miopunch-native-probe \
  --features member-signaling-test-hooks,nat-lab-test-hooks \
  --test native_service_streams --test resources --test path_maintenance --no-run --jobs 2
```

从 Cargo 回执取得对应测试产物。`native_service_streams` 的拨号端原流修复与撤权组使用宿主回环。接收端修复、静默首选、双端联系和取消组在一次性 Linux 网络命名空间内封锁直连 TCP，确认独立联系已起拨后按场景解封。启用 `lo` 后用 `--ignored --exact` 运行这四组及 `exhausted_native_path_reports_terminal_before_the_next_business_reply`。`resources` 的 `member_fusion::udp::changed_interface_address_revalidates_the_original_udp_stream` 也使用 `--ignored`，在同类命名空间为一对交换 veth 设置下列起始拓扑。

```sh
ip link set lo up
ip link add mio154a type veth peer name mio154b
ip link set mio154a up
ip link set mio154b up
ip addr add 10.154.0.1/24 dev mio154a
ip route add default dev mio154a
```

## 现场记录

| 记录 | 版本、变化及首份回复 |
| --- | --- |
| 2026-11-18 历史实测 | 两端 `9d3db16c`。`u22sz` 在台北时间 15:55:55 离开办公网 `172.31.0.36`，15:56:47 获得热点地址 `10.108.137.75`，15:58:44 回到办公网。首次 `business-restored` 分别为 `r_proxy` 15:58:38.117 和 `u22sz` 15:58:54.901，放在各自新的连接计数上。状态按秒采样，单笔回复实际走哪条路径未知。 |
| MIOPUNCH-141 | 截至 154 验收，工单仍在 Backlog。11 月 19 日固定网络读取记录了部署产物和首份有效回复，141 所需的新一轮人工接口切换、当时两端版本和切换后首份回复尚未观测。 |

家宽 WAN 缺少同轮抓包。旧版中 `u22sz` 发出匹配的 UDP 响应，本地记录可核对目标及发送 socket。该响应是否到达家宽 WAN、实际公网回程源端口，以及为何 Linux 未收到匹配响应仍未知。[历史切网日志](network-demo-switch-2026-11-18/README.md) · [现场时序与回程复盘](../notes/2026-11-18-network-switch-review.md) · [固定网络验收及待执行的 141](network-demo-140-2026-11-18.md)
