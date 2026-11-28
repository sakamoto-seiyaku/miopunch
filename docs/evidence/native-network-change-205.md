# MioCube 按实际绑定处理换网

MIOPUNCH-205 基于 `cbf108bf`，遵循父规格 MIOPUNCH-195。

## 实现

netwatch 比较完整接口地址及前缀，并接收 Linux LINK 通知。显式换网通知先刷新实际
接口状态；状态确有变化时按差异处理，没有可见差异时仍按 NAT-only 事件保守刷新。
绑定标记包含实际接口身份、源地址及前缀；已知观察目的时同时检查实际路由源。
接口快照未变的自然路由事件也触发已知目的的源地址复查，实际源未变则不更新寿命。
不以任意下一跳或路由表重写都已被覆盖为前提。

增强 UDP、TCP 观察及候选验证持有对应实际绑定的寿命。另一族发生变化时，仍有效
的观察和在途工作继续。旧 TCP 源消失后，正在等待响应的观察通过现有网络通知结束，
关闭实际待处理连接；无关通知只唤醒检查，不重启观察的三秒时钟。接口消失时，
依赖旧源的两族观察、物理占用和 TCP 载体结束，新源重新取样和公告。

Noq 按实际路径决定是否恢复。Iroh 记录每条受影响路径的换网时间，要求它取得新的
ACK；其他实际源仍有效的路径保留身份及确认。TCP 的资源包装器转发真实源有效性
查询，避免包装后的默认值把已失效的 TCP 路径误判为健康。普通探测的在途响应及
验证继续持有位置，健康路径的提前返回不会再反复取得并释放新位置而自行唤醒。

单族换网还会过滤周期 QAD 的旧报告及公告，保留另一族有效候选原来的观察时间和
到期时间。旧混合快照不能继续携带失效族；过期候选不会被过滤操作续期。旧网络的
晚到报告不能重新发布已失效地址。整接口变化和无可见差异的显式通知仍全部刷新。

## 原生成员真实入口

`scripts/check_native_network_change.sh` 在独立 user/network namespace 中运行实际
NativeClient，两端使用真实准入、成员会话和一条保持打开的 TCP 转发流。第三参数
选择双端或仅 provider 移动，第四参数选择 UDP 或 TCP 载体。观察站和数据 relay
均由用例自己持有。路由器使用稳定 MAC，替代接口在工作节点启动前预置为 DOWN；
切换时撤销旧接口、升起新接口。route、ICMP、邻居和 FDB 是独立拓扑资料，实际
新路径 ACK 及原业务数据的限时断言决定成员恢复是否成功。

- UDP 双端完整入口：`/tmp/miopunch-205-address-frame-udp-both/`，退出码 0。
  IPv6 前缀变化后保留原 IPv4 PathId 及其在途观察，新 IPv6 取得 ACK。整接口变化
  结束旧 IPv6 物理 reservation 和真实 IPv4 STUN 请求，在新源重新取样、公告和
  验证。reservation 证据发生于取样前，不代表已经发出 raw UDP 探测包。
- TCP 单端完整入口：`/tmp/miopunch-205-forward-binding-tcp-single/`，退出码 0。
  TCP-only 配置使用实际监听端口观察、实际 custom 地址及原转发流。旧源上的真实
  待处理 TCP STUN 请求被中断，新源的 TCP 样本和新 IPv6 路径 ACK 均通过。
  同一个移动原会话的旧 custom PathId 1、6 在整接口变化时被标记 affected，前缀
  变化时仍有效的 IPv4 PathId 1 则保持不受影响。
- 两个完整入口都核对原转发流的三轮双向数据、两端原 stable ID 和流归属。最后
  明确关闭原会话，确认其不能 repair，再通过既有 ping 入口建立不同 stable ID，
  两端取得 PONG。新 IPv6 ACK 仍限八秒，原流回复仍限五秒。
- 最终 UDP 单端完整入口：`/tmp/miopunch-205-four-final/single-udp/`，退出码 0，
  36.03 秒。仅 provider 换前缀及整接口，未移动的 consumer 仍取得新对端地址、
  实际新路径 ACK 和原流回复；两端终止后新 stable ID、PONG 和清理全部通过。

独立公共观察与自动同族观察使用原有 supersession 合同。公共 UDP 观察被自动轮次
替代时，用例只在原截止时间内接受该原会话同族、仍 current 的已接受计划，要求
其实际样本非空且全部来自新源。TCP 公共观察使用独立的已配置观察目的，避免对
同一保留四元组重新拨号造成 EADDRNOTAVAIL；未移动的一端复用仍有效的实际观察。

周期 QAD 的真实双栈 NAT 观察回归位于
`ipv6_change_retains_the_periodic_ipv4_qad_candidate_without_renewing_it`。原 red
`/tmp/miopunch-205-qad-cache-red.log` 已通过两族实际样本和 IPv6 源前置，随后在
实际移除 IPv6 源后无法取得保留的 IPv4 快照。green `/tmp/miopunch-205-qad-cache-green.log` 退出码 0，11.45 秒。

