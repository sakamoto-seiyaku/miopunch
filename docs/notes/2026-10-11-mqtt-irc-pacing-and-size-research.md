# MQTT / IRC 发送间隔与报文大小重新核查

2026-10-11，用户要求重新确认 MQTT / IRC 的合理发送间隔和大小，质疑先前
IRC 每片 600 ms 是否过分。本记录区分协议、软件默认、实际服务观察和建议；
研究建议不是新增产品决议。依据仍为 [ADR 0005](../adr/0005-side-channel-control-bootstrap.md)
和 [ADR 0009](../adr/0009-unified-session-and-capability-based-paths.md)：公共消息
载体限于联系与受保护控制引导，不获得普通业务承载资格。

## 已确认的结论

- **MQTT 没有协议规定的逐包 600 ms 等待，当前适配器也没有同类固定发送睡眠。**
  MQTT 可以受 broker 的包长、发布频率、字节率、队列及连接频率限制；这些是
  具体服务或实现的限制，不能理解成 MQTT 协议要求。
- **先前 IRC 的 600 ms 是主动选择的实验参数，不能称作 Ergo 强制间隔。**
  10 月 3 日原记录已明确写出这个边界；当天实际 testnet 的 `005` 已公布
  `SAFERATE`，先前推导公网 pacing 时没有利用这一关键证据。
- **Ergo 确有服务端 fakelag，但不等于客户端每片都要等待。** 上游默认允许
  五个命令突发，之后服务端按每秒两个命令处理；闲置超过两秒后重新允许突发。
  此默认同时适用于核查的 2.7.0 和当前稳定版 2.19.1，不证明 testnet 的实时配置。
- **大小必须按完整 wire 长度计算。** MQTT 的 1200–2048 B datagram 可放在
  一次约 1.3–2.1 KiB 的 PUBLISH 中；IRC 的 512 B 常规行限制包含前缀、目标和
  CRLF，不能把 512 B 直接当正文或原始二进制载荷额度。

本地依据：[600 ms 原始观察及边界](2026-10-03-browser-webrtc-replan.md)、
[当天完整 ISUPPORT 回执](../evidence/browser-webrtc-replan-2026-10-03/irc-entry-observation.json)、
[当前 MIR1 编解码与 profile](../../src/irc_datagram.rs)。外部依据见下节。

## MQTT：协议事实与服务配置

OASIS MQTT 3.1.1 的 Remaining Length 字段最多编码 268,435,455 B，这是可变
头和 payload 的长度上限，不是公共 broker 承诺的 payload 额度。QoS 0 不等待
PUBLISH 确认；QoS 1 允许多个不同 packet identifier 的消息同时在途。
这两个流程没有规定每次 PUBLISH 之间必须睡眠固定时长。
[OASIS 3.1.1 §2.2.3、§4.3](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html)

