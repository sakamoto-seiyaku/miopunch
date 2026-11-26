# MioCube IPv6 增强 UDP 原会话验收

MIOPUNCH-202，基线 `d490572f`。按 MIOPUNCH-195 的双栈规格扩展现有增强
UDP owner；实测平台为 Linux，另完成现有 browser-probe 的 WASM 编译检查。

## 行为

IPv4、IPv6 共用原有协商、观察分类、打法选择和执行流程，各自保存实际绑定、
观察、分数、重试进度及本轮预算。实际接口／地址族／UDP 的物理占用在取样前
取得，增强期间仅暂停本族普通 QNT，另一族仍可接收候选、开始新轮和验证路径。
两族共用原有八成员池，同一成员合计一份。

真实 raw winner 以远端 socket、本地 IP、本地 socket ID 保留精确响应许可。
双方 WINNER_READY 收齐即释放物理份，随后由原成员 QUIC 验证、选路并继续原流。
受限 MQTT 联系首拨也使用同一增强执行，只有原连接取得合格数据路径才交付。

IPv4 协调消息的已有编码保留；IPv6 的 Hello 和单族观察快照携带族标识及 IPv6
地址。每个列表仍上限 64，封套允许 4096 字节以容纳同样数量的 IPv6 地址。
打法的发包、临时端口、期限和清理限制沿用，单轮监听仍不超过 256 次。
这套增强协调属于原生成员入口；普通 Noq 消费者默认不安装成员物理门控，
新增暂停状态默认为关闭。Linux／Android 的既有逐包 TTL 发送同时支持 IPv6
hop limit 和源地址；其他平台沿用既有能力与降级报告，未实测其运行行为。

## 真实入口

`tests/support/member_fusion_arbitration.rs` 的四个入口使用真实准入记录、relay、
MQTT、两个独立 IPv6 STUN socket 与成员原流；UDP 故障只安装在独立网络命名空间。

- `ipv6_enhanced_uses_the_original_member_binding_and_stream`：两端从实际 IPv6
  成员绑定各取两个样本。阻断主 UDP 输入并抑制候选 ACK，要求双方先启动真实
  raw 探测。放开输入后双方保留胜出绑定、释放物理份，此时原 QUIC 尚未 ACK；
  恢复 ACK 后选择该精确胜出路径，在同一原流分别读取并回送负载。核对原 stable
  ID、两端单轮发包／端口上限与原期限内终态。
- `restricted_contact_qualifies_its_original_with_enhanced_ipv6`：只提供获准 MQTT
  受限联系，取样前 UDP 不可通。IPv6 增强实际 winner 经过原 QUIC 验证后才交付
  首数据路径，交付的 stable ID 等于准入前认证的 ID；双方原生业务流真实往返。
- `enhanced_ipv6_leaves_late_ipv4_probing_on_the_original_idle_connection` 与
  `enhanced_ipv4_leaves_ipv6_probing_on_the_original_idle_connection`：增强轮运行时
  注入另一族两个真实迟到映射，观察新 REACH_OUT、实际 ACK 和选中路径，原流
  持续双向传输，暂停族的探测仍受阻，候选不被撤回，原轮按原期限释放暂停。

最终 raw 原流入口 5.32 秒通过，日志 `/tmp/miopunch-202-raw-delivery.log`，
固定物 `.artifacts/miopunch-202-delivery.network`；最终编译收据
`/tmp/miopunch-202-delivery-build/receipt.json`（460 Fresh、9 rebuilt）。
受限首数据最终 4.26 秒、IPv6 增强期间迟到 IPv4 10.23 秒、反向 11.72 秒。
日志分别为 `/tmp/miopunch-202-restricted_contact_qualifies_its_original_with_enhanced_ipv6.log`、
`/tmp/miopunch-202-enhanced_ipv6_leaves_late_ipv4_probing_on_the_original_idle_connection.log`、
`/tmp/miopunch-202-enhanced_ipv4_leaves_ipv6_probing_on_the_original_idle_connection.log`。