`route_source_change_refreshes_only_the_known_destination_scope` 在同一接口同时保留
A、B 两个地址。先只修改观察站路由的 MTU、保留源 A，实际在途请求与全局、绑定、
路由及无关族 token 都继续有效；再只把已知观察站的首选源改为 B。原 red
`/tmp/miopunch-205-route-source-red-2.log` 在路由 token 仍有效处失败。green
`/tmp/miopunch-205-route-source-green.log` 退出码 0，4.39 秒：原请求在一秒内
返回 Interrupted，新请求的两个真实 STUN 样本均来自 B，已分配的 A 绑定及无关族
继续有效。两次切换前后完整接口地址列表相同。

## 验证记录

独立 Standards 审查：无文档规则违反；一个 P3 启发式建议为 TCP 与 iroh 的源存在
判断有重复。它们处于各自实现层，本票保留现有边界。独立 Spec 审查发现一个 P2：
接口快照不变时丢弃已知目的的路由源变化。修复后限定复核确认解决，新增发现 0。最终针对已准入分类及仲裁前置的限定 Spec 复核也无新增发现，
确认客户端原分发与原核心断言均保留；该结论不证明时序稳定性。

最终共享 Native 受影响图构建通过：
`/tmp/miopunch-205-admitted-refresh-build/receipt.json`，463 Fresh、9 rebuilt，
81.02 秒。最终 WASM 检查通过：
`/tmp/miopunch-205-admitted-refresh-wasm-build/receipt.json`，294 Fresh、2 rebuilt，
13.34 秒。仅删除临时诊断后复用同一流程的已固定产物，没有为日志删除重编。

一次完整受影响回归为 `/tmp/miopunch-205-final-regression-runtime/results.json`。
该批在路由源 P2、仲裁夹具前置及服务端唤醒更正之前运行，没有重复整批。

| 目标 | 通过 | 失败 | 忽略 |
| --- | ---: | ---: | ---: |
| Native lib | 67 | 2 | 9 |
| path maintenance | 25 | 5 | 1 |
| network maintenance | 27 | 2 | 10 |
| native service streams | 16 | 5 | 33 |
| UDP observation | 14 | 0 | 1 |
| resources | 48 | 10 | 11 |
| tcp_punch 的 worker 单元目标 | 1 | 0 | 0 |

该 worker 目标不等于 `tcp_direct` 全集。两项 lib、四项 path、旧 TCP 换网、
部分 resources 及 service 失败与 203/204 已有记录重叠；本票没有把所有失败归为
基线。完整日志保留每个失败名称及原断言。

慢信令 fast→slow 用例在该批及随后一次精确运行失败。加入仅诊断时刻的版本在
原预算及全部断言下通过：快入口 3004 ms 关闭，慢入口 3323 ms 接受，header
3350 ms、预算 3998 ms，第二批 6624 ms，确认提交 6810 ms，仍小于原七秒。
日志 `/tmp/miopunch-205-timed-fast-slow.log`；诊断已移除。204 同宿主对照也通过。

仲裁用例的首个失败是手动 dry 前已有未取样的自动 Business holder；requests=0
不能证明未占位。真实 holder、优先级与 crossed 次序确认 slot 规则正常，且未触发
新的 network_refresh。夹具复用既有 reservation barrier，先确认双方手动 reservation，
再在释放 barrier 前建立原两条业务流并双向收发。原 Interrupted、WouldBlock、
零样本、稳定身份、流身份、工作轮期限及冷却断言均保留。更正后的首次运行在最后
pulse 等待处失败；整批和自有 broker 都结束后，带仅测试诊断的同图版本完整通过，
两端仍选 relay Path0、close=None、42 轮业务、两端各两个样本。
日志 `/tmp/miopunch-205-crossed-pulse.log`；诊断已移除。

两项新增 service 失败各复验一次，见 `/tmp/miopunch-205-service-focused/results.json`：
silent relay 在原四秒内未取得首个 MQTT 业务回复；small reply 的大流字节实际从
524288 增至 655360，但未满足两次采样进展断言。后者单列，未调整原采样或预算。
204 同宿主 silent 对照 2.272 秒通过，差异位于成员准入之后的首次 UDP 路径确认，
并非 MQTT 启动晚。先缩限 server REACH_OUT 正向唤醒仍未通过。已有 PEER_IPV6 诊断进一步确认启动
`purpose=Control` 的 custom 控制载体被误当作 healthy 原会话。刷新标记现在只用于
已准入并完成业务交付的原会话（`Ordinary` 加既有数据策略），随后 server 只在实际
标记 refresh 时唤醒 admitted planner。client ADD_ADDRESS／REMOVE_ADDRESS
原有分发保留。

