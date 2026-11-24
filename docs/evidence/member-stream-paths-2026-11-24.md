# 原生成员共享 QUIC 按流分配验收

2026-11-24，在一次性 Linux 网络命名空间中，用原生 CLI 建立两个已准入成员。终端先在共享成员 QUIC 上收发，再在同一连接上启动并发 TCP 转发。双端分别记录实际编码、解码的 STREAM 帧，按连接、流、发送方向和直连绑定匹配。转发在每个方向交付 2,293,760 字节，发端逐块比较回送字节，服务端核对总量。终端逐条核对 26 次回显，并独立读取远端 shell 写入的有序记录。

| 开放直连 | 实测业务路径（双向） | 本机运行记录 |
| --- | --- | --- |
| A（10.92.0.0/30） | 两条流均走 A | `/tmp/mio177-final-veth-direct/` |
| B（10.93.0.0/30） | 两条流均走 B | `/tmp/mio177-final-veth-relay/` |
| A、B，单向各 4 ms | 已有终端流 12 留在 A（路径 1），新转发流 20 走 B（路径 2） | `/tmp/mio177-final-both/` |
| A 单向 4 ms、B 单向 45 ms | B 已获 ACK，终端流 12 与新转发流 20 均走 A（路径 1） | `/tmp/mio177-final-slow/` |

两路开放的场景均先核对已有终端在 A 收发，再开放 B 并在双端验证近期 ACK 和实测 QUIC RTT。同档时双端 A/B 分别为 9.095/8.796 ms、10.243/8.781 ms。慢 B 时分别为 10.668/75.090 ms、10.570/54.645 ms。

每个目录中的 `stream-paths.json` 记录双端逐流匹配，`result.json` 记录业务校验、引导时终端身份及传输统计。四种条件均从 relay 引导至业务完成保持同一终端、流和成员连接，直连就绪后业务数据走 UDP 直连。启用 `nat-lab-test-hooks` 的 CLI 记录实际 STREAM 的路径、偏移和长度。

在双路夹具中追加一次回归复测。TCP 转发在 B 上交付后只切断 B 的 UDP，原流在 A 上完成双向交付，终端继续在 A 上交付 26 次交互。双端成员连接和流标识保持一致。临时驱动为 `/tmp/mio177-cut-backup.py`，结果为 `/tmp/mio177-final-cut-backup/`。

单条直连时沿用既有连接级换路。原生 TCP 修复测试 `restricted_contact_repairs_the_live_native_forwarding_stream` 通过，切断初始 TCP 路径后，两端实际在替代路径上收发原流并核对应用字节。日志为 `/tmp/mio-177-final-tcp-repair.log`。

在一次性 user/network namespace 用相同产物分别运行 `--case veth-direct`、`--case veth-relay`、`--case both` 和 `--case slow`。 [验收脚本](../../scripts/check_member_stream_paths.py) · [MIOPUNCH-177](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/62278649-a24e-4b69-a3b7-a6f674413199)

本轮 CLI 编译的 Rust 源码和 `Cargo.lock` 对应提交 `529166d7`，构建回执为 `/tmp/mio-177-build-single-route/receipt.json`。两项产物固定在 `.artifacts/runs/mio-177/pinned/`，分别链接到以下不可变对象。

- `miopunch-client` 从当轮共享缓存的 `target/debug/miopunch-client` 固定。对象为 `.artifacts/sha256/3b/3ba0da59090c7bb8760df61316d099c98b973b612ff86c9a3630f2506bfbcbc8-555`。
- `browser-probe-native` 复用开始验收时从 `target/debug/browser-probe-native` 固定的版本。保留固定对象 `.artifacts/sha256/8d/8d5204f6f47363fc1a197a1adf095c2839dce1e2dfecae1a4eb5c9d5506d55d9-555`，原构建回执未保留。

## MIOPUNCH-178 · 业务流持有期和第三条直连

原生 CLI 在一次性 Linux 网络命名空间中准入两个成员。A 先承载原终端与两个 TCP 转发，B 就绪后承载第三个转发。这三条转发保持静默，终端继续收发。全部检查按双端连接、流、方向与带负载的 STREAM 帧匹配；独立 TCP 接收端核对七条有序消息，独立 shell 文件核对四条终端输入。

| 开流前已持有的流 | 新转发流 | 实际路径 |
| --- | --- | --- |
| A 3，B 1 | 29 | 双向 B |
| A 1，B 1 | 33 | 双向 A（既有偏好） |
| A 1，B 1，另外验证 C | 41 | 双向 C，原终端 9 和原转发 25 继续分别走 A、B |

C 是 `10.94.0.0/30` 上额外开放的直连，通过已有地址提示和同一原连接验证。三路都获近期 ACK，双端测得 RTT 分别为 8.745/8.923/8.929 ms 和 8.901/9.063/9.659 ms。全程保持原成员连接。临时驱动 `/tmp/mio178-cli.py`，双端 STREAM 原始日志和结果 `/tmp/mio178-cli-final/`。

