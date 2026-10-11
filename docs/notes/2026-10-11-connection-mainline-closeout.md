# 连接流程阶段收尾：relay、MQTT 与 IRC

2026-10-11。用户在比较实测耗时后确认本轮可以收尾。保留的方向是：获准
relay 可先让业务可用，MQTT 作为主要旁信道信令选项，IRC 作为备用联系渠道；
在节点维护预算内继续改善原生直连或 RTC。该结论延续
[ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)，不修改
已有可用直连优先的起步顺序、已有成员联系优先、默认 Quality 选路或资源许可。
它不是每次连接都必须先走 relay，也不要求 MQTT 与所有 IRC 设施同时常驻。

## 实测口径

| 已执行的场景 | 实际阶段 | 耗时 |
| --- | --- | --- |
| Host Browser ↔ Pixel Browser，公共 EMQX 信令，已有成员材料 | 成员 connect/accept、RTC 建路与认证路径确认，至普通业务句柄可用 | 发起端 6.867 秒，应答端 7.034 秒 |
| Host Browser ↔ Host Native，公共 APS1 relay | 成员握手与 relay 路径验证，至 relay 普通业务句柄可用 | 6.711 秒 |
| 上一条原连接继续改善 RTC，同机 RTC 候选 | RTC 协商、建路、新鲜 ACK 和双边确认，原连接/stream 继续业务 | 另用 7.874 秒 |
| 本轮最终 Native ↔ Browser，公共 IRC | 初次受限准入请求返回 AwaitingApproval | 43.081 秒 |

Relay 起步与 RTC 改善的两个 SDK 调用耗时合计为 14.585 秒；这不是一条
无间隔的整段 wall-clock 测量，也不包含首次入网的人工审批等待。Relay
阶段已验证业务收发，业务不必等 RTC 改善。MQTT/RTC 的普通句柄返回后，
双向业务流另行验证通过；IRC 的 43 秒尚不包含后续普通成员数据建路。

这几组是不同日期、构件、端点和阶段的已执行记录，不能计算同条件速度倍率，
也不是本轮最终 SDK 对 MQTT/relay 的重新性能测试。IRC 使用显式 60 秒
caller/attempt、8 秒 assembly 和 30 秒 queue 的比较条件，尚不认证完整默认
profile。MQTT/RTC 没有排除覆盖网络，relay 对照的 RTC 候选位于同机；二者
均不证明独立物理公网之间的 NAT 打洞成功率，也不提供 TURN 中继耗时。

标量摘录、原回执 hash 和准确计时边界见
[比较回执](../evidence/connection-mainline-closeout-2026-10-11.json)。原记录分别见
[MQTT/RTC](../evidence/unified-browser-browser-2026-10-06/README.md)、
[relay 起步与改善](../evidence/unified-relay-contact-2026-10-07/README.md)及
[IRC 最终实测](../evidence/irc-cid-confirmation-2026-10-11/README.md)。
Relay 原回执仍保留 SDK socket 的 CLOSING 尾部与全部终止任务退出未完整证明
的限制，不能用本轮 IRC 的自然清理资格覆盖它。

## 本轮确认的取舍

已支持的 relay、MQTT 和 IRC 足以作为当前连接能力主线，先保留经过实测的
实现，不继续扩大本轮 IRC 性能优化。直连与 RTC 是可用数据路径和后续改善
手段；是否选择它们仍按当前许可、实际质量和用户偏好判断，不保证直连总是
比 relay 更好。具体默认公网服务及数量没有在这次收尾中冻结。

MQTT/IRC 只承担受保护的联系与控制交换，不因握手成功而获得普通业务承载
资格。IRC 补充联系入口的可用性；如果没有可用获准数据 relay、成员数据中转
或已验证直连，仍须明确报告数据路径不可用。

MIOPUNCH-89 的实测优化已完成并提交；当前讨论和性能探究范围到此收尾。
M84/M69 的实机环境、完整默认 profile、故障切换及其他原有验收范围仍由
Plane 跟踪，不以本笔记宣告完成，也不因此启动新的实验或编译。
