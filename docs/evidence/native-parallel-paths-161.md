# MIOPUNCH-161 同轮 TCP 与 UDP 找路

受限首拨在同一原 QUIC 连接上预留 TCP、UDP 两个内部控制交换，并行执行各自的现有观察、协调和路径验证。第一份合格的数据路径许可即可继续成员准入和业务交付。另一项已启动的工作保留到完成、失败、原期限、取消、换网或连接关闭。应用公开控制交换仍在业务交付时退休。原生融合 ALPN 为 `/5`，用于区分两个固定交换的布局。

TCP 候选在受限阶段和已交付的融合成员连接上复用同一条连接拥有的候选入口。它保留实际监听端口、观察、互拨、双端确认和新鲜 ACK 条件。待提交的 TCP 载体继续由现有 PendingTcpPath 门控。已有 relay 上，TCP 的节点名额独立于普通候选验证，已开始的工作保留原来的业务释放取消条件。

UDP 首次合格的前台工作不再固定等待一个质量维护间隔；后续轮次沿用原来的质量间隔、预算和冷却。受保护 OBSERVE 交换已经承诺的轮次继续执行，QNT 更新是否需要新轮的提示不会撤销这项在途工作。新轮的发起与接收仍检查相应资格。双方取得路径验证后，自动改善可以持有尚未被评分选中的 UDP 备路。业务学习仍要求既有实际胜出路径与业务证据。已有健康合格 TCP 时停止新 UDP 找路，已开始的轮次自然收尾。

融合选路使用 iroh 的分档、RTT、健康过滤和当前路径防抖。TCP 地址按实际 IP 族取得相同的默认偏置，其他排序条件相同时优先 TCP。当前路径的五毫秒防抖继续生效。普通库的原选路接口保留。

## 正式入口结果

- 最初的 NativeClient 红测在 TCP 观察被挂起时等待不到 UDP 首笔业务。并发实现先经 UDP 读回业务，放开 TCP 观察后在同一连接、同一流上取得 TCP ACK。日志分别为 `/tmp/miopunch-161-parallel-red.log` 和 `/tmp/miopunch-161-parallel-green.log`。
- 两个真实 NAT 网关后的仅信令两组通过，分别以 TCP 和 UDP 开始业务，然后保留两个已验证路径并持续收发同一业务流。默认参数日志为 `/tmp/miopunch-161-default-parallel-lab/parallel-tcp/` 和 `parallel-udp/`。
- 相同双 NAT 拓扑的健康 relay 两组通过，首笔业务经 relay 返回；两端开始实际 UDP 观察及 TCP 互拨后，先放行其中一路，再放行慢路。原业务在探测期间持续，两个实际路径均取得新鲜 ACK，并保留相同连接与流。默认参数日志为 `/tmp/miopunch-161-default-relay-fixed/`。
- relay 诊断复现了已承诺 OBSERVE 后重复检查 QNT needed 的竞态。一端已观察，另一端因 needed 刷新返回 WouldBlock，随后被已消费的冷却阻挡。失败定点日志为 `/tmp/miopunch-161-coordination-diagnosis2/`。修复只移除了承诺后的重复资格检查，诊断输出已移除。
- 确定性选路 6 项通过，包括 IPv4/IPv6 等价地址、RTT 差异、TCP 平手、显式偏好、不可读路径统计和当前路径防抖。日志为 `/tmp/miopunch-161-unified_path_selection.log`。
- 普通消费者 2 项、受限端点 19 项通过。既有 TCP 原流迁移 6 项、正式维护宿主 TCP 迁移及业务取消后候选退役通过。日志为 `/tmp/miopunch-161-existing-tcp.log`、`existing-traversal.log`、`cancel-traversal.log` 及同前缀的端点日志。
- UDP 混合观察定向批次 4 项通过。停止观察的 Network 子场景在既有断言处失败；同 host、同 exact 入口的 161 前 `resources-fdce46ac5c8ca148` 产物也在同一行失败。旧产物时间为 2026-11-21 10:36。对照为 `/tmp/miopunch-161-interrupted-pre161.log`，当前为 `interrupted-exact.log`。该夹具没有前台业务，既有换网后的 standby 自动轮会再次采样，测试源码与核心断言保留。
- WASM 检查通过，回执为 `/tmp/miopunch-161-wasm-check/receipt.json`。本轮原生最终行为构建回执为 `/tmp/miopunch-161-coordination-fix-build/receipt.json`。构建使用共享 target 与锁，463 项 Fresh、8 项重建。

双 NAT 驱动是 `scripts/check_member_parallel_nat.py`，正式业务工人是 `tests/support/native_service_parallel.rs`，复用已有成员准入、信令、观察和拓扑。场景不要求切断先胜路径，原流故障恢复归后续子票。159 已记录的完整 root 运行、raw allowlist 与默认栈限制继续保留，本轮没有把定向结果扩称为完整套件全绿。
