# MioCube 局域网与 IPv6 连接讨论稿

2026-11-26，临时讨论记录。范围是已准入原生设备的离线局域网互连，以及 IPv4、IPv6 的直连和穿透。 [现行连接设计 ADR 0019](../adr/0019-miopunch-application-on-p2panda-and-iroh.md) · [现行成员业务连接 ADR 0020](../adr/0020-shared-member-business-connection.md)

## 本轮讨论结论

- **双栈共用一套流程。** 同一对成员继续共用一条业务 QUIC 会话。IPv4 TCP、IPv4 UDP、IPv6 TCP、IPv6 UDP 各自保留候选和已验证路径的历史。候选筛选、预算、协商、重试、换网后的刷新和路径验证共用现有成员连接模块。TCP 和 UDP 分别按自己的传输方式拨号或打洞，地址族决定用哪组地址和实际绑定。 [成员连接约定](../adr/0020-shared-member-business-connection.md) · [成员联系历史](../../src/member_contacts.rs#L12)
- **两种地址族的合格直连同档。** 首条合格路径就绪即可开流。两条直连都可用时看实际连接质量，沿用 IPv6 的 3 ms RTT 偏好，以及同档换路的 5 ms 防抖门槛。IPv4 直连先就绪时可以先承载业务，IPv6 取得验证后再在原会话上参与选路。 [现行路径选择](2026-11-24-simple-direct-path-selection.md#线上选路需要知道什么)
- **增强探测的暂停按地址族隔离。** 增强 IPv4 只暂停同族内它接替或争用的常规探测，IPv6 可以继续开新轮和验证。IPv6 将来有增强尝试时应用同一条规则。现有增强 UDP 对应同族的普通 UDP 探测，TCP 互拨按自己的探测和暂停条件执行。当前 IPv4 暂停分支会推迟客户端开启新的 QNT 轮，这就是需要核对和收敛的耦合。 [增强轮暂停入口](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L739) · [当前 QNT 调度](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1642)
- **普通离线 LAN 复用现有 mDNS。** 首先覆盖同一条可收发组播的本地链路。两台已准入设备在断公网后能互相发现并完成真实业务往返，就满足这轮讨论的 LAN 目标。

## 断公网后的局域网互连

原生端已有 mDNS，默认关闭。`Active` 发布节点身份和地址，也能发现对端。`Passive` 只发现。两端都设为 `Active` 时，成员拨号可以按已知节点身份查找并尝试发现的 IP，随后沿用现有身份验证和业务连接。mDNS 的作用范围是同一条可收发组播的本地链路，通常对应同一 VLAN。同一台路由器桥接的网线和 Wi‑Fi 可以互相发现，仍需允许客户端之间的单播才能直连。 [原生 mDNS 接线](../../src/native_network.rs#L835) · [成员拨号](../../vendor/iroh/src/endpoint/member_fusion/dial.rs#L418) · [RFC 6762](https://www.rfc-editor.org/rfc/rfc6762.html#section-3)

普通 LAN 先验证现有能力。两台已准入设备开启 `Active`，接到同一台路由器，断开 WAN，并在缺少有效地址缓存的情况下冷启动。确认两端自动发现彼此，且实际完成一次文件或终端业务。现有验证记录覆盖了发现后发起准入请求。只有 `fe80::` 地址的 LAN 有单独的接口范围问题，当前接收端由 IP 和端口重建地址时未带接收接口。 [现有验证](2026-10-01-resource-policy-validation.md#explicit-lan-disclosure-and-upstream-branch-differences) · [mDNS 依赖源码](https://docs.rs/iroh-mdns-address-lookup/0.5.0/src/iroh_mdns_address_lookup/lib.rs.html#525) · [RFC 4007](https://www.rfc-editor.org/rfc/rfc4007.html#section-6)

显式指定对端 IP 和端口适合组播受限但单播可达的网络。无法直连、双方可达同一内网服务器时，可以配置获准的自建 relay。路由器端口映射默认关闭，用途是允许路由器外部拨入。 [iroh 地址配置](../../vendor/iroh/src/endpoint.rs#L801) · [中继配置](../../src/resources.rs#L333) · [端口映射](2026-10-01-udp-portmap-validation.md#configuration-and-recorded-implementation-choices)

## IPv6 公网直连与穿透

| 能力 | 当前实现 | 讨论建议 |
| --- | --- | --- |
| IPv6 UDP 直连和常规打洞 | 原生 iroh 已在原 QUIC 会话上交换、探测和验证 IPv6 候选。Pixel 6a 现场记录实际选中过 IPv6，期间发生过短暂回落。 | 单独验证双侧 IPv6 网关过滤未经请求的入站 UDP 时，现有 QNT 双向探测能否建立直连。记录首条合格路径和原会话业务结果。 |
| IPv6 TCP 直连 | 现有自定义 TCP 载体支持无接口范围的 IPv6 地址，已有 Linux 回环验证。正式配置目前只有一个 `tcp_listen`，Linux 和 Windows 默认监听 IPv4。 | 优先验证同一原生端点同时提供 IPv4、IPv6 TCP，并且 IPv6 候选能被公布、拨到和实际选中。复用现有 TCP 载体、成员连接和业务流。 |
| IPv6 TCP 打洞 | 现有打洞和从监听端口发起的 TCP 观察只支持 IPv4。 | 当两端均无法被直接拨入时，评估由已认证会话协调双方从实际绑定端口发起 TCP simultaneous open。分别确认目标操作系统的 socket 行为和 IPv6 网关过滤行为。 |
| IPv6 增强 UDP 打洞 | 现有增强轮的地址样本、映射分析和线格式围绕 IPv4。 | 优先复用已认证的协调、候选更新、有限重试和原连接上的路径验证。遇到常规 QNT 无法解决的真实 IPv6 失败，再按成因增强。 |

[IPv6 UDP 成员探测](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1404) · [IPv6 现场记录](2026-11-22-direct-path-failover-research.md#当前现场能说明什么) · [TCP 原生配置](../../src/resources.rs#L474) · [IPv6 TCP 回环验证](2026-10-01-tcp-direct-validation.md#wire-framing-ipv6-and-lifecycle) · [TCP 打洞限制](../../src/tcp_transport.rs#L142) · [增强 UDP 线格式](../../vendor/iroh/src/endpoint/member_fusion/udp/wire.rs#L190)

IPv6 TCP 直连的价值是提供一条可用性和性能不同的路径。持续 UDP 可能在某些网络受限，即使小包 RTT 看起来很好。TCP 载体遇到丢包时也可能发生队头阻塞。对照同一对设备在 IPv6 UDP 和 IPv6 TCP 上的实际文件有效吞吐，以及传文件时的终端回显延迟，再讨论选路是否需要考虑大流表现。可以先在一端允许入站 TCP、两端之间的 UDP 被限速或阻断的场景验证，观察 TCP 能否直接建立业务连接或接替原会话上的流量。 [测量与现行选路](2026-11-24-simple-direct-path-selection.md#线上选路需要知道什么) · [原流迁移到 TCP 的既有验证](../evidence/native-iroh-member-tcp-152.md) · [TCP 队头阻塞](https://www.rfc-editor.org/rfc/rfc9114.html#section-1.1)

常见的全球 IPv6 地址直连失败，首先应区分地址或路由变化、状态防火墙过滤、UDP 策略阻断和实际地址或端口改写。双向出站可能打开状态过滤，持续 UDP 阻断需要 TCP 或可达中继。现有 IPv4 增强算法里的端口预测、随机端口和多 socket 探测针对映射行为，适配到 IPv6 前需要在目标网络上观察到可解释的 IPv6 地址或端口改写。即使复用同一套协商和验证，实现仍可按不同地址族的实际失败选择打法。 [IPv4 映射分析](../../src/nat_mapping.rs#L325) · [IPv6 UDP 过滤](https://www.rfc-editor.org/rfc/rfc6092.html#section-3.2.3) · [IPv6 TCP 过滤与 simultaneous open](https://www.rfc-editor.org/rfc/rfc6092.html#section-3.3.1)

## 首次联系、原会话加路与恢复

1. **首次联系。** 有健康成员会话就复用。需要新拨时，已有的活跃 UDP 地址享有先试窗口，近期验证过的 TCP、UDP 候选可与获准的数据中继并行。仅公告的地址和地址查询目前进得较晚。统一后让当前可拨的双栈候选共用同一次拨号预算。数据中继先就绪时可以先交付业务。只有受限联系入口时，先建立受保护的控制通道，在首条数据路径通过验证后开业务流。 [首拨调度](../../vendor/iroh/src/endpoint/member_fusion/dial.rs#L147) · [受限首拨](../../vendor/iroh/src/endpoint/member_fusion/bootstrap.rs#L14)
2. **原会话加路。** 当前普通 UDP 由 QNT 调度，普通 TCP 另有地址公告与拨号，TCP 打洞和增强 UDP 各有自己的调度和退避。统一后由共享成员连接维护同一组候选与验证结果。已有可拨候选先直接验证。需要双端协调时，优先使用原会话交换候选并调度常规探测。只有观测或路径状态显示需要时，再安排对应地址族的增强工作。路径通过原 QUIC 的验证后参与选路。 [普通 UDP](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1404) · [TCP 候选与打洞调度](../../vendor/iroh/src/endpoint/member_fusion/direct.rs#L434) · [增强 UDP](../../vendor/iroh/src/endpoint/member_fusion/udp.rs#L319)
3. **断路恢复。** 先用仍健康的路径承载业务。原会话还能传控制消息时，在它上面继续建路。需要单独的受限联系时，借获准入口交换候选，回到仍存活的原会话验证新路径。原会话已结束时，回到同一套首拨流程。 [现行连接设计](../adr/0019-miopunch-application-on-p2panda-and-iroh.md) · [原会话恢复](../../src/connection_recovery_core.rs#L281)

## 候选地址额度

| 位置 | 现有额度 |
| --- | ---: |
| 普通 UDP 的 QNT 地址交换 | 默认 32 个，实际受双方协商的额度限制。 [默认值](../../vendor/iroh/src/socket.rs#L147) · [协商实现](../../vendor/noq-proto/src/connection/mod.rs#L7484) |
| 一条 QUIC 连接上的并发路径 | 默认 8 条。 [路径额度](../../vendor/iroh/src/socket.rs#L139) |
| 每个成员的重拨缓存 | 8 个。 [重拨缓存](../../src/member_contacts.rs#L12) |
| 紧急恢复消息 | 最多 8 个候选。 [恢复消息](../../src/connection_recovery_core.rs#L94) |
| 普通 TCP 直连公告 | 一次最多 2 个候选。 [TCP 公告](../../vendor/iroh/src/endpoint/member_fusion/direct.rs#L457) |

先核对候选如何进入有限额度。去重和过滤失效地址后，首批给实际可用的地址族和传输组合留机会，优先纳入近期验证过的地址，其余候选按预算递补。用双栈、多网卡、VPN 和历史地址同时存在的场景，检查 TCP 公告是否足够、恢复时有无候选被同一类地址挤掉。QNT 还需核对满额时换网。当前更新先尝试加入新地址，再移除旧地址。 [当前 QNT 地址更新](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L2876)

## 地址观测和可选 IP

普通 QAD 已能观测 IPv6 UDP 地址和端口。目前增强轮的混合 QAD／STUN 采样和证据编码仍按 IPv4 工作。双栈改造时，观测关联到实际使用的地址族、TCP 或 UDP，以及对应的本地绑定。增强策略根据有状态过滤、前缀改写或端口改写的实际证据选择。 [IPv6 QAD](../../vendor/iroh/src/net_report/report.rs#L103) · [当前增强取样](../../vendor/iroh/src/socket/stun.rs#L183) · [NPTv6](https://www.rfc-editor.org/rfc/rfc6296.html#section-1.2)

地址观测配置的后续设计可以允许 STUN 和 QAD 分别填写可选拨号 IP。填了就拨指定 IP，留空就使用系统 DNS。对 QAD，原观察站域名继续用于 SNI 和证书验证。大陆家宽测试中，同一 UDP socket 分别向 Cloudflare `[2606:4700:49::]:3478` 和 Mullvad `[2a03:1b20:bef1::1]:3478` 完成三轮 Binding，两站每轮回显同一个外部地址和端口。该线路的系统 DNS 把 Mullvad 解析成 `2001::1`，向这个地址发请求未收到回包。Mullvad 作为第二个测试观察点。 [现行 QAD 证书校验](../adr/0019-miopunch-application-on-p2panda-and-iroh.md#L56)

## 后续验证场景

- 双栈都可达时，IPv4 和 IPv6 TCP、UDP 候选都能进入评估，合格路径沿用现有选路规则。IPv6 黑洞时，IPv4 仍能承载首笔业务。
- UDP 被限速或阻断而 TCP 可达时，IPv6 TCP 能交付真实业务，并能在原会话上接替已有路径。
- 一种地址族进入增强探测时，另一种仍能交换或刷新候选、开启普通探测并通过验证。已有健康业务流继续收发。
- 多网卡和旧地址将候选推过现有额度时，真实可达的双栈候选仍得到验证机会。局域网对照组用断 WAN、地址缓存无效的双机冷启动验收。
