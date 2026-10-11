# 当前 SDK 连接主线：有限代表用例与验收审计

用户本轮选择先运行少量当前 SDK 代表故事，再按固定基线
`29b5a2ceec62b37d6870add3744045a15cd858bb` 独立审查并收拢 M84/M69。
本轮没有修改 Rust 产品实现、选路默认值、锁图或设备网络配置。
预算修正只涉及探针：Native 以实际保存策略为准，Browser relay 显式 60 秒；
Browser pair 可选择 12/60 秒，原 CLI 默认仍为 12 秒。

## 实际完成的当前代表用例

| 用例 | 实际结果与计时边界 | 资格范围 |
| --- | --- | --- |
| [公共 relay，RTC Answer API 缺失](public-relay-no-rtc.json) | 真实准入、一次审批、材料保存、双方重开；没有获准 data relay 时返回 DataPathUnavailable 且无普通 handle，恢复 APS1 后双方成员 gate、fresh ACK、PING/PONG；完整驱动 42.382 秒 | Host Native / Pixel Browser，公共 TLS MQTT 与获准 data relay；双方实际 60 秒 caller |
| [MQTT/RTC 两浏览器双向](mqtt-rtc-both.json) | Host→Pixel connect/accept 10.949/11.088 秒；Pixel→Host 8.322/8.308 秒；真实普通 handle、双向业务、原 M、初始及后续 fresh ACK、材料重开 | 公共 EMQX 信令，Host / Pixel USB 浏览器；实际双方 caller 60 秒；未排除覆盖网络 |
| [relay 转 RTC](relay-improve.json) | 初次 open 242.480 ms；同 M improve 716.750 ms；原 binding/stream/PONG 与新的 fresh ACK 保留 | 自有 loopback data relay、公共 MQTT、同机 RTC；PreferDirect 仅为对照策略，产品仍为 Quality |
| [实际 carrier 中断与修复](carrier-repair.json) | 关闭唯一真实 PC，原 relay 已退休、备用路径为零；recover 576.620 ms；返回后 3.111 秒仍在原 M/binding/stream 收到业务 | 同机真实 RTC carrier 故障，不是模拟回调，也不是独立物理 WAN |
| [RTC 无路](rtc-no-path.json) | 真实新增 PC/DC；原 60 秒 caller 于 60.003 秒返回 DeadlineExceeded；对象已退休，原 relay/M/binding/stream 与 PONG 仍正常 | 自有不可达 RTC 配置、健康自有 data relay；没有把信令成功当成新路径成功 |
| [联系入口回退](entry-fallback.json) | 三个真实 Native SDK 成员，撤回登记并关闭成员转发链接，回退 MQTT 联系控制后选 UDP 业务；caller 523 ms、cleanup 152 ms，保留原 12 秒绝对 deadline | 自有本地 broker；MQTT 不承载普通业务；actualMemberUnavailableObserved=false 原样保留 |
| [复用已合格的 IRC 当前构件](irc-reused-current-sdk.json) | M89 最终完整公网故事：purpose2 请求 43.081 秒，真实审批/材料保存/双方重开/获准 relay member1 fresh ACK 与 PING/PONG | 源码与构件重核验通过，未重跑未变测试；60 秒 caller、8 秒 assembly、30 秒 queue 是比较配置，完整默认 profile 未获资格 |

计时属于不同设施和不同调用，不用于性能排名或速度倍率。
完整驱动 wall-clock 包括设置、审批、重开、观察及清理，不能替代 SDK caller。
原始命令、退出码、phase 和独立清理计时见各 JSON；原始私有 DTO 留在
回执标明的位置，仅公开标量、枚举和 hash。

## 首失败和观察边界

[保留的失败](retained-failures.json) 包含构件选择 guard、路径准备、旧预算断言、
MQTT 启动超时和 observer 超时。第一次浏览器 12 秒 caller 超时，后续显式
60 秒执行的实际调用却都小于 12 秒，所以原因仍未知，不能声称延长预算
解决了网络或产品缺陷。Carrier 首次 MQTT 启动失败发生在 candidate 之前，
未确定 DNS/TCP/TLS/CONNACK 子阶段；一次未改源码重试通过。
入口回退只认证撤回登记并关闭成员链接后的回退；没有观察到向残留已登记
入口发送失败，不能把 `actualMemberUnavailableObserved=false` 换成 true。

通用 `check_unified_maintained.py` 的 page RPC 20 秒 observer 先于 SDK 原
60 秒 caller 到期。本轮沿用既有 M84 专用 runner 的 75 秒 improve observer，
保留通用 runner 失败；没有声称修复了所有正式 observer 入口。

