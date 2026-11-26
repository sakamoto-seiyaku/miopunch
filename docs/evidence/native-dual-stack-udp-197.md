# MioCube 双栈普通 UDP 验收

实施票为 MIOPUNCH-197，父规格为 MIOPUNCH-195。实施基线为
`a81cce85`，主体提交为 `94e43245`。本轮在 Linux 宿主的隔离网络中验收。

## 已交付行为

原生成员的冷地址可并行起拨，任一合格路径即可交付。普通 QNT 分别记录两族轮次，
增强 IPv4 暂停期间可以发布新 IPv6 候选并启动 IPv6 普通轮次。单族轮次保留另一族
其余探测与 challenge。普通库的原有整轮 API 和默认严格候选上限继续适用。

成员候选按实际协商容量分配两族窗口，并在原重试间隔内递补超额地址。替换先退旧项，
帧发送先 REMOVE_ADDRESS 后 ADD_ADDRESS。成员接收端对重排的满窗更新保留固定容量，
淘汰的序号不会被重传复活。服务器为后来一族提供名额时保留另一族其余进度。

选路仍调用现有 biased RTT selector，沿用两族同档、IPv6 3 ms 偏好和同档 5 ms
防抖。当前连接的路径资格仍按实际 socket、local/remote tuple 和授权检查。

## 真实入口

- `native_client::dual_stack_tests::first_member_path_is_independent_of_the_other_address_family`
  从真实 NativeClient 完成 IPv4、IPv6、双栈、IPv4 黑洞、IPv6 黑洞五场景。
  首次 PONG 分别为 486、516、887、553、546 ms，两端随后独立 PING/PONG 并复用会话。
  日志为 `/tmp/miopunch-197-first-path.log`。
- `scripts/check_native_udp_dual_stack.sh` 使用两个独立 namespace 的 IPv6 conntrack
  入站过滤，首先阻断 IPv6，确认两端已在共享 relay 会话收到 PONG。每端加入 40 个
  不可达 IPv4 和 40 个不可达 IPv6 候选，随后加入唯一可达的新 IPv6 前缀。
  最终两端选中新 IPv6 路径并收到 PONG，stable ID 与 relay 阶段一致。
  修复后的完整运行在 `/tmp/miopunch-197-filtered-quota-fixed`，两端日志及带计数的
  防火墙规则同目录保留。普通 QNT 的发起端实际发送 82 个 REACH_OUT 帧。
- `member_fusion_arbitration::enhanced_ipv4_leaves_ipv6_probing_on_the_original_idle_connection`
  先在 relay 上打开业务流，双端由现有 STUN 取得真实映射。确认同一轮增强 report
  两端实际探测计数均大于零。新 IPv6 映射的转发尚未启动时，QNT REACH_OUT 已增加，
  两端 IPv4 仍暂停且普通 IPv4 sink 没有包。随后开启两个真实 UDP socket 的端口映射，
  两端在原会话选中新映射并取得 ACK。同一业务流前、中、后发送不同载荷并在对端匹配，
  stable ID 保持一致。暂停释放受原 report.deadline 加 100 ms 调度容差约束，round 保持一致。
  最终日志为 `/tmp/miopunch-197-overlap-input.log`，结果为 1 passed。
  固定产物为 `.artifacts/miopunch-197-overlap-input.maintenance`。

该重叠场景的 IPv4 primary 故障精确匹配 loopback 上的双端地址和端口，并在 INPUT
丢弃，允许真实发送及 STUN/sink。新 IPv6 候选使用端口映射，接口保持稳定，因此本用例
验证候选抵达，物理换网及按族取消仍由 MIOPUNCH-205 验收。

## 回归与构建

- Noq 现有 NAT traversal 集合加两项回归共 6 passed，覆盖暂停的旧族满窗后两个新族
  候选均可探测、公告更新的重排/重传、默认消费者严格上限。
  `/tmp/miopunch-197-proto-results.log`。
- Noq multipath 完整模块 68 passed。`/tmp/miopunch-197-proto-multipath.log`。
- connection_path_selection 完整集合 3 passed，unified_path_selection 完整集合
  6 passed。日志为 `/tmp/miopunch-197-connection_path_selection.log` 和
  `/tmp/miopunch-197-unified_path_selection.log`。
- contact_member_priority 的 7 项有通过证据。旧入口的一项在默认测试线程栈上溢出，
  同一产物使用 `RUST_MIN_STACK=8388608` 后通过。具体日志为
  `/tmp/miopunch-197-contact-stack.log` 及 `/tmp/miopunch-197-contact-*.log`。
- network_maintenance 常规集合最初为 19 passed、8 failed、5 ignored。
  其中 6 项缺少测试要求的 MQTT broker 前置。复用仓库 MQTT 镜像补齐后，8 项精确复跑中
  6 项通过。`/tmp/miopunch-197-maintenance-broker/results.json`。
- 原生 library check 通过。共享源码的 WASM browser library check 通过，收据为
  `/tmp/miopunch-197-check/receipt.json`、`/tmp/miopunch-197-wasm-check/receipt.json`。
  WASM check 为 291 Fresh、5 rebuilt，最后单一重叠 fixture 构建为 468 Fresh、1 rebuilt。
  构建均使用共享 target、已锁定依赖和离线模式。

## 已知限制

两项维护测试在相同权限、相同 broker 前置、相同 fixture 的 `a81cce85` 基线也失败。
分别为 `stale_saved_contact_expands_to_discovery_and_recovers_an_unknown_member` 和
`original_application_members_retry_a_failed_tcp_observation_after_network_change`。
基线比较结果在 `/tmp/miopunch-197-baseline/runtime/results.json`。六个生产文件在构建后
由 finally 恢复，比较使用固定产物。

`fused_member_forwarding_reports_unavailable_without_a_data_path` 实际拒绝业务，返回
TimedOut，旧断言要求 NotConnected。其 options 仅含 restricted 联系，没有 IP 发现或
冷地址，接收端禁用 UDP，轨迹未出现数据路径放行。本票的冷地址并行及普通成员 QNT
数据阶段未进入该负例。日志为
`/tmp/miopunch-197-contact-fused_member_forwarding_reports_unavailable_without_a_data_path.log`。
因此此处不宣称全部旧回归绿色。其他旧基线失败继续参考 196 的证据。

本轮未测试物理设备及非 Linux 系统，没有改变线帧格式。跨平台真实连接仍沿用各票的
可用宿主验收边界。

## 独立审查

第 1 轮 Standards 和 Spec 各确认一项，均指向服务器满额跨族窗口的同一根因。
合并修复后两轴聚焦复核通过。Spec 还只读核对最终重叠 fixture 及实际通过日志，确认
实探、两族重叠、原业务流、同一轮固定期限均有断言。没有未处理的审查发现。