MQTT 5 的 `Maximum Packet Size` 是整个 MQTT Control Packet 的限制，含 header；
服务器在 CONNACK 中公布的值约束客户端，客户端 CONNECT 中公布的值约束
服务器。没有公布时仍受协议编码上限和实际实现能力限制。`Receive Maximum`
限制同时尚未确认的 QoS 1 / 2 publication 数量，不能限流 QoS 0；它不是
每秒消息数，也不是逐包等待时间。MQTT 5 还允许用 quota、包过大、连接率
超限等原因码报告服务拒绝。
[OASIS 5.0 §3.2.2.3.3、§3.2.2.3.6、§4.9](https://docs.oasis-open.org/mqtt/mqtt/v5.0/os/mqtt-v5.0-os.html)

EMQX 当前文档把最大包长默认值列为 **1 MB**。这只是软件默认值；部署者可覆盖。
[EMQX MQTT 配置](https://docs.emqx.com/en/emqx/latest/guides/configuration/mqtt.html)

EMQX 的发布消息数和字节数 limiter 可以按客户端和 listener / node 配置；
listener 过载可暂停读客户端，node 限流可以丢弃 QoS 0、以 quota 原因拒绝
QoS 1 / 2。subscriber delivery 限流也可能丢弃 QoS 0。这个行为说明 socket
写入成功并不能证明对端成功收到；文档里的数值例子不是公共服务的限额。
[EMQX rate limit](https://docs.emqx.com/en/emqx/latest/guides/rate-limit.html)

`broker.emqx.io` 的当前官方页面把它定位为原型、学习和测试的公共 broker，
列出 TCP / TLS / WS / WSS / QUIC 入口。页面和公共 dashboard 未公布可核查的
逐客户端数字配额。因此本轮**不能确认此服务必定允许 1 MB、1000 msg/s 或
任何固定速率**，也不能用宣传中的 Cloud 套餐额度替代公共 broker policy。
[EMQX 公共 broker](https://www.emqx.com/en/mqtt/public-mqtt5-broker)、
[官方 dashboard](https://www.emqx.com/en/mqtt-dashboard)

当前项目使用 MQTT 3.1.1 / QoS 0，所以 MQTT 5 的属性不是本适配器已取得的
协商结果。主代理源码审计确认：MIO1 封套 20 B、topic 53 B，完整 QoS 0
PUBLISH 在 datagram 1200 B 时为 1278 B、2048 B 时为 2126 B；未含 WebSocket、
TLS、TCP / IP 开销。Browser MQTT 另有 4096 B 收包限制、有界待收缓冲和队列，
Native 使用事件循环和队列背压；这些是内存和处理界限，不是逐包静态睡眠。
原先的相同 wire 推导见 [10 月 3 日记录](2026-10-03-browser-webrtc-replan.md)。

另需区分 MQTT discovery 和 datagram carrier。Discovery 查询后有 2 s 候选
收集窗口，最多收集 16 个候选；这是一次发现阶段的等待，不是每次 PUBLISH
之间等待 2 s。Discovery 的 1024 B packet cap 对应小型查询 / 候选消息；
carrier 的应用 datagram cap 为 2048 B、原生 MQTT packet cap 为 4096 B。
5 s keepalive 和 8 s 订阅阶段 timeout 也不决定 datagram 逐包发送间隔。
[discovery 实现](../../src/mqtt_discovery.rs)、[carrier 实现](../../src/mqtt_transport.rs)

## IRC：512 B、fakelag 与 SAFERATE

RFC 2812 §2.3 的常规消息上限是 **512 B，含 CRLF**；服务器转发消息会加入
发送者前缀。IRCv3 message-tags 为 tags 提供独立额度，正文相关部分仍是
512 B。不能因为收到较大的 CAP / tag 或 multiline 能力广告，就向未经共同
支持的正文发送超长 MIR1。
[RFC 2812 §2.3](https://www.rfc-editor.org/rfc/rfc2812.html#section-2.3)、
[IRCv3 message-tags size limit](https://ircv3.net/specs/extensions/message-tags#size-limit)

`LINELEN` 不是本轮核查出的普遍已批准 IRCv3 协商机制：IRC 定义表将其标作
proposed，IRC++ 的 2026-05-05 文档还标作 deprecated。它只有在实际服务器
广告、适配器明确支持、双方收包能力确认时才有意义；不能凭 Ergo 软件名
或 multiline 广告推断可以扩大本项目的 512 B parser。
[IRC 定义表](https://github.com/ircdocs/irc-defs/blob/gh-pages/_data/isupport.yaml)、
[IRC++ LINELEN](https://ircpp.net/specs/linelen/)

Ergo 2.7.0 和 2.19.1 的上游 `default.yaml` 同为 fakelag enabled、window 1 s、
burst-limit 5、messages-per-window 2、cooldown 2 s。默认是服务端允许短突发，
超出后睡眠至上一条处理时间相距至少 500 ms，超过 cooldown 后重新允许突发。
这不是“每个 IRC 包必须等 600 ms”的规则。
[Ergo 2.7.0 默认配置](https://github.com/ergochat/ergo/blob/v2.7.0/default.yaml)、
[Ergo 2.19.1 默认配置](https://github.com/ergochat/ergo/blob/v2.19.1/default.yaml)

两版 `client.go` 都在已注册客户端运行命令前调用 fakelag；`PRIVMSG` 计入。
`Touch` 按命令数及上次时间计算，不按正文 byte 数量计算罚时。当前版本新增
少数命令的豁免预算，默认豁免表没有 `PRIVMSG`。因此在行限额内合并更大的
片可以减少命令开销。源码证明的是具体实现行为，不是 testnet 实时配置。
[2.7.0 client.go](https://github.com/ergochat/ergo/blob/v2.7.0/irc/client.go)、
[2.7.0 fakelag.go](https://github.com/ergochat/ergo/blob/v2.7.0/irc/fakelag.go)、
[2.19.1 client.go](https://github.com/ergochat/ergo/blob/v2.19.1/irc/client.go)、
[2.19.1 fakelag.go](https://github.com/ergochat/ergo/blob/v2.19.1/irc/fakelag.go)

**SAFERATE 改变客户端应该如何处理突发。** 在 IRCv3 的 PR #556 中，该 token
承诺不会因 server rate limiting 断开客户端；实现可通过 fakelag 延迟处理
和有界输入队列来保持此承诺，客户端可关闭自身保守 rate limiting。它不承诺
零延迟、不提供数值配额、不授予滥用公共资源的许可。2026-10-11 读取 GitHub
API 确认该 PR 仍 open / 未合并，文件标记 work-in-progress，head 为
`3f38dbbf1d05b165002b814f2bf14d94caf0e9a9`；应按实际实现与广告来使用，
不能称作已批准规范。
[PR #556](https://github.com/ircv3/ircv3-specifications/pull/556)、
[核查的提案正文](https://github.com/ircv3/ircv3-specifications/blob/3f38dbbf1d05b165002b814f2bf14d94caf0e9a9/extensions/saferate.md)

Ergo 2.19.1 在生成 ISUPPORT 时无条件加入空值 `SAFERATE`，同时仍存在上述
默认 fakelag。两项源码放在一起明确说明 `SAFERATE` 不等于关闭服务端节流。
[Ergo generateISupport](https://github.com/ergochat/ergo/blob/v2.19.1/irc/config.go#L1849)
Halloy 官方客户端配置也将自身 anti-flood 用于没有 SAFERATE 的服务器。
[Halloy anti_flood](https://halloy.chat/configuration/servers.html#anti_flood)

testnet 官方说明允许非滥用的 Ergo / IRCv3 测试，通常运行近期 master，并记录
全部 traffic 供调试。官方页没有数值消息配额。软件稳定版配置、10 月 3 日
旧 ISUPPORT 都不能代替一次新连接的实际服务声明。
[Ergo testnet 使用说明](https://testnet.ergo.chat/)

## 当前 MIR1 大小推导

主代理审计确认当前 MIR1 fixed header 为 **94 B**，原始 byte 以 hex 展开为
两倍正文长度。128 B chunk 产生 350 B 正文；1200 B datagram 需要 10 片。
当前 reassembler 正文上限 400 B，因此即使 wire 有空间，在该编码下最多也
只能容纳 `floor((400 - 94) / 2) = 153 B` 的一片；现有 decoder 又明确要求
chunk 为 128 B。**153 B 是后续一致升级才可能使用的候选，不是当前协议已支持。**
[当前 MIR1 编解码](../../src/irc_datagram.rs)

如果 line 上限 `L` 包含 CRLF，`P` 是不含冒号和空格的 `nick!user@host` 长度，
`T` 是 target 长度，转发的 `:<prefix> PRIVMSG <target> :<body>\r\n` 的固定开销
为 `P + T + 14`。保持当前 header / hex 时，候选 chunk 上限应从
`floor((L - P - T - 14 - 94) / 2)` 推导，再和应用正文 / decoder 上限取最小。
每次发送仍须检查完整 byte 长度；host cloak、nickname、target 和编码变化
都影响预算。不能只把全局 CHUNK 从 128 改成固定 153 后宣称所有 IRC 服务安全。

本轮实际观察的 sender prefix 为 35 B。若 target 长达 64 B，153 B hex chunk
产生 400 B 正文，转发完整行就是 `35 + 64 + 14 + 400 = 513 B`，超出上限。
在这个具体前缀 / 目标组合下最多应为 152 B；这进一步说明 153 不能直接作为
跨服务固定值。

## 一次有界公网观察

本轮的 [有界合成观察脚本](../evidence/carrier-pacing-research-2026-10-11/irc-bounded-burst.py)
要求两次全新正常 TLS 连接均收到 SAFERATE，才会在两个自有随机 nickname
之间发送数据；预算最多 38 条 PRIVMSG / 20 KiB，没有账号、频道或真实秘密。
第一次在受限 sandbox 内未产生阶段回执，外部收尾终止其所属进程，exit 143。
该次属于未完成观察，实际注册、发送计数、服务器时序和原因均未确认；不能
说成服务限流、DNS 失败或成功。脚本的上限也不能作为实际发送计数。

随后正常网络下的单次观察在 2026-10-11 06:21:10–06:21:22 UTC 完成。
两连接正常验证 TLS 证书，注册耗时 790 / 764 ms，两者均新收到 SAFERATE。
实际共发送 38 条数据 PRIVMSG、14002 B；片间人为睡眠为 0。四组都是同一个
合成 1200 B datagram，全部逐 byte 一致到达；发送者 prefix 35 B、私信 target
14 B。结果如下，完整行大小均含 CRLF。
[公网原始回执](../evidence/carrier-pacing-research-2026-10-11/irc-public-burst.json)

| Chunk / 片数 | 两方向提交至 socket 时间 | 首末到达跨度 | 发起至完整到达 | 最大收到完整行 |
|---|---:|---:|---:|---:|
| 当前 128 B / 10 片 | 0.211 / 0.202 ms | 3004 / 2507 ms | 3357 / 2857 ms | 413 B |
| 候选 153 B / 8 片 | 0.121 / 0.111 ms | 1503 / 1502 ms | 1855 / 1851 ms | 463 B |

此观察确认当前服务允许上述小型突发，不要求客户端逐片等 600 ms；仍有明显
服务端处理间隔，与上游默认约 500 ms 的 fakelag 行为一致。这里的“一致”是
推论，未读取服务器配置，不应改称已确认每秒精确配额。Burst 余量和上组消息
结束后闲置时间影响两组耗时；这不是随机化基准或吞吐上限实验。

回执的功能字段 `passed=true`，但两连接 `wait_closed()` 均得到 `SSLError`，
最终进程 exit 1，不能称作所有检查及正常 TLS 关闭通过。实际进程已结束，
没有为此重复发送公网测试流量。该次用 TLS-to-TLS 的 direct nickname 投递，
没有 WSS、项目 channel 路由、真正 MIR1 reassembler、QUIC / TLS 内层或完整
SDK 准入验证；153 B 明确不兼容当前 SDK decoder。原先约 229 秒的完整慢路径
也不能仅凭这次合成观测推断会降至以上秒数。

## 条件性建议与待验证边界

1. **MQTT 继续按一个完整 QUIC datagram 对应一个 PUBLISH**；当前 1200–2048 B
   范围有明确 wire 预算，不需要为模仿 IRC 拆成 128 B 或追加 600 ms 睡眠。
   最大 datagram、全部队列和 socket pending bytes 保持有界。若获得 MQTT 5
   限额，遵守双方属性；MQTT 3.1.1 依据服务 policy 与实际收发回执，不能把
   QoS 0 写入成功当成投递成功。
2. **新连接实际收到 SAFERATE 时，可以取消每片人为保守等待，发送有界的
   当前控制报文突发，让服务器 fakelag / TCP 背压承担实际节流。** 合理间隔
   是“不叠加人为固定 600 ms”，不是承诺服务端 0 ms。需要按同一 service
   connection 汇总所有控制会话的消息数、byte 数、待发量和尝试预算，防止
   QUIC 重传、并行请求持续扩大队列；取消、权限变化和 deadline 仍然有效。
3. **没有 SAFERATE 时按实际服务文档或配置的速率执行。** 对确认使用默认
   Ergo fakelag 的自有设施，五命令 burst / 之后两命令每秒有来源；500 ms
   是该配置的长期服务间距，不是所有公共 IRC 的全局默认。未知服务没有
   可由这次研究推出的普遍安全 10 / 100 / 600 ms 数值。
4. **IRC 先利用当前行预算，随后评估统一升级编码 / chunk / decoder。**
   153 B 比 128 B 只让 1200 B datagram 从 10 片降到 8 片，节省命令开销，
   但没有消除 hex 膨胀与服务端节流。Base64 等紧凑编码需要独立 wire 版本、
   双端一致性和真实服务器完整往返验证，不能从格式计算直接算作实现成功。
5. **用有界合成观测确认服务当次行为，完整 SDK 流程确认产品效果。**
   一次少量 owned-nick / owned-channel 往返可判断是否存在明显客户端过度
   等待、实际行长和 fragment 完整性；不能据此宣布公共额度、吞吐上限、
   長期大规模合理性或完整准入资格。长期依赖仍需要运营方 policy / 授权，
   不应用不断提高测试速率来搜索公共服务最大承载。

本轮研究没有更改产品 pacing、fragment 编码或默认公共资源列表；600 ms
应保留为明确的慢速故障 / 回归配置，不能继续描述为当前 Ergo 强制限流。
