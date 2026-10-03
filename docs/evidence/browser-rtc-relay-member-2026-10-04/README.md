# 原 relay M 的 RTC 路径资格

[实施说明](../../notes/2026-10-04-browser-rtc-relay-member.md)；规划第 5 步；固定前点 `f5bae5c727c632faf5cac0b3473b01cbbe938fe4`。

- [validation.json](validation.json)：实际结果、审查、14 个 member-relay / 5 个 RTC path 检查及 artifact SHA-256。
- [rtc-member.json](rtc-member.json)：真实同 M、原 stream、两端 fresh probe/ACK、retained 超过原 12 秒 deadline、第二/新 M 隔离、丢 QUIC probe、2 秒 caller、native pump 故障及实际清理。
- [source-receipt.json](source-receipt.json)：RTC 实验源码及 native/WASM 构件身份；后续 S2 fixture-only 修复单独列出。RTC 输入和实际构件在该修复后仍逐项一致。
- [S4](s4-regression.json)、[S2](s2-regression.json)、[S1](s1-regression.json)：当前构件的真实受影响回归。S2 fixture 在同一原 12 秒 deadline 内等待 browser 关闭 A，避免 FIN 尚未送达就本地关闭连接；申请身份/权限断言保留。
- [Standards](standards-final.md) / [Spec](spec-final.md)：独立两轴审查。两个必修 pump 问题已关闭；两项可选重复代码建议暂存；计数命名已区分本 lease 与整个 transport。
- `*-red.json` / [checks-history.json](checks-history.json)：保留实际失败，不能作为最终资格。`member-relay-red.log` 是当前代码曾出现的兼容回归；固定前点同 case 对照和恢复旧载体评分后的单变量结果见 validation。最终完整 14 项另在 [日志](member-relay-final.log)。

资格限于同机 Chromium 153/Linux native、browser 原 QUIC client / RTC offerer → native server / answerer、正常证书的自有 relay、无 STUN/TURN 的非 relay RTC 候选。未扩展为异网 NAT 成功率、WSS M hook、native dialing side、其它浏览器、完整资源/撤销产品矩阵或业务/非空 replica 资格。队列容量和 Scope/M-close 的阻塞取消用真实 TLS-M/lease 和受控通道容量 seam 验证。

没有保留原 exporter/TLS key、网络秘密或临时邀请；S2 附加回执去除了邀请字段。原临时日志和编译回执位置保留在 JSON 中。

Native socketAfter 是即时快照：10 秒 drop 为 13（起点 6）；同一 native 进程在下一个 short PC 构建前已回到 6。该观察说明异步回收，不是每个 case 都即时 6→6，亦不代表已计数全部 webrtc 内部任务；具体快照与限制见 validation.socketObservation。
