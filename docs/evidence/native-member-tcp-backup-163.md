# MIOPUNCH-163 已有 TCP 备路的主动回程确认

原生融合成员的原连接健康层将已获准的 TCP 载体纳入同绑定回程确认。首次验证沿用有作用域的 ACK 回程偏好，普通维护 ping 沿用默认回程。UDP 出现既有 PTO／在途 ACK 停滞时，缺少近期回程证据的现存 TCP 可以发出 PATH_CHALLENGE，并用实际绑定上匹配的 PATH_RESPONSE 确认回程。

确认完成后，TCP 在现有证据窗口、绑定、当前资源许可和网络代际内保留资格。健康 relay 先接手，以及 TCP 刚被选中，均不会抹去这份证明。当前路径健康仍由既有 PTO 与在途停滞逻辑决定。TCP 的该分支要求真实确认响应及未被判 suspect，原有 IP／relay 的判据继续使用其既有流程。增强 TCP 的最终双端 Ready 和实际 retain 完成处记录连接、路径与目标，供运行诊断读取。

## 正式业务验收

复用 161 的 `parallel-relay-udp` 双 NAT 拓扑和 NativeClient 转发原流。先经真实 relay 交付业务，UDP 先接管，另一项已经开始的 TCP 在同一连接完成验证和最终保留。网关仅为双方 TCP 增加 40 毫秒单向延迟，使 UDP 主路和 TCP 备路具备稳定的真实 RTT 差异。两侧最终提交后，使用已有 ACK 抑制接口老化 TCP 证据，实际 TCP socket 持续存在，原 UDP 流继续往返。

- 正例在 TCP ACK 年龄超过实际窗口后，通过网关封锁两端 UDP 数据。原流可先通过 relay 继续。原 TCP 路径 1 取得新的 PATH_CHALLENGE／PATH_RESPONSE，随后同一请求流经该现存通道到达独立提供端。拨号端 TCP tx STREAM 从 0 增至 1，提供端同通道 rx STREAM 从 0 增至 1，回复通过对端当时的健康路径返回。恢复耗时约 1382 毫秒。两端连接标识、原流编号和 TCP 本地／对端绑定保持一致，各自仅有一次实际 TCP 观察与互拨。
- 反例沿用相同底座，在 UDP 断流时同时封锁 TCP 回程。拨号端 TCP challenge 从 2 增至 3，response 保持 2，双端 TCP STREAM 均保持 0，TCP 未被选中。原流经健康 relay 在约 1017 毫秒继续往返，两端原连接和流编号保持一致。

原先回环验收确认了 TCP 未进入主动补验的缺口。双 NAT 定点日志进一步复现了 PATH_RESPONSE 已返回，却因 relay 先接手而被排除，以及 TCP 入选后证据因角色变化失效的情况。最终验收保留原有每端独立选路及每笔业务 5 秒期限。

## 证据和复现

最终原生构建回执为 `/tmp/miopunch-163-selected-proof-build/receipt.json`。WASM 回执为 `/tmp/miopunch-163-invariant-wasm/receipt.json`。正负两组目录分别是 `/tmp/miopunch-163-invariant-positive/parallel-relay-udp-positive/` 与 `/tmp/miopunch-163-invariant-blocked/parallel-relay-udp-blocked/`，其中双端日志包含 READY、STALE、FINAL、真实帧统计与实际原流字节断言。驱动结算日志 `/tmp/miopunch-163-invariant-cells.log` 的两组均通过。后续增加的跨端流、物理绑定和单次 TCP 观察核对已直接在这两组保留日志上通过，无运行行为改动。

在一次性 user／mount／network namespace 中，挂载临时 `/run/netns`，运行 `scripts/check_member_parallel_nat.py <native_service_streams executable> <new evidence directory> --case parallel-relay-udp --backup positive`，反例把最后一项改为 `blocked`。源码、依赖和锁文件由现有 Cargo 共享构建锁管理，测试使用已有 `rtc-probe,nat-lab-test-hooks`。

普通端点 2 项、受限端点 19 项与既有原生融合 TCP 6 项通过。一次完整根库单元运行得到 64 通过、3 失败、4 忽略，日志 `/tmp/miopunch-163-full-lib.log`。旧 SDK 的 active budget 全 relay 断言及 RTC 150 毫秒 caller 门限失败在 162 的前置产物上已有同入口证据。新增看到的 first-contact relay 夹具失败在现成 163 前产物 `miopunch_native_probe-226b931e26f19520` 的同宿主同 exact 入口也复现，日志 `/tmp/miopunch-163-firstcontact-pre163.log`。这些断言保持原样，完整仓库全绿尚未成立。
