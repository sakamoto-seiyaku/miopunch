# MioCube 按实际出口保留打洞占用

MIOPUNCH-201，基线 `8e5302e2`。范围为已有 IPv4 TCP／UDP 成员入口；
后续双栈扩展属于 MIOPUNCH-202。验收在 Linux 宿主及独立网络命名空间运行。

## 交付行为

融合端点按实际本地接口、地址族、TCP／UDP 保存物理打洞 owner。同一成员的
并行引用共用 owner；MIOPUNCH-200 的八成员建路位置继续独立计数。
实际路由选择不发包；STUN／QAD、原始探测和保留胜出绑定使用同一本地源。
数值 STUN URL 不经过 DNS，loopback 别名归到真实 loopback 接口前缀。

普通 QNT 在实际目的发送前取物理份，受阻时保留重试额度及未发响应。
已有验证路径正常收发；32 项响应队列中的受阻项不挡住健康路径、其他出口
或另一地址族。双端原始打洞已保留的胜出绑定另以精确远端 socket、本地 IP、
本地 socket ID 登记临时响应许可；在发送本端 WINNER_READY 前登记，双方
READY 收齐才释放物理份。后续 QUIC 验证响应不重新争抢占用，失败、取消或
现有验证／保留流程结束时 RAII 撤销。额外门控为融合端点显式安装，普通 Noq
消费者默认行为沿用。

增强 UDP 从取样前占用到双端 WINNER_READY 并保留胜出绑定即释放；之后
原 QUIC 的 ACK 验证、选路和原流继续。下一成员的合法观察不会使已保留
胜出绑定因旧观察 token 被替换而失效。移除跨成员固定冷却，保留同成员
实际发起工作后的重试节奏及原打法包数、端口、期限与清理。

TCP 观察与互拨使用相同本地源／监听端口，实际 TCP socket 和 preface
握手完成即释放物理份。原 QUIC 验证继续由实际 circuit、网络代次、路径许可
保护。首拨遇未取样的忙状态会释放就绪端并随原池通知重试；取消、结束及
原绝对期限约束整个尝试。观察站解析／路由失败可在剩余期限内递补后续站点。

## 真实入口与证据

四成员入口 `member_fusion_arbitration::busy_peer_releases_the_unsampled_exit_for_another_member`
先令 B–C 保留未取样份：A–B 不新增任何 STUN，A–D 可以完成实际取样。
再阻断 A–D 的主 UDP 输入：双方已开始真实探测时 A–B 仍须等待且观察不增。
放开后，两端保留双向胜出绑定；抑制原 QUIC 返回 ACK 时 A–B 已能实际各取
两个样本。随后恢复 ACK，两端选择原胜出路径，并在原 stable ID、原业务流
独立接收并回送数据。首次完整隔离通过日志 `/tmp/miopunch-201-raw-final.log`
（12.39 秒），核心断言未放宽。最终同入口额外带首个不可用地址族观察站，
验证它不抑制后续 IPv4 站点。最终原入口完整通过日志
`/tmp/miopunch-201-one-mask-final.log`（6.23 秒）；保留原 25 秒总限及所有
排斥、NEXT-before-ACK、同连接／同业务流断言。故障前等已有普通 QNT
结算，复用现有路径 mask 一次声明真实 relay／primary，整个输入故障保持
到双端实探；没有叠加尝试放宽只能收紧的 Noq mask。

实际 TCP 观察、握手、原流切路通过日志 `/tmp/miopunch-201-tcp-first.log`
（6.16 秒）。最终 Commit 入口同样增加首个不可用观察站，保持真实本地源
端口及原业务断言，日志 `/tmp/miopunch-201-reviewed-tcp.log`（6.63 秒）。
首次完整四成员早释放日志为
`/tmp/miopunch-201-selected-entry.log`。

旧晚 IPv6 验收保留原断言、使用同一隔离入口通过，日志
`/tmp/miopunch-201-late-ipv6.log`（12.15 秒）；旧双端 IPv4 STUN 前置原先
被数值 URL 的无 DNS 解析问题挡住。两物理路径原流用例在真实 loopback
别名归属修复后通过，日志 `/tmp/miopunch-201-split-fix.log`（6.02 秒）。

Noq 原 NAT challenge／response 用例补充：物理 gate 拒绝时不发送离路径
响应或 reciprocal challenge，恢复后保留的响应继续并完成双端建路。
响应队列另检查受阻 IPv4 队尾不阻塞既有路径或允许的 IPv6 响应。
响应保留时，发送就绪只计算当前 tuple 的响应，避免纯 PADDING 循环。
9 项 NAT 协议检查通过，日志 `/tmp/miopunch-201-response-scheduling-protocol.log`；
队列检查为 `/tmp/miopunch-201-reviewed-response-queue.log`。
MIOPUNCH-200 的普通 QNT 真实 ACK 位置入口仍通过，日志
`/tmp/miopunch-201-reviewed-qnt.log`（6.01 秒）。最终 raw-winner 精确响应
许可由上述四成员原入口实际验证。

## 回归、审查与边界

一次完整选定回归（修复审查问题前）保留原失败：lib 66 通过、2 失败、8 忽略；
resources 50 通过、8 失败、11 忽略；arbitration 4 通过、1 失败、2 忽略；
实际应用 TCP 集合 3 通过、3 失败。日志分别为
`/tmp/miopunch-201-{lib,resources,arbitration,tcp}-regression.log`。
新增的 resources 两路径失败已以上述定向真实入口闭合。

lib 两项、资源可达性／mDNS／旧增强观察分类与顺序失败沿用
[200 的基线说明](native-member-building-200.md)。TCP 两个 line 484 尾计数
沿用 198 证据交 203；换网后 TCP 观察 line 323 与 197 的 a81 基线、198
复验相同，日志 `/tmp/miopunch-201-tcp-network-fix.log`，由主线程确认交后续
重试／换网票。arbitration 的待命换网唤醒 line 546 保留交后续票。

独立 Standards／Spec 审查发现的观察站递补、原期限、离路径响应门控、
TCP 握手早释放及首拨忙重试均经定向修复与原审查者只读聚焦复核。
两个 P3 重复代码建议经主线程决定留待实际维护需要，不改变交付行为。
Native 与 WASM 沿用共享构建缓存、锁文件和既定 profile；未实测其他宿主平台。

最终 Native 生产构建收据为
`/tmp/miopunch-201-retained-winner-native-3/receipt.json`（9 rebuilt）；
最后夹具修正仅重编 network 目标，收据
`/tmp/miopunch-201-one-mask-native/receipt.json`（468 Fresh、1 rebuilt）。
最终真实入口固定物 `.artifacts/miopunch-201-final.network` 用于后续精确复现。
WASM 响应调度检查收据 `/tmp/miopunch-201-final-wasm/receipt.json`；
按主线程决定复用其未触发的默认路径证据，raw-winner 新许可由 Native 验证。
