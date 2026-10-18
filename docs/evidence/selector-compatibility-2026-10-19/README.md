# MIOPUNCH-101 selector 兼容性验收

[任务](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/dfc14b63-5e61-4cd0-804a-efb980e70739)；
[实施说明](../../notes/2026-10-19-selector-compatibility.md)。
基线 `8230c62daa11d6f2f9eae26eb077bf4a8b9dd142`，实施
`d5ee7ac6c993d42aa80e93488157d7ba82a09e43`。本项恢复旧 selector 的 peer
调用及应用契约，新增显式连接级入口；不会关闭 M91 或替代 M69／M84 的
公网／实体设备矩阵，也不在此分类 M102 的 Backup ACK 通用性。

`path_selector` 每次 peer 选路只调用一次旧 selector，不再在 apply 阶段
逐连接重复调用。`connection_path_selector` 注册独立
`ConnectionPathSelector`，只看到当前 original connection 的 live qualified
paths。Native／Browser PathPolicy 接新入口并共用原质量评分。最后一次
显式注册生效；`default_connection_path_selector` 保留任何显式策略。
原 `current()` 仍是地址视图，精确 `is_current` 包含 local socket。新结果及
保留结果在应用前再次检查当前原连接、live path、资源和 socket 所有权。
peer hint 不授予别的连接的 custom、association 或 physical socket 能力。

## 旧消费者原版／失败／修复

固定 [消费者](../../../tests/path_selection_consumers/ordinary_stateful.rs) 在首次
产品修改前完成合格原版基准。它只用原版 API，创建两条同 peer 的真实 QUIC
连接，保持两者打开并实际 PING/PONG，记录回调和最终选路。三个合格比较均
使用完全相同源码；原版为 published iroh 1.2.0 及其固定 lock，fork 使用
仓库固定 graph。清理后的合法回调不计入保持连接打开的记录窗口。

| 运行 | 实际候选数序列 | 最终选路与清理 |
| --- | --- | --- |
| 原版 | `[1, 2]` | 两条连接选中真实 IPv4 路径，正常 close |
| 未修复 fork | `[1, 1, 2, 1, 1]` | 先正常 close，再因额外逐连接调用断言失败 |
| 最终源码 | `[1, 2]` | 两条原连接真实 PING/PONG，正常 close |

首次 v1 夹具未清空默认 IPv6，不能用“两个候选”识别两条连接；该准备记录
保留，未当成合格原版基准或产品 RED。合格 v2 在首次产品修改前，使用原版
`clear_ip_transports` 后只绑定 IPv4；修复时未再修改其源码。

## 受影响验证

| 检查 | 通过数 | 覆盖 |
| --- | ---: | --- |
| 新公开连接级消费者 | 3 | 实际双连接 scope／current／定时重评估；注册与 default 优先级 |
| Physical socket | 12 | 同址双 socket、精确 current、退休结果拒绝、跨连接拒绝、256 监听器、IP pin、丢弃、generation 失效 |
| 统一连接 | 15 | 原连接 fresh ACK、准入／撤销／取消／deadline、真实 RTC answer 退休、健康成员 stream 保留 |
| 共用质量评分 | 6 | 五毫秒 hysteresis、carrier preference、安装状态与 interval 边界 |
| 当前 Noq 协议状态机 | 4 | 同址不同 socket、收包不能接管绑定、迁移使旧 generation 失效、fresh probe 拒绝旧 ACK／closed path |
| 全根测试 typecheck | 通过 | `check --locked --offline --features rtc-probe --tests` |

共 40 项外层测试。FD 限制用例启动一个内部子 harness；初次结果解析取到
内部 1 项，后按最后的外层 12 项摘要纠正，原始解析回执、stdout 和 count
审计一并保留，没有因此重跑或改写测试输出。

新增实际 socket 用例确实观察到地址 current 匹配 2 个候选、精确 current
只匹配 1 个。切换后多次重播退休选择均不能接管路径，随后在同原连接完成
数据交付与 PONG，原／新描述符均释放。Noq 生产代码未改动。

首次统一连接批次 14 通过、1 项在 WebRTC constructor 返回 ENODEV。
独立 OS socket 在同 `network=none` 环境重现 mDNS join 错误，仅加入
`224.0.0.0/4 dev lo` 即恢复；interface flags 未改变。修正后的隔离环境
只有 loopback、没有 default route，完整 15 项用同一源码和 binary 通过。
保留默认 mDNS，未靠关闭 feature 或产品改动绕过失败。

## 完整 Native／Browser SDK

本次冻结 Native 和 Browser 图实际运行受限请求、一次审批、membership 与
admission secret durable 保存、双方相同 persistent identity 重开，以及
member relay stream 返回 PONG。请求约 681 ms 返回 AwaitingApproval。
Native 两次退出均为 code 0；浏览器、context、WebSocket、relay 与容器正常
清理。随后删除生成的两把 TLS key 与私有 NSS fixture。七组实际 source／
lock／tool／artifact 图在 SDK 执行期间保持相同 guard；没有运行中重编译或
扩展图。相同产物仅在实际 Cargo 输出逐字节相同且 pin hash 校验后重用。

[validation.json](validation.json) 保留允许公开的标量、hash 和判定；SDK
原始收据、邀请内容、保存材料和 actor/store 私有日志留在本地，不入库。
二十六份既有原消费者、root lock／profiles 和既有原始 SDK pins 保留；
资源接线除 selector 注册调用外与 M100 相同。249 个预先存在的 WIP 文件
hash 与 dirty set 均未改变，提交后 index 为空，未 push。

[review.md](review.md) 为独立 Standards／Spec 最终双轴审查，各 0 发现；
审查者检查源码与证据，没有重跑检查。
[raw-verification.tar.gz](raw-verification.tar.gz) 包含固定公开消费者、实际原版
source、manifest／lock、编译和运行回执、实际 `.d`、选中输入清单、相关
source copies、失败／环境诊断、controllers 与审查报告。
[archive-manifest.json](archive-manifest.json) 逐成员记录 hash，并已解档读回核对。
完整 immutable binaries／WASM 和无损历史 unit-test 压缩件保留本地路径、
hash 与恢复回执。没有复制 Cargo target、重解析 lock 或改变 profile。
