# fake TCP 传输原理与 UDP 打洞后的路径转换

日期：2026-10-03。状态：已归档的源码与规范研究证据。用户随后要求保留本文，并将后续验证标为低优先级任务
[MIOPUNCH-66](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/f3b59f26-9db9-4992-9638-a0ca45f251cb)。
研究方向是参考类似原理做 fake TCP 传输，没有决定接入现成项目或立即实现。
TCP 仍是需要时显式开启的可选能力；本轮实际执行的是独立的
[UDP/TCP STUN 集成票 MIOPUNCH-67](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/058e5f38-8632-4dba-92af-f1f28bbae3d5)。
后续工作状态以 Plane 为准。

用户尚未给出项目名。本文暂按 Rust 项目 **Phantun / fake-tcp** 核查，它自称将 UDP 数据报转换为面向 L3/L4 防火墙与 NAT 的 fake TCP。上游源码固定到 [`6256f630`](https://github.com/dndx/phantun/commit/6256f630a1e6f91489805669afc9700c1289a48e)，查询时 docs.rs 的 fake-tcp 文档为 0.7.1；不假定两者版本完全一致。[项目说明](https://github.com/dndx/phantun#overview)、[库文档](https://docs.rs/fake-tcp/0.7.1/fake_tcp/)

## 1. 关键结论

**可以研究这种数据报载体，但 UDP 打洞成功不等于 fake TCP 已打通。** Linux conntrack 的连接标识包含传输协议，TCP 与 UDP 即使使用相同 IP 和端口数字也不是同一个连接状态；RFC 5382 也明确 TCP 的过滤策略可以独立于 UDP。把已有 UDP 包改为外层 TCP 后，不能把原 UDP 映射或放行状态视为已获得的 TCP 状态。[Linux tuple 定义与比较](https://github.com/torvalds/linux/blob/v6.12/include/net/netfilter/nf_conntrack_tuple.h#L33)、[RFC 5382 §4.3](https://www.rfc-editor.org/rfc/rfc5382.html#section-4.3)

**推断：** 可把已认证的 UDP/QUIC 会话作为协调渠道，另行交换 fake TCP 的候选、角色和尝试时机，建立独立的 TCP 协议路径，再在原 QUIC 连接上验证和选用它。这里复用的是会话及控制能力，不是 UDP NAT 映射。规范描述了 TCP 的 SYN 建映射与 simultaneous-open，但不能据此保证实际 NAT 会放行。[RFC 5382 §4.2](https://www.rfc-editor.org/rfc/rfc5382.html#section-4.2)、[本地原连接路径接口](../../vendor/iroh/src/endpoint.rs#L1132)

## 2. 线上的“TCP”与保留的数据报语义

| 载体 | 链路上的外层 | 交付语义与限制 |
| --- | --- | --- |
| 普通 UDP | IP / UDP | 本文只把它作为已建立的对照路径 |
| 本仓库现有真 TCP 载体 | 内核 `TcpStream` | 用真实 TCP stream 承载 QUIC 数据报帧；这是当前源码行为。[源码](../../src/tcp_transport.rs#L765) |
| Phantun 式 fake TCP | IP / TCP / 数据报内容 | 用户态栈保留数据报交付，未提供真 TCP 的数据重传、拥塞控制和流控。[库说明](https://docs.rs/fake-tcp/0.7.1/fake_tcp/) |

`packet.rs` 将 IPv4 Protocol / IPv6 Next Header 设置为 TCP，生成 TCP 端口、SEQ、ACK、flags、window 和校验和。**外层不是 UDP**；“UDP 转 TCP”描述的是输入/输出数据报与线上封装的关系。推断：如果仅在 UDP payload 里放一个 TCP 风格的头，线上仍然按 UDP 处理，不能得到本方法的协议分类效果。[报文构造与解析](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/packet.rs#L31)

握手走 SYN → SYN+ACK → ACK，并有握手超时重试；建立后，每次 `send` 把一份数据报放进带 ACK 标志的 TCP payload，SEQ 按 payload 长度推进；`recv` 直接交付收到的 payload，更新 ACK，没有等待缺失字节补齐后按序交付的分支。**握手有重试，不应概括成“一概不重传”；缺少的是可靠 TCP 数据流的重传与排序保证。**[状态机与数据收发](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/lib.rs#L155)

Phantun 的 client 收到本地 UDP 数据后调用 `stack.connect`，server 调用 `stack.listen/accept` 后转发到配置的 UDP 目标。这是其客户端/服务端用法，不能当作任意两端 NAT 后 P2P 建连已经完成的证据。[client](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/phantun/src/bin/client.rs#L166)、[server](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/phantun/src/bin/server.rs#L160)

## 3. UDP 打洞之后，哪些可以复用

- **控制通道：可以作为协调基础。** 推断：已可用的端到端 QUIC 会话可承载新路径协商，不必为外层 fake TCP 再引入独立节点身份或成员资格体系。现有旁信道 ADR 已区分端点安全会话与业务路径、成员准入。[ADR 0005](../adr/0005-side-channel-control-bootstrap.md)
- **候选与 NAT 状态：必须按 TCP 独立验证。** Linux 从 IP 头取协议号并写入 conntrack tuple；RFC 5382 的 TCP 映射由内部 SYN 建立或复用。UDP 观察到的公网 IP/端口最多是尝试线索，不能证明 fake TCP 将得到同一个外部端口、相同过滤或可达性；即使数字碰巧一致，协议状态仍独立。[Linux 解析](https://github.com/torvalds/linux/blob/v6.12/net/netfilter/nf_conntrack_core.c#L289)、[TCP 映射规则](https://www.rfc-editor.org/rfc/rfc5382.html#section-4.2)
- **P2P 握手：Phantun 不能直接证明已经覆盖。** 核查版本的 `SynSent` 分支仅接受 SYN+ACK，没有接到对端裸 SYN 后进入 simultaneous-open 的分支；`Stack::connect` 还自行随机选择本地端口。推断：参考原理时仍须解决源端口持有、TCP 候选观察、双端发 SYN 及 crossed SYN 状态转换，而不能照搬 client/server 后宣称具有 P2P 打洞能力。[connect 状态机](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/lib.rs#L251)、[端口选择](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/lib.rs#L374)、[simultaneous-open 规范](https://www.rfc-editor.org/rfc/rfc5382.html#section-4.2)

由此得到的候选流程是：保留现有 UDP 会话 → 通过受保护控制消息协调新的 fake TCP 尝试 → 完成独立 TCP 映射与握手 → 在原 QUIC 连接验证新路径 → 再按获准策略使用。任何阶段失败都不能把“UDP 已通”记为 fake TCP 成功。**这是依据规范和本地接口的推断，尚无实际网络验证。**[TCP NAT 规范](https://www.rfc-editor.org/rfc/rfc5382.html#section-4.2)、[QUIC 路径验证](https://www.rfc-editor.org/rfc/rfc9000.html#section-8.2)

## 4. 与当前 iroh fork 的可能衔接

当前 fork 的 `CustomEndpoint::poll_recv` 与 `CustomSender::poll_send` 处理包和自定义地址；`Endpoint::open_data_transport_path` 接收**原有已准入 Connection** 和 `TransportAddr`，内部按 `remote_id/stable_id` 找到原连接并调用 `open_path_ensure`，不拨号替代连接；真正建立要观察 `Connection::paths`。这些是当前本地源码事实，不代表上游稳定 API，也不证明 fake TCP 已实现。[custom transport 接口](../../vendor/iroh/src/socket/transports/custom.rs#L125)、[公开路径接口](../../vendor/iroh/src/endpoint.rs#L1132)、[原连接路径建立](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1603)

**推断：** 类似 fake TCP 的实现可以放在 QUIC 之下，作为独立 custom 数据报路径，封装原 QUIC 包；原会话的身份、密钥和 stream 不需由 fake TCP 重新实现。QUIC 的拥塞控制仍需保留，fake TCP “没有拥塞控制”并不意味着 miopunch 的业务可以无限发送；当前 fork 已在每条 QUIC path 保存拥塞控制器和验证状态。[本地 path 状态](../../vendor/noq-proto/src/connection/paths.rs#L168)、[Connection 拥塞观察](../../vendor/iroh/src/endpoint/connection.rs#L1091)

因此原连接活着且新载体通过验证时，保留同一会话在接口上有依据；已有流是否连续、超时与资源策略是否正确，仍须另行验证。当前 `tcp_transport.rs` 用内核 TCP socket 和 stream 分帧，参考 fake TCP 原理意味着新增数据报载体与用户态协议处理，不能靠现有 `TcpStream` 的一个配置开关完成。[现有 TCP socket](../../src/tcp_transport.rs#L667)、[现有 TCP stream](../../src/tcp_transport.rs#L765)、[原连接关闭即终止的接口约束](../../vendor/iroh/src/endpoint.rs#L1132)

## 5. 边界与未验证事项

1. **权限和系统路径。** Phantun 示例通过 TUN、内核转发及 SNAT/DNAT 发送和收取外层 TCP，非 root 运行仍要求 `CAP_NET_ADMIN`。Linux TUN 是用户态读写 IP 包的接口，不是普通 UDP socket；载体内部 TUN 与业务虚拟网卡是不同职责。本文没有证明 Android 普通应用、浏览器、Windows 或 macOS 可直接照用这种系统路径。[Phantun 用法](https://github.com/dndx/phantun#usage)、[Linux TUN 文档](https://docs.kernel.org/networking/tuntap.html)
2. **放行范围。** 项目目标是 L3/L4 NAT/防火墙，不支持 L7 代理；fake TCP 不因此成为 TLS/HTTPS。Linux TCP conntrack 还检查状态、SEQ/ACK 与窗口；推断：只生成 TCP 头不足以保证任意设备或网络放行，乱序、持续丢包、RST 和状态寿命均可能影响通路。[Phantun 范围](https://github.com/dndx/phantun#overview)、[Linux TCP 检查](https://github.com/torvalds/linux/blob/v6.12/net/netfilter/nf_conntrack_proto_tcp.c#L482)
3. **安全与 MTU。** Phantun 本身不提供加密或 anti-replay；推断：miopunch 应继续依赖原端到端 QUIC 包保护，fake TCP 握手不能替代身份认证或准入。无 TCP options 的数据包头为 IPv4 20 + TCP 20 字节，IPv6 40 + TCP 20 字节；其 IPv4 设置 DF，项目不做 IP 分片重组。QUIC 载体仍需交付至少 1200 字节的数据报，实际 MTU 不能只沿用 UDP 路径数值。[项目安全边界与 MTU](https://github.com/dndx/phantun#compariation-to-udp2raw)、[报文头构造](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/packet.rs#L7)、[QUIC 尺寸要求](https://www.rfc-editor.org/rfc/rfc9000.html#section-14)

本次仅核查源码、规范和现有接缝，未构建、测试、调整内核或发送实验流量；没有公网穿透、运营商限速改善、跨 NAT 成功率、长连接不中断或跨平台可用性的实测结论。此前[传输选型研究](2026-09-27-transport-fit-research.md)中“优先集成现成 fake TCP”的建议只是历史研究意见，不用于覆盖本次“参考原理”的需求，也不构成新的实施决定。

## 6. 简短追查：NAT3 的记忆来源与 raw IP 能否简化 TCP 打洞

### NAT3 是较严格的代表场景，不是 TCP 的数字门槛

找到了与用户记忆接近的原文：旧[台架章程“主覆盖集”](../../archive/legacy-go-openspec-2026-09-28/docs/decisions/p0-nat-lab-charter.md)写的是 **NAT3 可作为 NAT1／NAT2／NAT3 这组 easy-like 条件中更保守的代表**，并非“TCP 至少 NAT3 才能打洞”或“NAT3 必然成功”。这里 NAT3=`EIM + APDF`：同一内部端点跨目的地保持外部映射，但回包只放行已发送过的精确对端 IP:port；NAT1／2 的过滤更宽松。当前[NAT3 验证](2026-09-29-nat3-apdf-validation.md)证明的是固定映射 UDP fixture，不能移作 TCP 的分类保证。

TCP 还要检查自己的映射及状态机：RFC 5382 要求 TCP EIM 和 simultaneous-open，但明确 TCP 过滤可独立于 UDP，也记录设备错误阻止交叉 SYN 或 SYN-ACK 的情况。因此 **UDP 被叫作 NAT3，不足以推出 TCP 能通；普通 TCP 打洞的友好条件是 TCP 映射稳定、双端主动发 SYN、允许相应握手状态，及早到 SYN 的处理不破坏后续尝试。**[RFC 5382 §4.1–4.3](https://www.rfc-editor.org/rfc/rfc5382.html#section-4)

仓库[现有真 TCP 证据](2026-10-02-tcp-punch-validation.md)限于 Linux IPv4、端口保持映射、静默丢弃未获准 SYN、支持 simultaneous-open 的真实双 NAT fixture；目的地相关端口改写是有界失败。旧验收中还有 TCP `diag-fail-allowed`，其 gate PASS 不能当作穿透成功率。[旧验收对照](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md)

### 用户态 IP/TCP 可减少本机 socket 约束，网络条件仍需满足

此处将“基于 IP”解释为**用户态构造外层 IP/TCP 包并自行处理握手**，不是绕开公网路由或 NAT。以下是基于原理的推断，尚无本仓库 fake TCP 实测：

| 可自行控制的部分 | 不能因此获得的能力 |
| --- | --- |
| 固定源端口，自己安排 SYN 时机、TTL、重试和 crossed-SYN 状态转换；减少依赖 `listen/connect` 同端口复用及内核 socket 自动终止的约束 | 不能指定远端 NAT 的外部端口；目的地相关或不可预测映射仍需另解候选问题 |
| 自行生成 SEQ/ACK 和 SYN-ACK；不必以完整内核 TCP stream 作为数据载体 | 不能强迫 NAT 接受某种 SYN／SYN-ACK 包序或错误窗口；多层 NAT/CGNAT 的每层状态仍需成立 |
| 握手后维持 fake TCP 数据报语义，让 QUIC 保留端到端可靠性与拥塞控制 | 仍需 TCP 独立候选和握手，不能复用 UDP 的放行状态 |

普通 socket 打洞的同端口复用、主动／被动连接竞态在原始 P2P 论文 §4.1–4.4 中有专门分析；Phantun 的包构造和用户态状态机证明这类控制可以从 socket API 移至包处理层，Linux conntrack 的 TCP 状态及窗口检查则仍在网络路径上。[Ford 等原始论文](https://www.usenix.org/legacy/events/usenix05/tech/general/full_papers/ford/ford_html/)、[Phantun 状态机](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/lib.rs#L251)、[Linux TCP conntrack](https://github.com/torvalds/linux/blob/v6.12/net/netfilter/nf_conntrack_proto_tcp.c#L482)

历史上确有主动构造 SYN-ACK 的 NATBlaster 方法；原论文 §5.5 的双端协调方案及 §7 的实现说明证明这个思路并非新猜想。但其方法还有端口预测、SEQ 信息交换、TTL／错误报文等前提；原论文也另列由 helper 伪造源地址的替代方案，不能把这些前提自动套给 Phantun 式 P2P。后续原始测量论文同样指出 raw 方法会受 NAT 的包序和序号行为影响。[NATBlaster 原论文](https://natblaster.sourceforge.net/paper/natblaster.pdf)、[Guha/Francis 原始测量 §2](https://www.usenix.org/legacy/event/imc05/tech/full_papers/guha/guha_html/index.html)

**结论：能简化“本机如何精确发包与处理握手”，不能据此简化为“NAT3 一律可通”，也没有证据证明总体实现更简单或 NAT 覆盖更广。** 还会增加系统权限、路由/TUN、避免本机内核干扰或 RST、校验和与状态维护成本；当前 Phantun `SynSent` 不处理 crossed SYN，收到 RST 也直接失败，不能把“用户态有能力自定义”说成“它已经实现”。[固定版本源码](https://github.com/dndx/phantun/blob/6256f630a1e6f91489805669afc9700c1289a48e/fake-tcp/src/lib.rs#L251)、[项目系统路径](https://github.com/dndx/phantun#usage)

候选观察也必须来自**实际 fake TCP 源端口及其发送路径**；普通内核 TCP STUN socket 即便端口数字相同，也不自动证明 TUN／用户态包所经 SNAT 与映射相同。此项是系统路径差异的推断，需要后续单独验证。上述研究轮次没有改变官方 UDP QAD 主路径、STUN 备用定位、TCP 显式启用方式或现有流程；用户后续安排见本文开头的关联任务。