`tests/legacy_multi_socket.rs::ipv6_nonprimary_winner_retains_the_exact_socket_and_original_connection`
补充真实非主 IPv6 端口：两端各绑定两个额外 socket，逐包 hop limit 7 发出并取得
双向 raw 回包，保留真实 socket 后在原 QUIC 交换业务。另一个同对端连接保持独立
绑定，未认证报文不能改写胜出源地址。完整该目标 14 项通过。

初始 red 使用仅 IPv6 的实际成员绑定，原 IPv4 限定实现在观察入口失败：
`/tmp/miopunch-202-red.log`。首次 IPv6 原流通过见
`/tmp/miopunch-202-v6-first.log`。

## 回归与边界

选定 Native 图一次编译收据为
`/tmp/miopunch-202-native-regression-2/receipt.json`（463 Fresh、11 rebuilt）。
原生完整集合在仅 loopback 的隔离命名空间中运行，日志目录
`/tmp/miopunch-202-regression-runtime/`。其中 legacy_multi_socket 14 通过；
lib 67 通过、1 个旧 active budget 失败、8 忽略；legacy_nat_runtime 32 通过、
2 失败、1 忽略；network_maintenance 17 通过、10 失败、9 忽略；resources
52 通过、6 失败、11 忽略。
这些数字不是全仓绿灯，也不将所有失败视为已确认的旧基线。

默认 wildcard 周期 QAD 需要宿主可用接口，只有 loopback 的完整集合不满足其
环境前提。使用同一已编译产物回到正常宿主，udp_observation 全部 14 项通过，
日志 `/tmp/miopunch-202-udp-observation-host.log`；原 QAD 周期刷新／真实换网单例
保留原 40 秒断言通过，日志
`/tmp/miopunch-202-host-ordinary_qad_refresh_keeps_the_physical_generation_but_real_network_change_invalidates_it.log`。

旧 IPv4 `enhanced_udp_moves_the_original_business_stream` 在完整批量及其仍运行时的
宿主单例均在原 4 秒期限内超时、未观察到 STUN 发送。既有
`.artifacts/miopunch-201-reviewed.resources` 同断言 4.47 秒通过。批量全部结束后，
仅记录原请求结果的当前图两次分别 3.19、3.76 秒通过，原业务、期限及实际四次
取样均保留；产品未为此改动。此处保留时序敏感的未解释超时，不将一次绿灯当
成修复或把旧图一次绿灯当成确定的新产品回归。证据为
`/tmp/miopunch-202-v4-original-host.log`、
`/tmp/miopunch-202-v4-original-201-reviewed.log`、
`/tmp/miopunch-202-v4-diagnostic.log` 和
`/tmp/miopunch-202-v4-diagnostic-second.log`。临时请求日志已移除。

旧 `native_upgrade_reserves_qad_and_both_stun_observers_on_each_eligible_round`
在单独宿主对照仍于第二轮 `LocalBudget` 返回 WouldBlock，原断言未改。
对应旧 NatConnector 与 IPv4 默认动作未由本差异改变；没有当前 201 固定物对照，
因此归因仍未证实，不记作已复现的基线。对照汇总为
`/tmp/miopunch-202-legacy-host-results.json`。其余旧 TCP 重试／换网与待命失败
延续 [201 的关联说明](native-physical-punching-201.md)，交后续相关票整合。

Noq NAT 协议 9 项通过，日志 `/tmp/miopunch-202-noq.log`；
WASM 检查收据 `/tmp/miopunch-202-wasm/receipt.json`（290 Fresh、6 rebuilt）。
独立 Standards／Spec 各完成一次审查；Spec 无发现，Standards 的一项 P3 重复
遍历已改为旧 IPv4 入口委托通用按族判断，经原审查者聚焦复核关闭。
