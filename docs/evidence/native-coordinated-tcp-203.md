# MioCube 双栈 TCP 协调互拨

MIOPUNCH-203 基于 `fef74678`，沿用父规格 MIOPUNCH-195、ADR 0019 及原成员连接。

融合端点从传输实际监听表选择本族绑定，取样前取得同出口、同族 TCP 占用。
双方在原受保护交换中确认就绪、共享真实 TCP STUN 观测及许可候选，再从原监听
端口主动互拨。IPv4 保留原 `MIOTCP01` 与 DirectTraversal 字节，IPv6 使用
`MIOTCP02`、新增内部 stream kind 及受限首数据交换。两族使用同一实现，各自持有
实际物理范围及已执行轮次时点，先合格的数据路径交付原成员会话。

真实 TCP 连接完成后，弱引用握手凭据跟随该 socket 的实际任务存活，确认本地源
绑定即可释放打洞占用。载体前言仍控制 channel 激活，原 QUIC 随后完成新鲜 ACK、
准入、选路及业务交付。握手关闭会使尚未激活的凭据失效。

198 留下的额外 TCP 观察来自成功结算和健康状态发布之间的调度竞争。成功分支
读取实际健康状态并清除旧轮遗留的 `next_traversal`，随后真实健康路径失效仍触发
重新资格。原空观察队列断言、同代冷却时限和原流断言均保留。

## Linux 实际结果

- 两族都从获准 relay 会话取得原 TCP 路径。夹具先压住新路径 ACK，业务继续在
  原流收发，随后允许 ACK 并在准确 TCP 路径独立接收。原会话标识及流编号保持一致。
  最后握手图的 IPv6 用例通过 6.37 秒，IPv4 通过 7.60 秒，结果位于
  `/tmp/miopunch-203-final-network-results.json`。
- 正式 NativeClient 仅通过 MQTT 联系时，两族都完成首条 TCP 业务请求及回复。
  显式禁用 TCP 的对应组没有监听、观察或业务交付。日志为
  `/tmp/miopunch-203-{v4,v6}-restricted.log`。
- 现有双 NAT 驱动增加 IPv6 路由及两端有状态入站过滤。每组在启动互拨前确认
  双方向直接拨入均失败，随后观察到双方从实际监听源端口主动连接。两族、两个
  访客方向均通过，首次业务回复分别为 1.328、1.432、1.282、1.299 秒。
  原认证标识、相同 attempt、原业务流、选中路径和两端 STREAM 收发均经独立读取。
  结果位于 `/tmp/miopunch-203-{v4,v6}-delivery/positive-{a,b}/result.json`。
- 完全阻断两端 TCP 的对应四组保留原 8 秒业务期限及无首回复断言，并确认每端
  只有一次成员认证，双方 attempt 一致。通过结果位于
  `/tmp/miopunch-203-{v4,v6}-blocked-final/results.json`。

阻断组运行期间，NativeClient 现有后台任务还会查询设备移除通知。实际调用为
`native_client.rs` 的 `receive_removal_notice` 到 `native_device_management.rs`
的 `REMOVAL_ALPN` 联系。TLS 后记录证实其协议为 `miopunch/device-removal/1`，
当前成员接收端按原有规则拒绝该查询。夹具逐条匹配协议、身份和发起方，将 raw
MQTT Initial 总数严格核对为一次成员联系加实际管理控制联系数。正例仍要求 raw
Initial 恰为一次，未知额外联系仍使验收失败。

审查指出握手凭据原先要等载体前言到齐才就绪。新真实 socket 用例在对端暂不发送
前言时先确认 TCP 连接与准确监听源，再确认凭据就绪而 channel 尚未激活，最后关闭
对端验证凭据失效。原实现红测 1.63 秒失败，修复后两族合计 0.97 秒通过，日志为
`/tmp/miopunch-203-handshake-{red,green}.log`。

## 回归与审查

一次完整受影响集合顺序运行于正常宿主，使用两个自有 MQTT broker 及已固定 CLI。
结果在 `/tmp/miopunch-203-regression-runtime/results.json`。

| 集合 | 通过 | 失败 | 忽略 |
| --- | --- | --- | --- |
| lib | 66 | 2 | 8 |
| resources | 51 | 7 | 11 |
| tcp_direct | 10 | 2 | 0 |
| network_maintenance | 27 | 1 | 9 |
| native_service_streams | 19 | 2 | 33 |

