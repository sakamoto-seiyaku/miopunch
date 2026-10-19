# Backup ACK 兼容性验收

2026-10-19，[MIOPUNCH-102](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/45c5771d-0337-44b3-97cb-89e788874db7)
依据 [ADR 0012](../../adr/0012-preserve-library-contracts-and-defaults.md)，采用
[显式 probe 的绑定级 ACK 契约](../../notes/2026-10-19-backup-ack-contract.md)。
实施提交 `5aa261890cd2a5e81ecab998e10ef0758a5080cc`；基线 `3be145a09bf8847d767e101a9b8c4a8642bf1070`。

## 结论

同一固定普通消费者在原版与 fork 上的实际结果否定了“通用缺陷修复”的分类。
普通 Noq 入口恢复跨路径 ACK；fork 既有 start_path_probe／Path::start_probe
显式选择本地绑定的 Backup ACK 经本路径返回。双方各自选择可在旧 Available
主路黑洞时取得备路 fresh ACK；慢备路的 ACK 延迟代价由显式选择承担。

选中偏好在绑定替换、关闭及本地 IP 改变或清空时重置；重复未知地址通知也
重置，相同已知地址通知保留。没有放宽普通数据权限、修改认证证据算法、
更换拥塞控制或改写连接关闭行为。旧 Available 主路未恢复时的关闭路由边界
仍存在，不能把备路 ACK 当成对端已收到关闭；恢复／退休及实际维护流程另验。

| 受控回环业务确认 | 原版 | 修改前 fork | 最终普通 fork |
| --- | ---: | ---: | ---: |
| B 返回方向延迟 250 ms | 约 1 ms | 约 253 ms | 约 1–2 ms |
| 同上加一次实际 UDP 丢失 | 约 29 ms | 约 532 ms | 约 29 ms |

两端实际 TLS／流数据、SendStream 确认完成、ACK carrier、主路黑洞和恢复、
关闭窗口以及全部任务 join／端口重绑均由原始回执保存。它们是回环诊断结果，
不承诺公网性能。原版 published Noq 1.3.0 的默认 features 和锁文件保留；
fork 只切换本地 NoqProto 依赖行，固定源码没有为迁就 fork 而改写。

## 可复查入口

- [validation.json](validation.json)：修改前固定原版／fork 对照、最终重复、
  实际丢包、回归红绿、五个最终构建及完整 SDK 摘要。
- [consumers/ordinary.rs](consumers/ordinary.rs)、
  [ordinary_loss.rs](consumers/ordinary_loss.rs)、[probe.rs](consumers/probe.rs)：
  与实际执行的固定源码逐字节一致；后两者是单独固定的补充场景。
- [selected-input-continuity.json](selected-input-continuity.json)：实际 `.d` 选中
  源码、工具、锁、不可变产物及源码副本的最终哈希验证。
- [review.md](review.md)：Standards／Spec 双轴审查，发现与处理及最终结论。
- [archive-manifest.json](archive-manifest.json) 与
  [raw-verification.tar.gz](raw-verification.tar.gz)：每个成员均解包读回核验；
  包含普通消费者、原版源码、完整锁、构建／运行驱动与原始日志、各轮选中源码、
  失败准备及历史测试程序的无损压缩还原记录。
- [private-material-checks.json](private-material-checks.json)：以实际私有邀请值
  扫描全部公开文件及归档成员，确认未收录 SDK 私有材料。

## 最终验收范围

51 项外层测试通过：协议 17、socket 12、统一连接 15、路径维护 7。
包括未放行的实际加密 STREAM/PING 包在 ACK 到期后仍不能获 ACK；取得资格后
重播同一加密包可交付原内容；旧 ACK／关闭路径拒绝、绑定隔离与迁移保留。
路径维护使用自有、无外网的 MQTT broker，核验恢复、陈旧地址拒绝、关闭会话
不复活、同流 TCP→UDP 切换及真实延迟重新评估；broker 与测试容器自然结束。

修正后的 Native／Browser SDK 完成受限请求、一次审批、授权与 secret 持久化、
双方同身份重开、member relay PING/PONG，随后连接、进程及自有设施正常关闭。
只使用自有回环 TLS/WSS／数据中继和新建合成身份，所有参与者退出后删除生成
私钥与 NSS。SDK 私有原始回执、邀请、存储及参与者日志不入库，只保留摘要／哈希。

历史夹具两次准备问题分别是新增 helper 的 Rust 返回类型错误，以及把被动本地
迁移注入普通 server 而被既有接口提前拒绝；它们不算有效产品反例。另一次路径
维护缺少 MQTT 环境，原始失败仍保留，随后相同二进制加自有 broker 正常通过。
有效的恢复绑定与连续未知绑定回归均先失败，再由真实 ACK 路由验证修复。

本项覆盖所列兼容性与当前维护流程；完整公网、实体设备和全部平台／features
矩阵由后续整体验收承担。既有 249 份工作区改动的内容及脏状态保持原样。