最终一次四入口结果为 `/tmp/miopunch-205-four-final/results.json`，使用无临时
诊断的 `.artifacts/miopunch-205-admitted-refresh.*`：

| 入口 | 结果 | 实际证据 |
| --- | --- | --- |
| 原 silent relay／MQTT | 通过 | 首次业务回复 3.218 秒，小于原四秒；16.54 秒总用例 |
| accepting side 已知地址复用 | 通过 | 原 `[1,1]` 交换次数断言；5.75 秒 |
| 实际 UDP 单端换网 | 通过 | 36.03 秒；两阶段、原流、旧工作结算、终止／重拨及清理 |
| 原 crossed offers | 失败 | dry 断言及原工作轮三秒等待已过；最后停止 pulse 的原 250 ms 等待超时 |

最后一项的后续结果、协调者及冷却断言因 pulse 超时未执行；前述纯诊断版本完整
通过覆盖了这些断言。两次结果并列保留；没有扩大 250 ms、吞掉 pulse
或重跑到绿。原换网入口、silent 和 accepting 的最终正例不代表全部回归通过。
同流程的底层 gate 诊断显示两端实际 lo／IPv4 位置均取得或复用，没有 scope、绑定
标记或物理拒绝，见 `/tmp/miopunch-205-silent-gate.log`；该次首次回复 3.853 秒。
四秒窗口的旧失败继续保留，未扩为常规调度、RTT 或重试策略修改。

### 2026-11-27 结案回归

包含最新修复的工作树完成同口径回归。七个 Native 目标共 219 通过、3 失败、
71 默认忽略。附加的服务端隔离用例通过，两个临时 MQTT broker 均清理成功。
运行器退出码为 1，结案条件仍待满足。
[本轮结果](/tmp/miopunch-205-closeout.7cVYGw/runtime/results.json)

- **快入口让位给慢信令。** `fast_contact_yields_to_slow_signaling_without_restarting_the_repair_clock`
  在此前的 205 定向运行和本轮整批都返回原 7 秒调用期限已到。中间一轮整批用
  6.900 秒通过，离截止仅约 100 毫秒，属于压线通过。用例等快入口约 3 秒退出，
  再等慢入口 3.3 秒才送出可用 UDP 候选，两段等待共用原调用期限。
  在这个余量下，测试宿主的调度、并发负载和本地网络时序都可能左右结果。
  用例的稳定性和本轮超时的具体归因仍待验证。
  [此前单跑失败](/tmp/miopunch-205-fast-slow-focused.log:914) ·
  [6.900 秒通过](/tmp/miopunch-205-full-regression-current.eKmsaZ/runtime/path.log:1443) ·
  [本轮失败](/tmp/miopunch-205-closeout.7cVYGw/runtime/path.log:1440)
- **同代冷却后重试 TCP。** `active_original_application_members_retry_tcp_after_same_generation_cooldown`
  在 198 和 203 曾挂在最后的观察队列应为空。上一轮 205 整批通过，本轮在更早的
  中继路径选中断言处失败。这是现有记录中首次看到该断言失败。
  [198 失败](/tmp/miopunch-198-cooldown-focused.log:93) ·
  [203 失败](/tmp/miopunch-203-settled-active_original_application_members_retry_tcp_after_same_generation_cooldown.log:101) ·
  [本轮失败](/tmp/miopunch-205-closeout.7cVYGw/runtime/network.log:499)
- **p2panda 主动发布。** `p2panda_high_level_active_publishes_while_its_passive_branch_is_inactive`
  在 203 和查到的四次 205 整批均挂在 `active_publication.is_ok()`。
  本轮前的一次定向单跑通过。
  [203 失败](/tmp/miopunch-203-regression-runtime/resources.log:987) ·
  [定向通过](/tmp/mio205-audit-verified/mdns.log:4) ·
  [本轮失败](/tmp/miopunch-205-closeout.7cVYGw/runtime/resources.log:1976)

## 产物与平台

本票真实运行覆盖 Linux。共享浏览器源码单独检查；本票没有 Windows、Android、
macOS 或 iOS 的新设备运行结果。

旧中间产物在宿主可见进程和实际 inode/命令引用审计后按明确名单回收，释放
4,836,913,152 字节。明细为 `/tmp/miopunch-205-pin-consumers.json` 和
`/tmp/miopunch-205-pin-cleanup.json`；保留必要红例、当前验收产物和日志，没有清空
共享依赖缓存或调整容量上限。

最终产物固定时仍沿用 32 GiB store 上限。再次审计宿主可见的 307 个进程，确认
11 个已完成的诊断 pin 无执行、FD 或参数消费者后，删除其链接及唯一 store 对象，
释放 2,147,020,800 字节。明细为 `/tmp/miopunch-205-finish-pin-consumers.json`
和 `/tmp/miopunch-205-finish-pin-cleanup.json`；保留当前消费者、工具和依赖缓存。
