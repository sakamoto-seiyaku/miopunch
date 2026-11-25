# MioCube 双方节点就绪到首次 UDP 直连的耗时

2026-11-25 · 已冻结，仅记录。

问题是：在当前 Windows 与 Linux 双端都已就绪、开始建路的条件下，首次经 UDP 直连收发为什么需要约 20 秒？关注现有流程中重复的工作和可以避免的等待。

两端近同步重启，以下时间各自从本机节点就绪算起。Windows 在 8.110 秒收到首个经中继的维护流回复，20.014 秒首次采到原连接选中 UDP 且该路径已有流数据收发。Linux 对应为 10.500 秒和 23.003 秒。路径按秒采样，首笔实际走 UDP 的回复时刻未单独测得。[现场交接](/tmp/miocube-direct-design-handoff-szn4ai3y.md) · [Windows 状态](/tmp/miocube-live-udp-sh91k5y2/u22sz.jsonl) · [Linux 状态](/tmp/miocube-live-udp-sh91k5y2/r_proxy.jsonl)

| 已观察到的现象 | 仍未确定的部分 |
|---|---|
| 两条中继候选分别在 Windows 就绪后 4.943 秒和 7.362 秒完成同一对端的成员校验。后一条约 0.3 秒后取消。 | 两条连接并行校验，重复工作对胜出连接耗时的影响未知。[拨号记录](/tmp/miocube-live-udp-sh91k5y2/dial-u22sz.log) |
| Windows 在 7.787 秒拿到共享连接，约 9.72 秒进入这轮增强 UDP 打洞。 | 这约 1.9 秒内，具体是哪项条件延后或放行了打洞，现场记录未能区分。 |
| 本轮开始后 0.913 秒才开始查地址，2.385 秒才发首个增强探包，即节点就绪后约 12.1 秒。期间双方依次确认开始取样、打洞计划和端口就绪。 | 各次确认是否都需要单独等待对端回复，尚无定论。[Windows 打洞记录](/tmp/miocube-live-udp-sh91k5y2/trace-u22sz.log) · [协商实现](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L771) |
| 双方互报监听就绪后，Linux 按本轮策略再等约 3 秒才发首包。 | Windows 首批对外探包何时全部发完未打点，3 秒中有多少可以省下未知。[Linux 打洞记录](/tmp/miocube-live-udp-sh91k5y2/trace-r_proxy.log) · [策略来源](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md) |
