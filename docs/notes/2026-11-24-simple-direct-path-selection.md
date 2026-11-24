# 变化网络中的直连选路与测量

2026-11-24。建议 miopunch 沿用一条活动业务路径，按实际连接最近的往返表现换路。此前的双直连实验预先指定了低延迟和高带宽路径，验证了按流分配的潜在收益。线上自动识别哪条路更适合终端或大文件，仍待实网验证。[双直连实验](../evidence/warm-quic-terminal-2026-11-24.md)

## 线上选路需要知道什么

| 问题 | 可以直接取得的信号 | 怎样使用 |
| --- | --- | --- |
| 这条路现在还能往返吗 | 原 QUIC 的确认进展，以及必要时在同一条路径上返回的确认 | 当前路停滞时，优先考虑已有新鲜回程证据的备路。 |
| 两条合格直连目前哪条响应快 | 近期平滑往返时间（RTT） | 用切换门槛过滤小幅波动。ACK 经其他路径返回时，样本包含去程与实际回程的时延。[RFC 10069 §5.4](https://www.rfc-editor.org/rfc/rfc10069.html#section-5.4) |
| 文件实际上收得多快 | 文件持续传输时，接收端新增有效字节除以经过的时间 | 用来验收大流效果。空闲候选的可用带宽记为未知，应用自身供数不足时也无法据此估算链路容量。 |

当前原生成员连接默认把合格直连排在 relay 之前。同档比较实测 RTT，IPv6 有 3 ms 偏好，候选的偏置 RTT 至少改善 5 ms 才替换健康的当前路径。TCP 直连继承实际 IP 地址族的偏置，只在候选同档且偏置 RTT 相同时打破平局。[默认策略与重评间隔](../../src/path_selection.rs#L28) · [成员路径入口](../../src/path_selection.rs#L160) · [偏置和切换判据](../../vendor/iroh/src/socket/biased_rtt_path_selector.rs#L155)

如果运营商只对持续 UDP 大流限速，空闲时的小包 RTT 可以很好，文件传输时才会看到交付速率受限。测量应走同一对成员正在使用的承载。LAN、WAN、TCP、UDP 和地址族可以留作诊断信息。是否允许使用收费出口，则取决于用户或系统提供的使用策略。

## 可借鉴的软件和研究

- **ZeroTier** 提供一次使用一条主路的 `active-backup`，也提供把网络流固定到链路的 `balance-xor` 和可按质量调整分配的 `balance-aware`。它按端口、协议等识别网络流，同一条加密 QUIC 内的终端与文件 stream 要由 QUIC 端点区分。其文档也指出，光纤和蜂窝性能相差很大时，盲目均摊可能表现不佳。[ZeroTier Multipath](https://docs.zerotier.com/multipath/)
- **腾讯 TQUIC** 的多路径模式提供 MinRTT，给下一个包选择可用路径中平滑 RTT 最低的一条。**阿里 XQUIC** 分别提供 MinRTT 和 Backup 调度器。这些实现可作算法参考。多路径 QUIC 标准把具体调度留给实现和应用。[TQUIC API](https://tquic.net/docs/api_reference/c_quic/) · [XQUIC MinRTT](https://github.com/alibaba/xquic/blob/main/src/transport/scheduler/xqc_scheduler_minrtt.c) · [XQUIC Backup](https://github.com/alibaba/xquic/blob/main/src/transport/scheduler/xqc_scheduler_backup.c) · [RFC 10069 §5.5](https://www.rfc-editor.org/rfc/rfc10069.html#section-5.5)
- **ECF（CoNEXT 2017）** 在实验台与真实网络中发现，时延和带宽差异大的多条路径上，只按最低 RTT 调度可能让高带宽链路利用不足。作者用 RTT、拥塞窗口、待发数据量估计完成时间。这项研究针对 MPTCP，说明了低时延选路在追求聚合吞吐时的局限。[ECF 论文](https://api.repository.cam.ac.uk/server/api/core/bitstreams/9f216be1-4124-4f10-bbad-137e912cc7ff/content)
- **Flent RRUL** 用 TCP 上下行流量制造负载，同时测量 ICMP 和 UDP 往返，适合独立检查一条线路在传大文件时会增加多少排队时延。产品验收可以直接在文件传输时记录终端命令提交到回显的 P95，以及同一窗口的文件有效吞吐。[Flent 测试说明](https://flent.org/tests.html#the-realtime-response-under-load-rrul-test)

项目已有的 Linux 对照显示，交互流换到另一条独立队列后，P95 从 168.4 ms 降到 16.3 ms。仅换一个目的地址、仍经过原队列时，P95 从 168.0 ms 变为 168.1 ms。这里的收益来自绕开排队。两个 IP 地址本身不足以推断两个独立出口。[共享瓶颈对照](../evidence/warm-quic-terminal-2026-11-24.md)
