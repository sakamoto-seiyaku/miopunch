# RTC 放行前资格变化与迟到事件

本次对应 [WebRTC 第 8 步](2026-10-03-browser-webrtc-replan.md#8-资格变化与迟到事件)。四个独立标准故事已经通过真实 Chromium/Linux native RTC：取消本次建路后，新 attempt 可以成功；在双方初始新鲜路径 ACK 和私密确认之后、普通句柄放行之前，实际成员移除、当前秘密世代安装或该 RTC carrier 禁用均使旧 attempt 不返回业务句柄。迟到事件没有重新发布旧地址或恢复旧 M。[证据包](../evidence/browser-rtc-invalidation-2026-10-04/README.md)区分标准资格、组件证明和失败诊断；最终审阅及提交状态以其中回执为准。

资格范围仍是已保存材料的当前成员、自有正常 TLS/private NSS、同机自有隔离 namespace，ICE/STUN/TURN 列表为空。首次 WSS 材料取得、真实业务/replica、异网与成本按后续步骤处理。

## 放行决定与新鲜 ACK

原实现能够先完成 RTC 路径 ACK 和私密 `final`，随后在实际 native SDK 秘密世代切换已关闭旧 M 时，browser 尚未观察到对端关闭且自己的旧材料仍有效，继续返回普通句柄。这个真实运行失败保留为 `secret-stale-final-red`；它不是编译或设施错误。

现在两端在既有私密 `final` 之后、普通放行之前，通过同一 lease 的 `validate_release` 启动新的 opaque QUIC probe。它必须属于同一原 M、同一 RTC route 和 path，且达到新的 packet watermark；之前的 ACK 不能满足这次等待。等待消耗原 caller absolute deadline，并观察原 M 关闭、取消、lease retirement 和当前 qualification。等待结束后，两端再次检查本地当前资格；既有最终 actor route seal 仍负责排队到普通授权之间的检查。没有增加身份握手、协议版本、候选、拥塞策略或产品超时。

真实正常 WSS 回归中，browser watermark 从 10 增至 17，native 从 5 增至 13，均在 path 1 得到新的 ACK。普通数据仅走获准 RTC，signaling 的业务字节不增长；同一 M 在原 12 秒 setup deadline 之后继续交换数据。

公开 transport 组件回归独立证明了新等待的行为：先得到旧 ACK，再成功得到更大 watermark 的 ACK；停止该测试自己的 RTC queue bridge 后，实际原 UDP M、其他路径、当前 permission 和旧 ACK 都继续有效，但新的 RTC release validation 在 150 ms caller 时钟内失败。同一原 M 仍能经 UDP 完成真实 stream。这个组件不是 browser RTC 资格，也不模拟秘密或成员资格。

## 实际来源与四个独立故事

| 故事 | 实际变更和观察 | 包括准备与清理的时间 |
| --- | --- | --- |
| 取消后新建路 | 实际 per-attempt cancel，3.4 ms 返回取消；原 M 2460968 退休，新 M 2471168 得到新的 RTC ACK 并交换数据，保存材料及运行节点保留 | 9.029 s |
| 成员移除 | native `manage` 产生并验证/持久应用真实签名记录，同一 public header 经 browser SDK/IndexedDB 应用；membership/authorization revision 变化，secret/resource 不变 | 7.555 s |
| 当前秘密世代 | feature-gated 诊断入口调用真实 durable material installation、current store pin 和 gate source 更新，generation 0→1；成员及 resource 不变，browser 本地旧 generation 0 保留 | 7.656 s |
| RTC carrier 禁用 | 对实际已安装 `RtcTransport` 调用 `set_resource_allowed(false)`，resource revision 0→1；membership/authorization/secret 不变 | 7.696 s |

秘密世代故事是 native 当前凭据失效注入，不是完整 browser/network-wide rotation。组件测试另外验证真实安装后旧 proof/M 失效，以及关闭、重开存储后当前 generation/pin 和节点身份、成员列表保持。resource 故事只作用于该 RTC carrier/registry，不声称全局配置热更新。

标准秘密故事中，browser 实际进入新的 release probe，watermark 9→16，ACK 为 false，随后因原 M 关闭而不返回句柄。它不证明人为延迟关闭通知时的因果效果。本地尚未退休期间的一条额外 30-byte QUIC validation datagram 与普通业务区分；native 已退休 route 的实际发送计数不增长。两端本地退休以后以及迟到事件以后，实际 DataChannel send counters 均冻结。

## 原对象事件与生命周期

有界诊断 checkpoint 属于一个实际 transport 和 attempt，仅在 channel-ready 或 before-release 已有边界暂停本次 hook；原 deadline、取消、关闭及 lease current 检查继续运行。clear 退休旧 IO，不能给予业务许可，也不能将旧 attempt 交给后续新 M。

runner 捕获真正由 WASM 安装的 `onopen`、ICE gathering handler 及实际 full remote SDP；退休后 callback registrations 已拆除。调用捕获的 JS function 得到 Rust closure 已释放的拒绝，回执明确记录 disposed，未把它算作存活 Rust handler 被执行。对同一实际 closed PC 再调用 `setRemoteDescription`，完整 SDP 因 `InvalidStateError` 被拒绝。当前协议是 gather-complete full SDP，没有为了测试而加入 trickle ICE。

原 M、route 发布地址、TX/RX queue、association 和 pump 均到零或退休。native PC 实际关闭；成员及 resource 故事立即回到 socket baseline 6。秘密故事的即时 socket count 为 8，之后的 final/late 观察才回到 6，不将异步 driver 回收写成立即完成。每个 fixture 最后退出 0，删除精确自有 container/network，并通过查询证明两者已不存在。

每故事从准备证书/fixture 前开始计时，工作最多 45 秒，清理预留 15 秒，总计最多 60 秒；编译与 bindgen 单独记录。产品 12 秒 caller、10 秒 contact 和 2 秒 maintenance 时钟保持。归档没有 SDP、ICE credentials、DTLS fingerprint、exporter 或秘密材料内容；正常 WSS RTC stats 中原有的 22 个 `usernameFragment` 字段在归档时递归省略，回执分别记录原始 hash、变换及保存内容的 hash，不伪称这份 JSON 与原始字节相同。

## 入口与证据边界

```sh
rtk proxy python3 scripts/check_browser_paths.py --case rtc-invalidation --invalidation-mode cancel --output /tmp/mio-rtc-s8-cancel
rtk proxy python3 scripts/check_browser_paths.py --case rtc-invalidation --invalidation-mode member --output /tmp/mio-rtc-s8-member
rtk proxy python3 scripts/check_browser_paths.py --case rtc-invalidation --invalidation-mode secret-standard --output /tmp/mio-rtc-s8-secret
rtk proxy python3 scripts/check_browser_paths.py --case rtc-invalidation --invalidation-mode resource --output /tmp/mio-rtc-s8-resource
```

默认重新构建 native/WASM 与 bindgen；显式 `--skip-build` 用于已经核对执行产物的重复 fixture。四个最终标准故事具有相同的 52 个相关源码 hash 和 5 个执行产物。随后只补上 public unified wrapper；其真实 resource 故事单独验证当前入口。此前 72 项 native guards、native/WASM typechecks 及正常 WSS 回归与当前生产 Rust/执行产物相同，仅 driver 的三个 selector/wrapper 文件有明确记录的后续 delta，没有伪称旧 raw receipt 的所有源码 hash 仍是当前值。

另保留两次不同诊断：早期修正后的秘密故事虽然通过，但在新 probe 前已观察到原 M 关闭，属于非因果 green；controlled-PC-cleanup 诊断虽然使 native 原 M/IO 真实退休并保留真实 native DataChannel Open/SCTP Connected，却未达到 browser 原 M 仍开的前提，属于 prerequisite fail。该诊断原 assertions 和结果保持独立；没有通过延迟通知、伪造 live/credential 标志或削弱断言补成成功。标准故事、新 release 组件回归及正常 actor 新鲜 ACK 是本次实际资格证据。