清理的 `qualified` 是各 runner 的有限范围判定。最终公共 relay 中，100 ms
晚期观察仍有两个 CLOSING WebSocket、四个 relay SDK callback，
`allClosedObserved=false`、`allSdkTerminalTasksRetiredProven=false`。
Carrier repair 和 RTC 无路仍各观察到一个 MQTT 终止 socket 尾部，全部终止
任务退出同样未完整证明。这些原值保留在公开 JSON，不能用 tab 消失、Native
进程退出或 IRC 的自然关闭资格覆盖。成员 handle、RTC PC/DC、pump、队列、
listener 和本票拥有的转发/进程按各自记录回收；完整终止关闭仍是 M84 缺口。

## M84 完整验收审计

| 原验收要求 | 本轮核对结果 |
| --- | --- |
| 四类端点及反向；成员 gate、新 wire、fresh ACK、业务；准入/重开/非空 replica | 当前 Browser↔Browser、Browser→Native 与 Native→Native 联系回退已实跑；全方向当前实机矩阵与当前非空 replica 未完成。旧八用例及 OP13 记录只属原构件时期 |
| 实际 carrier 中断、RTC 无路、入口故障；保留失败及四层资格 | 三类窄故障均已实际完成；所有失败和未达物理 WAN 资格保留 |
| 一次一小例、分离编译/网络/协议/验收/cleanup exit | 顺序执行，每例前后 guard；原运行记录单独保留 SDK 调用、phase、退出和清理计时 |
| 实际 SDK/入口、命令/hash/角色/路径/caller/cleanup；有限诊断 | 当前 Native 新构建一次、WASM 原合格构件复用；入口回退单独当前测试构建；没有扩成旧全面 NAT 测试 |
| 本票资源、即时/晚期 socket；不得把强制结束称 graceful；公开无秘密 | scoped 回收通过，部分终止 socket/task 尾部仍未完全退休，不能认证完整 graceful；仅公开标量/hash |
| 正式入口新协议、移除内部旧 callers/wire 分支、受影响短回归 | 先前 contract 清理为历史实现证据；本轮新 wire/gate 已实跑并核验未变源码，但未重新认证所有正式入口/完整当前物理矩阵；没有把旧记录换成当前资格 |

因此用户本轮有限执行与审查范围可收拢，M84/M69 仍保持进行中。
完整无 VPN 异网数据路径、当前所有方向/非空副本、完整默认 IRC profile 及
终止 socket/task 关闭资格继续由 Plane 跟踪。

## 构建、复用与证据保全

[构建回执](builds.json) 中 Native 本轮 39.31 秒，520 Fresh / 9 rebuilt，
6579 实际输入、292 repository copies、478 depfiles；批次含 provisioner 和
maintained 两个 binary。Provisioner metadata 只移除 sibling maintained 的
一个 leaf 和对应 depfile，实际输入 6578；编译计数仍属于原批次。
WASM 复用 M89 v4：3159 输入、198 copies、264 depfiles；没有额外构建。
入口回退构建 58.57 秒、523 Fresh / 20 rebuilt，6642 输入、315 copies、
491 depfiles；该命令编译的其他目标计入批次，不能说只重建了单个测试。
原始 depfiles、源码副本和不可变 artifacts 位于原回执目录；完整 hash 图另附。

`target/`、profile、锁图和工具链保持不变。仅在 host 进程审计、basename/SHA
公开引用豁免及精确 hash/link 检查后删除五个无引用旧测试缓存，释放
820740096 字节；[审查](additional-obsolete-tests-review.json)与
[删除回执](additional-obsolete-tests-removal.json)保留。没有删除库、depfiles、
SDK pins、正常 binary 或 toolchain。
[234 份既有 WIP](protected-wip.json)保持 byte-identical。

[受影响短检查](focused-checks.json)：三项 Native 预算证据测试、十八个拒绝
反例、Python AST、JavaScript syntax 与 diff 检查通过；没有重复未变 Rust 测试。
实际执行探针副本及 hash 见 [manifest](probe-source-manifest.json)。

独立 [Standards 源码审查](reviews/standards-source-report.md)及
[最终证据审查](reviews/standards-final-evidence-report.md)无未解决发现。
独立 [Spec 源码审查](reviews/spec-source-report.md)及
[最终证据审查](reviews/spec-final-evidence-report.md)没有需要修正的源码或证据
问题；[完整验收审计](reviews/acceptance-audit.md)保留上述三类整票缺口。
[host 进程审计](final-host-process-audit.json)只证明本轮 runtime wrapper 与仓库
compiler 已退出，不代替 Browser SDK 终止任务的自然退休证明。