lib 两项、TCP allowlist／撤权两项及 resources 的旧观察、LAN／mDNS 和退避失败沿用
前票证据，完整原断言保留。maintenance 唯一失败仍是已对照的换网后 TCP 观察用例，
交后续换网票。冷却及退休重新资格通过完整集及定向验证。一次末次退休定向运行
又暴露旧轮遗留 due，已补齐成功分支清理。随后一次冷却运行在成功前的部分观察轮
发生取消并留下尾样本，取消来源尚未证实，保留
`/tmp/miopunch-203-settled-active_original_application_members_retry_tcp_after_same_generation_cooldown.log`。
两次带分支记录的运行及最后非插桩原用例均通过，最后冷却为 7.10 秒、退休为
7.55 秒，详见 `/tmp/miopunch-203-clean-scheduling-results.json`。
临时取消诊断已移除，未给该单次时序敏感失败附加推测性修复。

两个 service 失败发生在重开后 helper 对 `initial_data_path` 的强制等值断言。
`native_ping_reuses_a_verified_member_address_after_reopen` 实际 PONG 经另一条获准 IP
先行，`udp_first_reply_keeps_the_started_tcp_round` 已完成原流、晚 TCP 资格及备路保存，
重开后另一 TCP 地址先合格。对其精确基线归因尚未证实，原断言及完整日志保留，
供后续候选／恢复票核对。

独立 Standards 全审没有规则违规，保留一个 P3 地址族 bool 表达建议。
Spec 全审的一项握手释放 P2 已修复，原审查者聚焦复核关闭；随后对联系计数和
Android ABI 修复的聚焦复核无新发现。复核没有重启全审。

## 平台与构建边界

Linux、Android、Windows、macOS、iOS 通过同一观察和互拨代码选择地址族。
配置增强时，Windows 使用 SO_REUSEADDR，四个 Unix 目标使用 SO_REUSEADDR 与
SO_REUSEPORT，观察和主动互拨都绑定实际监听端口。普通监听及普通拨号仍有各自入口。
Android 的 `in6_pktinfo.ipi6_ifindex` 与 Linux libc 字段类型不同，局部转换保留位宽，
修复本次实际交叉编译失败。TCP 地址沿用既有未映射、无 scope 的 IPv6 格式。

- Linux 已完成上述真实成员及内核网络实验。
- Windows x86_64 GNU 最终测试图构建通过，522 Fresh、8 rebuilt，收据位于
  `/tmp/miopunch-203-windows-final-build/receipt.json`。
- Android aarch64 最终测试图构建通过，540 Fresh、6 rebuilt，收据位于
  `/tmp/miopunch-203-android-final-build/receipt.json`。
- WASM 库检查通过，290 Fresh、6 rebuilt，收据位于
  `/tmp/miopunch-203-wasm/receipt.json`。

Windows laptop 和 Pixel 6a 可达，但自动审批分别拒绝向具体设备传输及执行仓库
测试产物，理由是缺少对目的地和载荷的明确授权。两次传输均未执行，实际宿主结果
等待明确授权。可复用固定物为 `.artifacts/miopunch-203-windows-final.service.exe` 及
`.artifacts/miopunch-203-android-final.service`，独立临时目录执行和清理步骤已留给主线程。
macOS、iOS 当前没有可用实测宿主，本次没有其编译或运行结果。

所有构建沿用共享 target、既定 profile 和锁。Windows 固定编译镜像及 Android NDK
包装器均复用现存版本。Android 首次被 96 GiB 阈值阻止后，仅回收六个经宿主引用
核对的旧测试可执行文件，实际释放 1,416,527,872 字节，明细位于
`/tmp/miopunch-203-cache-cleanup.json`。

收尾时确认宿主无实际引用后回收本票十二个中间固定物，实际释放 2,671,337,472
字节，名单位于 `/tmp/miopunch-203-pin-cleanup.json`。保留最终 Linux 维护测试、
实际 NAT service、握手用例、已有失败回归及待设备执行的最终 Windows／Android
固定物。最终冷却及退休复现入口为 `.artifacts/miopunch-203-final.network`。
