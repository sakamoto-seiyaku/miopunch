# MioCube 原生成员建路并发验收

MIOPUNCH-200，基线 `d6d91ec8`。Linux 宿主与隔离网络命名空间实测。

## 已交付行为

融合端点的 `Building` owner 为最多八个不同成员保留主动建路位置。首拨、
普通 TCP 候选验证、TCP 互拨与修复、增强 UDP 和普通 QNT 使用同一池。
同一成员的并行候选、地址族和传输通过引用计数共用一份。实际工作结束、
取消或失败释放位置，等待下一次重试期间持有已有节流时间而释放位置。
正常收发和保留健康待命会话继续使用原连接。

固定上限定义在 `iroh::endpoint::MEMBER_BUILD_LIMIT`，SDK 调度引用该值。
旧 `max_concurrent_contacts` 输入仍可读取并归一到八，新的序列化不再输出
该旧配置项。其他维护周期、邻居数及健康会话保留配置沿用各自规则。

普通 QNT 的主动探测在拥有位置后发送。等待位置时保留协议重试次数。
已发探测的响应等待、实际 opening path 及已有返回路径 ACK 验证按其期限
共同持有成员位置。显式响应标记与原 `known_probes` 重试节流分别结算。
成功、取消或到期的显式 stop 清除该轮的探测、旧 challenge 关联以及客户端
尚未打开的路径，避免释放位置后迟到响应重新启动建路。
位置通知订阅由各连接的 `MemberPaths` 取消，准入升级复用同一订阅。

物理出口、地址族和传输的打洞独占期继续交 MIOPUNCH-201 处理。

## 真实入口

`tests/member_building.rs` 使用真实准入记录与原生融合成员入口。

- `ninth_member_waits_for_an_actual_building_position` 在实际拨号前的既有
  EndpointHooks 入口持住八个成员，第九个排队。一个完成后第九个启动，
  九个成员分别读取并回送八字节业务数据，重复业务复用原 stable ID。
- `cancellation_releases_a_position_before_the_original_deadline` 用同一九成员
  入口取消一个正在拨号的任务，等待者在原拨号期限前递补，剩余八个成员
  完成双向业务。两个请求联系同一成员时只执行一次真实拨号。
- `relay_improvement_shares_positions_with_first_contacts` 已有健康 relay 会话
  时仍能实际开始八个其他成员的首拨。原会话的真实 TCP 候选等待位置，
  一份首拨结束后完成路径 ACK 和选中 TCP，并在同一流完成切换前后往返。
  目标 TCP 使用实际绑定的 `:0` 运输器返回的端口。
- `ordinary_qnt_keeps_its_position_until_the_path_acknowledges` 抑制返回路径 ACK，
  双方实际路径响应已到达后仍保留成员位置。恢复 ACK 后两端选择原连接的
  UDP 路径，实际业务往返并释放位置。

首轮真实九成员 red 日志 `/tmp/miopunch-200-red.log` 显示九个都进入真实拨号，
断言八而失败。前三入口通过证据为 `/tmp/miopunch-200-all-building.log`。
后续修复沿用首拨两项证据，最终混合和 QNT 证据分别为
`/tmp/miopunch-200-delivery-mixed.log` 和 `/tmp/miopunch-200-delivery-qnt.log`。
交付固定产物 `.artifacts/miopunch-200-delivery.building` 可精确复现各入口。

## 回归与构建

一次完整选定集合位于 `/tmp/miopunch-200-regression-runtime/`。
lib 为 66 通过、2 失败、8 忽略，standby 为 5 通过、4 失败，resources 为
51 通过、7 失败、11 忽略。已有有效结果复用，后续只运行受改动影响的定向入口。

Noq NAT traversal 模块最终 7 项通过，包括显式 stop 后旧响应失效与 queued-path
清理断言，日志 `/tmp/miopunch-200-proto-final.log`。运行产物为
`.artifacts/miopunch-200-delivery.proto`。
Native 目标通过 `--locked --offline --features nat-lab-test-hooks` 构建，
收据 `/tmp/miopunch-200-complete-build-2/receipt.json`。
WASM 检查使用现有 browser-probe 包与相同共享缓存，收据见
`/tmp/miopunch-200-wasm-delivery-check/receipt.json`。

## 已知基线与后续关联

lib 两个失败是此前已用 c1 基线确认的旧 coordinator 用例，详见
`docs/evidence/native-lan-member-196.md`。
两个普通 five-idle standby 用例在同宿主 `d6d91ec8` 固定物保留原 40 秒断言
同样失败，结果 `/tmp/miopunch-200-baseline/runtime/results.json`。
旧迁移变体落在同一共享夹具断言，原输入现显式为 `Some(1)` 与 `Some(4)`。
主线程决定复用这组基线定性，继续保留五个健康会话和待命留存断言。
11 月 24 日历史产物的 active 用例通过，用来定位后才做 d6 精确比较。

resources 七项失败完整保留在集合日志末尾。其中旧可达性分类、中断观测及
p2panda mDNS 已在先前基线出现。其余增强观测用例涉及此前 MIOPUNCH-199
移除混合观测的旧顺序 fallback、QAD 等待及重试错误分类，交 201/202 继续核对。
原断言保留。

晚 IPv6 用例在本票最终图与 d6 基线的同一隔离 fixture 均停于 line 713 的
双端增强观测前置，尚未注入晚 IPv6 候选。日志为
`/tmp/miopunch-200-late-ipv6.log` 与 `/tmp/miopunch-200-baseline-late-ipv6.log`。
该增强入口连同已有 198 的 TCP 冷却尾计数差异继续交相应后续票处理。
本票新的真实普通 QNT 待 ACK 位置交接验收已通过。

独立 Standards 和 Spec 第 1 轮各发现一项生命周期问题，分别是通知订阅
随连接结束取消，以及 QNT 验证完成前持有位置。两项一次合并修复后由原
审查者只读聚焦复核关闭。最终显式 stop 的迟到响应边界经主线程核对并由
Noq 定向协议断言验证。运行平台覆盖 Linux，其他平台尚未实测。
