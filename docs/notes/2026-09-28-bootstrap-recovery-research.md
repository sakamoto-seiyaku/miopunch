# 小型网络的引导与重连：成熟 P2P 实践调查

日期：2026-09-28。状态：研究笔记，不是正式规格或实现任务。资料限 BitTorrent BEP、libtorrent 官方文档，以及同轮核查的 iroh 官方资料。libtorrent 在线参考文档本次标示版本为 2.1.0。

## 结论

**用户提出的“恢复已知联系人和成功路径，失效时通过其他联络渠道恢复，必要时使用稳定引导入口”有成熟 P2P 实践依据。** 这里需要设计的是小型私人网络的联系人维护与恢复过程，不需要先发明另一套发现协议，也不需要引入完整全球 DHT。

本轮继续沿用已有前提：产品已经为网络提供可用的初始联络途径，普通用户无需租服务器、填写 broker/topic 或编排打洞。以下研究回答系统如何使用和维护这些途径，不重新质疑这个场景前提。

## 五个可借鉴事实

1. **重启后利用保存的联系人，是标准行为。** BEP 5 要求 DHT routing table 跨客户端运行保存，并在启动后从已知节点继续探索。libtorrent 的 `session_state()` 也保存 DHT routing table，供下次 session 恢复。[BEP 5: Routing Table](https://www.bittorrent.org/beps/bep_0005.html#routing-table)、[libtorrent session_state](https://libtorrent.org/reference-Session.html#session_state())

2. **“认识”和“近期可达”分开维护。** BEP 5 区分 good、questionable、bad，依据近期响应、活动和连续失败更新；优先使用已确认可响应的联系人，旧节点先探测再替换。这里的 good 是协议活跃性，不能等同 miopunch 的成员信任或访问授权；具体 15 分钟等参数也不必照搬。[BEP 5: Routing Table](https://www.bittorrent.org/beps/bep_0005.html#routing-table)

3. **成功连接过的业务 peer 地址也会保存，bootstrap 是备用来源。** libtorrent resume data 保存上一轮连接的 IPv4/IPv6 peer 地址；另有 `dht_bootstrap_nodes`，官方明确将它们定义为没有已知节点时的备用节点。缓存路径与稳定引导并存，并不矛盾。[libtorrent resume 文件中的 peers / peers6](https://libtorrent.org/manual-ref.html#file-format)、[dht_bootstrap_nodes](https://libtorrent.org/reference-Settings.html#dht_bootstrap_nodes)

4. **恢复一部分连接后，可以由已有 peer 更新联系人。** BEP 11 的 PEX 在通过 DHT / tracker 等完成初始引导后提供另一种发现来源，减少持续查询其他入口的需要。它通常只传播已经成功连接的 peer，并报告断开，而不是无限转发一张从未验证的旧名单。[BEP 11: Peer Exchange](https://www.bittorrent.org/beps/bep_0011.html)

5. **失败需要超时、退避和再次尝试的条件。** libtorrent 设置连接超时、连接尝试速率和失败次数；重连等待随 failcount 增加，连接成功会清零。它不是永远高速重试，也不是一次失败就永不再试。本轮引用的是其具体等待策略，不把它泛化为所有 P2P 都采用指数退避。[libtorrent max_failcount / min_reconnect_time / peer_connect_timeout](https://libtorrent.org/reference-Settings.html#max_failcount)

这些事实支持复用一套成熟的生命周期思路；**不意味着把 libtorrent 作为 miopunch 依赖**。BEP 5 的 XOR 路由、bucket、全球节点空间及 torrent 查询语义，是其大规模 DHT 的需要，不是上述小型联系人维护模式的前提。

## 映射到当前场景

以下是从资料推导的建议，不是声称 BitTorrent 本身使用 MQTT / IRC：

```text
恢复本地身份、已知成员与联系线索
                  |
      尝试近期成功的直连地址 / 中继路线
                  |
   需要新线索或协商时，使用已有成员或消息渠道
                  |
   缓存联系人不可用时，启用产品提供的引导入口
                  |
       找到可联系成员，建立安全数据连接
                  |
       更新成功记录，继续必要的节点维护
```

这是尝试优先级，不要求等待所有旧地址逐个超时以后才启用其他渠道。可以给缓存尝试一个有限时间预算，再开展有界的备用尝试；具体并发、预算和退避参数后续实测决定。

重连缓存可以利用已知节点身份、最近成功的联络地址或中继线索、最后验证时间、近期失败状态，以及可用的备用入口。这只是描述恢复尝试需要参考的信息，不限制整个网络可以保存和同步哪些状态，也不重新引入“最小通讯录”的约束。**持久保存的是线索与成功经验，不能保存“它永远在线”或“这条 NAT 映射永远有效”的结论。** 接收到新地址后仍需验证实际可达性和对端身份；联系人失联也不应等同其成员资格消失。

已有安全连接可以帮助补充联络信息。MQTT / IRC 等消息渠道可以帮助找到成员并交换当次建连信息；其成功只推进到“双方能交流”，后面仍需打洞或实际中继得到可用数据路径。不同途径应该汇入同一个连接过程，而不是各自长出一套信任规则。

## 三个必须分开的状态

| 观察到的情况 | 合理含义 |
| --- | --- |
| 信令入口可用，暂时无人响应 | 入口正常，但尚未联系到成员；不能单凭无响应宣布全网成员全部离线 |
| 信令入口失效，但某条缓存直连 / 中继路线仍成功 | 仍可联系网络中的节点，不应因为一个引导服务异常阻止已有路线工作 |
| 能与目标交换信令，但打洞失败且无可用中继 | 已联系到目标，但数据连接尚未建立 |

如果现实中其他成员都离线，稳定入口也不能把它们变成在线设备。返回的第一个成员可以保留本地视图、完成自身联络准备并等待其他成员恢复；这与入口本身故障是不同问题。反过来，未收到回应可能是离线、路径问题或过期线索，产品应表达自己已知的事实。

## iroh 兜底边界

同轮 iroh 核查表明，其 address lookup 负责由 EndpointId 解析当前 relay / 直连地址，典型方案使用签名 Pkarr 记录，经 DNS / HTTPS 查询，并可选择 Mainline DHT。这提供成熟的联络信息解析能力，不等于替 miopunch 定义私人网络的成员与恢复策略。[iroh Address Lookup](https://docs.iroh.computer/concepts/address-lookup)

iroh 公共 relay 官方定位于开发 / 个人爱好用途，有速率限制、没有 uptime SLA，并只正式支持最新稳定版本，旧版本兼容可能撤去。它不应直接被写成产品已经获得的永久稳定服务保证。[iroh Public Relays](https://docs.iroh.computer/iroh-services/relays/public)

原生 relay 的 `send_packet` 在目标客户端未连接时丢弃报文，不是持久离线邮箱或 topic 存储。**“稳定引导入口”可以是产品承担的一项持续职责，但不能由此推断官方 relay 已提供持久消息与长期可用性。** 具体入口部署仍按用户已有的产品前提另行选择。[iroh v1.2.0 relay 转发源码](https://github.com/n0-computer/iroh/blob/v1.2.0/iroh-relay/src/server/clients.rs#L200)

## 对下一步的建议

先沿“老成员返回”场景补足缓存恢复和联系人维护，而不是扩大到新协议：有成功线索就尝试，线索过期或路径失效就利用已提供的其他联络入口，恢复联系后更新缓存。用“缓存可用”“缓存全部失效但入口可用”“入口失效但缓存路径可用”“只有自己上线”四个小变化检验流程即可。

文档核查只验证成熟设计先例与职责边界，没有测量真实重连延迟、NAT 穿透率或入口可靠性。尚未决定缓存大小、有效期、重试参数和任何 wire format。