融合端点的 `split_path_occupancy_follows_the_business_lease` 单独控制时序。两条 A 的写半流完成 FIN/ACK，其中一条已通过 `into_parts` 拆开，业务租约仍在。基线 `971db45e` 把下一条流送到 A，本次修改送到 B。两个运行时 worker 从同一开始信号独立关掉一条旧流并打开一条新流，观察到两条新流都走 A。维护流仍打开并在 A 实际交付，测试配置的 80 ms QUIC 保活在同一条 A 绑定产生多次双端 PING。释放新流后无须等待 FIN ACK，下一条流按 A/B 平局走 A，两条原流仍各自在旧路径交付。测试核对接收端实际读到的字节和该方向的物理路径。基线 `/tmp/mio178-red-direct3.log`，完整通过 `/tmp/mio178-reviewed-exact.log`。原生 TCP 修复 `restricted_contact_repairs_the_live_native_forwarding_stream` 通过，日志 `/tmp/mio178-tcp-repair.log`。

178 CLI 固定在 `.artifacts/runs/mio-178/pinned/miopunch-client`，构建回执 `/tmp/mio178-cli-build/receipt.json`，审查后测试构建回执 `/tmp/mio178-reviewed-test-build/receipt.json`。178 复用原 relay pin。完成旧 CLI 对照后回收 177 CLI pin 和其对象，共 201,023,488 字节，原日志和收据仍可用。[MIOPUNCH-178](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/629da7d6-6d23-4b6f-992c-1134c976c829)

同批较广回归中，`resources` 为 54 通过、3 失败、10 忽略，`native_service_streams` 为 17 通过、2 失败、33 忽略。失败涉及 UDP 观察中断、增强轮协商、LAN 发布、缓存重启首次连接路径和撤权后重新 ping。撤权 exact 的原 TCP 接收端已得到 EOF，provider 流和 receiver 均已退出，业务租约同步释放。后续 ping 尝试新的 TCP 路径，5 秒内未返回预期的错误。当前 exact 复现，精确 `971db45e` 在同环境通过；定点跟踪观察的业务流始终未分流，未定位后续 ping 差异的原因。原始记录 `/tmp/mio178-resources-suite.log`、`/tmp/mio178-native-services-suite.log`、`/tmp/mio178-revocation-exact.log`、`/tmp/mio178-revocation-971db45e.log`、`/tmp/mio178-revocation-trace.log`。

## MIOPUNCH-179 · 业务先行与第二条直连准备

原生 CLI 在一次性 Linux 网络命名空间中沿用 A、B、C 三路。双端选中已验证的 A 后，夹具延迟 B 的验证报文或持续丢弃 B 的 UDP。原终端保持打开，同时启动 TCP 转发，并发开的流都在放行 B 前完成实际收发。按两端的 QUIC 会话 ID、流 ID 和载荷 STREAM 核对每个发送方向，独立 TCP 服务与远端 shell 文件核对收到的内容。

| 开流端在原 QUIC 的身份 / B 状态 | B 未完成时的新 TCP 流 | 后续新流 | 待命重开 | 原始记录 |
| --- | --- | --- | --- | --- |
| 服务端 / 成功 | 9、13、17 经 A | 21、25 经 B，原 5、9 仍经 A | 33 经 A | `/tmp/mio179-final-phone/` |
| 客户端 / 成功 | 8、12、16 经 A | 20、24 经 B，原 5、8 仍经 A | 28 经 A | `/tmp/mio179-final-provider/` |
| 服务端 / 失败 | 13、17、21 经 A | 25 与原 9、13 仍经 A | 33 经 A | `/tmp/mio179-final-failure/` |

三轮在 B 放行或丢弃前均记录了原 QUIC 客户端向 B 发送的 UDP，C 的双端计数为零。成功轮随后开放 B、C，在双端测得 A/B RTT 差小于 5 ms，新增流在 A、B 间分配期间 C 仍为零。基线 `d5f8fbd7` 的同一入口在选定 A 后同时向 B 和 C 发送探测，失败记录 `/tmp/mio179-durable-red/`。

两端的终端操作和 TCP 流完全退出后观察六秒。A 的双端 UDP 计数继续增加，B、C 无增量。随后同一监听入口打开 TCP 流，双端实际 STREAM、独立回送和原会话 ID 均匹配。待命修复前的相同行为在六秒内双方 B 各增加六包，记录 `/tmp/mio179-accept-server/`、`/tmp/mio179-accept-reverse/`。

最终 CLI 构建回执 `/tmp/mio179-final-cli-build/receipt.json`（462 Fresh、5 rebuilt），固定产物 `.artifacts/runs/mio-179/pinned/miopunch-client`，对象 `.artifacts/sha256/de/de4c809b0f6e6b7fdb510a5936072a8a12aef6a8182b6f4474479c744f9021d5-555`。继续复用 177 的 relay。完成基线及待命差异对照后，回收确认没有宿主进程使用的 178 CLI 和 179 中间 CLI 的各两条链接，共 402,149,376 字节。[验收脚本](../../scripts/check_member_stream_warmup.py) · [MIOPUNCH-179](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/b88f2731-e0f6-46ac-ad4c-3fd0602ff270)
