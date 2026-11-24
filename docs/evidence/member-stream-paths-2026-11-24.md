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
| 服务端 / 成功 | 13、17、21 经 A | 25、29 经 B，原 9、13 仍经 A | 37 经 A | `/tmp/mio179-reviewed-phone/` |
| 客户端 / 成功 | 8、12、16 经 A | 20、24 经 B，原 5、8 仍经 A | 28 经 A | `/tmp/mio179-reviewed-client/` |
| 服务端 / 失败 | 9、13、17 经 A | 21 与原 5、9 仍经 A | 29 经 A | `/tmp/mio179-reviewed-failure/` |

三轮在 B 放行或丢弃前均记录了原 QUIC 客户端向 B 发送的 UDP，C 的双端计数为零。成功轮随后开放 B、C，在双端测得 A/B RTT 差小于 5 ms，新增流在 A、B 间分配期间 C 的连续计数仍为零。客户端发起的成功用例在开始计数前给首次 QNT 轮留出完整退避窗口。基线 `d5f8fbd7` 的同一入口在选定 A 后同时向 B 和 C 发送探测，失败记录 `/tmp/mio179-durable-red/`。

两端的终端操作和 TCP 流完全退出后观察六秒。A 的双端 UDP 计数继续增加，B、C 无增量。随后同一监听入口打开 TCP 流，双端实际 STREAM、独立回送和原会话 ID 均匹配。待命修复前的相同行为在六秒内双方 B 各增加六包，记录 `/tmp/mio179-accept-server/`、`/tmp/mio179-accept-reverse/`。

融合端点回归 `old_qnt_retries_do_not_probe_new_balancing_candidates` 从 relay 引导，先在独立 UDP socket 看到旧候选 D 的探测，再放开并确认 A。原 QUIC 上的两条业务流经 A 向接收端交付字节。B、C 的 ADD_ADDRESS 都到达后，独立 socket 确认 D 的旧重试仍在、B 得到探测、C 无探测，原 QUIC 的 REACH_OUT 计数不变。`0e3f9ae0` 的产品代码会让 C 收到探测，日志 `/tmp/mio179-overlap-red.log`。审查修复后同一操作和断言通过，日志 `/tmp/mio179-overlap-green.log`。

审查修复后的业务租约、首条直连、原生 TCP 修复和待命启动四项精确回归通过，日志分别为 `/tmp/mio179-split-lease.log`、`/tmp/mio179-first-direct.log`、`/tmp/mio179-tcp-repair.log`、`/tmp/mio179-standby-start.log`。审查前的 `resources`、`native_service_streams`、`member_standby` 整组运行触及 190 秒期限，日志在 `/tmp/mio179-final-*-suite.log`。

CLI 构建回执 `/tmp/mio179-reviewed-cli-build/receipt.json`（461 Fresh、6 rebuilt），固定产物 `.artifacts/runs/mio-179/pinned/miopunch-client`，对象 `.artifacts/sha256/67/6715cc6e908784f4fca8380121be366eb5ed8b90449802ea77d0022102d1c7c2-555`。Rust 测试回执 `/tmp/mio179-overlap-green-build/receipt.json`（460 Fresh、11 rebuilt）。继续复用 177 的 relay。回收 178 CLI 和两版 179 中间 CLI 的闲置对象与对应链接，共 603,242,496 字节。[验收脚本](../../scripts/check_member_stream_warmup.py) · [MIOPUNCH-179](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/b88f2731-e0f6-46ac-ad4c-3fd0602ff270)

## MIOPUNCH-180 · 分流后的断路和撤权

两组原生 CLI 场景以 A 上的活动终端开始，通过原成员连接的 CLI ping 提供 B 地址，确认两条直连都已就绪后观察 TCP 转发在 B 交付。用双端带负载的 STREAM 帧核对发送方向、流与连接，用独立 TCP socket 核对每条请求和响应的原字节、顺序与 EOF，用远端 shell 文件核对终端输入的顺序和次数。

| 场景 | 原业务 | 后续分配 | 原始记录 |
| --- | --- | --- | --- |
| 双端丢弃 B 的 UDP，保留 A | TCP 流 8 在 B 断开后继续在 A 交付，原终端流 9 留在 A。五次双向转发核对 25,344 / 12,672 字节，两个终端输入各执行一次。 | 断路后流 12 在 A，旧 TCP 流释放后流 16 也在 A。全程同一成员 QUIC。 | `/tmp/mio180-final-cut/` |
| 保留两路，撤销终端服务规则 | 双端终端操作退出，原附件报告结束，新附着请求失败。原 TCP 流 12、16 分别在 B、A 继续双向交付，独立目标没有额外连接。 | A 持有终端和 TCP、B 持有 TCP 时，新流 20 走 B。撤权释放 A 的终端占用后，两路各剩一条 TCP，新流 24 沿既有偏好走 A。全程同一成员 QUIC。 | `/tmp/mio180-final-revoke/` |

复用 179 的原生 CLI 与 177 的 relay 固定产物，此票没有产品源码变更或新增构建。共享夹具的 177 原双直连验收通过，记录 `/tmp/mio180-share-qualify-direct/`。在同一 Rust 测试产物上，原 TCP 通道跨失效 QUIC 双向各重放 262,144 字节，顺序和尾部准确且只接受一次目标；恢复期间撤销 TCP 服务后两端原 socket 结束，授权 ping 仍能交付；178 记录的成员撤销后新 ping 在当前产物上正常返回错误。日志分别为 `/tmp/mio180-isolated-original-inflight.log`、`/tmp/mio180-isolated-revoke-during-recovery.log` 和 `/tmp/mio180-baseline-membership-revocation.log`。[验收脚本](../../scripts/check_member_stream_cleanup.py) · [MIOPUNCH-180](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/8142bf13-0ff0-4f43-b117-79d051899837)

另外，重启后的缓存地址回归在当前固定产物重现 178 的已有失败。SCRAM reset 子场景要求首次 ping 走 relay，实际经已验证的 IP 成功返回，原断言和输出保留于 `/tmp/mio180-ping-reopen.log`。
