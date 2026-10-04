# RTC 数据断路后的原连接维护

本次对应 [WebRTC 第 7 步](2026-10-03-browser-webrtc-replan.md#7-断-rtc-数据保留信令入口)，接续[已保存成员的 MQTT/WSS 起步与放行前 RTC 转换](2026-10-04-browser-wss-member-rtc.md)。实际切断选中的 RTC UDP 四元组后，获准 relay 可以承接原 QUIC M 和原业务 stream；没有获准数据候选时，返回原 M 仍活但不可达。原句柄实际关闭后，载体返回仍只报告这条旧 M 的 terminal。[实施证据](../evidence/browser-rtc-data-cut-2026-10-04/README.md)保留最终源码、执行产物和清理身份。

这次资格范围为同机 Chromium/Linux native、自有隔离网络命名空间、当前已知且材料完整的成员。IWA 继续排除，UI 延后；正式 RTC 后端、默认设施和 TURN 未默认采用。IRC 慢速 profile 的第 2/6 步仍是未资格化停止分支；异网/NAT 成功率和成本按后续步骤推进。

## 原维护协调器与私密交换

native 既有 `connection-path-recovery/1` 交换抽到共享内部 core，browser 从原 `BrowserMemberConnection` 显式调用 `recover_path`。输入仍是原 Connection、该原 peer 的联系地址、当前允许的数据候选、资格检查、caller absolute deadline 和取消信号。没有另外的自动拨号或后台维护循环。

恢复控制连接先通过现有节点 TLS、成员/秘密证明和用途 gate；它只取得受限恢复 scope。对端使用现有 TLS exporter binding 查找同 peer 的实际 live Member M，并复查当前资格。原 M 的普通数据路径继续排除 MQTT/IRC，恢复控制 stream 只交换有界候选与确认；没有业务句柄、业务记录或第二条普通 M。

新路径在原 M 上启动 opaque probe，以实际认证 ACK 达到该次 watermark 为成功依据。信令 ready、路径列表、DataChannel open 和历史 ACK 不替代该证据。实际获准替代路径完成新鲜验证后才退休这条原 M 的旧 RTC lease；独立 M 的 association 不受影响。

## 明确记录的兼容决定

原 ALPN、binding label、候选上限 8 和私密消息上限 4096 字节保持。旧请求的两个字段是 `binding`、`offer`；新可选字段 `require_confirmation` 缺省 false，false 序列化时省略。native 原默认 API 继续发送旧两字段请求；新 confirmed API 和 browser 显式请求扩展。真实 protected endpoint 测试同时验证旧 literal 被现服务器接受、默认 API 的实际 wire 没有第三字段，以及两端不同接收地址时的确认与原 stream 延续。

扩展确认使用同一个受限控制连接上的额外 stream，返回已验证接收地址、新鲜 ACK 布尔值及固定宽度 `u64` watermark；请求者检查该地址属于自己已经允许并发出的 offer，再确认/结束控制交换。两端的 remote receive address 可以不同。机器本地 `usize stable_id` 只留在本机诊断，不跨 native64/WASM32 wire 传递。旧服务端不提供扩展确认时，新请求在原预算内明确失败，不伪造双方已确认。

core 服从 caller 剩余时钟和原三秒 cap；protected contact、候选打开、probe、确认和受限控制清理共享该时钟。browser 从本次维护开始使用同一个 deadline，材料检查也消耗它。cancel 只结束本次维护；RAII 关闭已有受限 control，并取消尚未形成 control 的 contact。显式取消与 120 ms caller 之后，原材料、原 M 和待恢复 stream 保留，后续显式维护可以继续。

一个候选的实际 open 拒绝只结束该候选，协调器继续验证其他获准路径；原 M terminal、当前成员资格或受限 control 失效仍由独立检查立即终止。公开回归通过额外 `Builder::hooks` 启动策略与真实 AdmissionGate 的 allowlist 交集，使已通过实际成员证明的 UDP M 拒绝 TCP 候选，同时资源配置仍允许 TCP；真实受限交换中另一获准 UDP 路径取得新鲜 ACK，原 M/stream 保持。这个条件明确是受支持的较严格启动策略，不把它说成默认端点必然拒绝候选，也不直接修改私有状态或伪造认证结果。

## Noq ACK 返回路径修正

实际 RTC 故障先暴露全局 ACK 聚合：旧 Available RTC 仍活时，新可用 Backup relay 的 probe ACK 被编码到断路的旧载体。允许验证帧和 ACK 后，还需要先尝试已有 CID、已验证、未 abandoned、自己的 PN space 有 pending ACK 的 Backup。普通 STREAM/DATAGRAM 的 `may_send_data` 决策与当前允许检查继续生效。

随后真实旧 TCP 回归发现相反角色：好的新 TCP 被选为 Available，断路的旧 TCP 成为 Backup；不加限制的 Backup ACK-first 会抢走新 TCP 的 ACK。最终修正把 **已验证、拥有 CID、未 abandoned 且 `!may_send_data` 的 Backup ACK-only turn** 限定为自己的 PN-space ACK。相同 predicate 同时用于 readiness、coalescing lookahead 和 ACK 编码；bookkeeping 与 ACK-delay timer 仍只处理 builder 实际记录的 ACK targets。未获准 RTC、Initial/Handshake、普通数据允许的路径保留原 cross-path ACK 行为。没有新 wire、持久 marker、队列或拥塞控制策略。

回归在真实 Noq 多路径模拟中保留旧 Available、切断其路由、保留已验证 Backup，要求两端新鲜 probe 完成，并验证该 Backup 没有普通 STREAM/DATAGRAM。移除 ACK-first 的隔离副本实际失败；最终 vendored 源码及测试副本身份相同。现有两个 public native TCP 修复角色也在原三秒预算内继续原 stream。

本次证明上述实际路径角色与 caller 条件。持续 pacing/congestion 阻塞 Backup 时，现有调度仍可能回到 Available 并在其上聚合 ACK；本次没有单独资格化这个条件或更改其 CC 策略。

## 自有 kernel cut 与清理

```sh
rtk proxy python3 scripts/check_browser_paths.py --case rtc-data-cut --cut-mode relay --output /tmp/mio-rtc-s7-relay
rtk proxy python3 scripts/check_browser_paths.py --case rtc-data-cut --cut-mode none --output /tmp/mio-rtc-s7-none
```

runner 使用固定已缓存镜像，只创建自己的一座 internal bridge 和一只标记 container。repo、native/Node/Chromium/RTK 与 Chrome 所需库只读挂载；设施、证书、私有 NSS 和证据在本次目录。正常 TLS 验证没有放宽。internal bridge 无 default route 导致 native mDNS join 的真实 syscall 返回 ENODEV 时，仅向该命名空间的实际单一 IPv4 interface 添加 `224.0.0.251/32` 路由；不改宿主 route、默认路由或产品 driver/profile。

准备阶段独立完成入网保存并关闭准备端点。测量阶段从相同 native DATA_DIR 和 browser IndexedDB 重开：原 M 首先只有 MQTT/WSS Control 路径，双方 hook 完成 RTC 转换才返回普通句柄。relay 情形只保留显式获准的自有 relay，测量中先保持它 dormant；取消及短 caller 检查之后才让真实正常 TLS relay WebSocket 打开。none 情形不允许数据 relay；ICE/TURN servers 始终为空。

RTC 两端实际 stats 必须给出相反且一致的 IPv4/UDP selected pair，`ss` 必须有相应 sockets。nft 只对该四元组的两个方向 INPUT/OUTPUT 计数 DROP；TCP 信令与自有 relay 不在规则内。pcap 使用 `any`、immediate mode、64-byte snaplen、最多 300 包；实际非空的 SLL2/IPv4/UDP packet header 全部解码匹配该 selected tuple，仅保留截断加密报文。初始 DROP 与结束 DROP 分开记录，规则存在时原 stream 序号 1 必须实际停止。

relay 恢复之后，同一个双向 stream 的序号 1、2 完成；两端各自 stable id、stream id 和 exporter 比较布尔值保持。原 M 上 relay 普通 STREAM 增长，signal 只增加受限维护流量。native RTC PC 关闭、队列和 pump 归零，立即及晚期 association/pump 计数为零，原 selected UDP socket 消失，而原 relay M 仍可用。none 返回活但不可达并保留原待完成 stream。

terminal 故事明确使用 **原 SDK 句柄主动 close**：先关闭旧 M，再撤销 DROP 让 UDP 返回，然后再次维护这条旧句柄，只能得到旧 stable id 的 terminal，旧待完成 stream 失败。这不声称 RTC 自然 idle expiry。browser PC/callback/pump/queue 与 native fixture 退出都保存实际结果；fixture 最后 drop 自己的 endpoint clone，等 transport 的真实关闭通知，不忽略非零退出。

协议/fixture 时钟从证书及容器准备前开始，工作最多 45 秒，预留 15 秒清理；总计不超过 60 秒。native/WASM 编译与 bindgen 分开计时并保存产物 hash。finally 删除精确自有 nft table、进程、container、network 和临时目录；实际标签查询必须证明 container/network 已不存在。任何清理命令、absence 查询或临时目录移除失败都保存 false，并以非零 CLI 退出，仍按原时钟尝试后续精确移除。真实子进程 timeout 与非零查询回归只注入最后 absence 查询故障，不替代 SDK、认证、协议或实际移除。证据省略 SDP、ICE 凭据、DTLS fingerprint、私密 exporter 和材料内容。
