# 已保存成员：MQTT/WSS 起步的原 M 在放行前换到 RTC

本次对应 [WebRTC 分步规划第 6 步](2026-10-03-browser-webrtc-replan.md#6-wss-起步-m-的放行前转换)。双方互相已知、持久化材料已完成、当前秘密和成员资格仍有效；先通过 MQTT/WSS 建立受限的原 QUIC M，再在双方 `before_data_release` 内取得这条 M 的 RTC 数据路径。普通句柄只在路径验证和双方确认完成后返回。[实施回执](../evidence/browser-wss-member-rtc-2026-10-04/README.md)分别保留正常、失败、兼容和回归结果。

这是显式安装 adapter 的实验入口。正式 RTC 后端、默认设施、TURN、UI 和 IWA 没有在本次定案。IRC 慢速 profile 的受限握手仍是[未资格化停止分支](2026-10-04-browser-irc-restricted-handshake.md)，不能把 MQTT 结果外推成 IRC 第 6 步成功。

## 明确记录的 wire 与角色决定

有限映射只涵盖既有业务和 replica 成员协议；用途、成员校验、应用 handler 和 LogSync 内容不变。

| 用途 | 旧 ALPN：严格 v2 | 新 ALPN：严格 v3 |
| --- | --- | --- |
| 业务 M | `miopunch/browser-member/1` | `miopunch/browser-member/rtc-path/3` |
| Replica M | `miopunch/control-replica/1` | `miopunch/control-replica/rtc-path/3` |

TLS 在同一连接选择 ALPN。客户端以既有 ALPN 通过 before-connect gate，同时提供新 ALPN；服务端显式偏好新再旧，不依赖 Router 排序，不因降级另拨 M。旧 v2 保持四字节 `[2,tcp,udp,relay]`；v3 是五字节 `[3,tcp,udp,relay,role]`，role 为 None=0、Offerer=1、Answerer=2，长度、版本、布尔位和角色值都严格检查。其它既有 native 应用 ALPN 保持原 v2，不扩大成任意协议升级。

只有已安装、当前允许、当前 M 角色和载体符合条件的 adapter 才声明 RTC。当前资格为 **browser 原 QUIC client / RTC offerer → native 原 QUIC server / RTC answerer**，双方角色互补才形成共同 RTC 能力。native dialer、browser server 或缺 adapter 时声明 None；不存在全节点 `rtc=true`。ICE/DTLS 角色仍读取实际 stats，不由 offerer/answerer 推断。

## 原连接的 Control 路径与最终放行

当前秘密证明、成员/持久化材料检查和 stage agreement 先完成。双方 hook 经已认证原 M 的私密控制 stream 交换能力、RTC context、SDP 与参数；复用第 4/5 步 Association、datagram codec、lease 和 pump。原 QUIC dialing side 使用现有 Control path 入口。DataChannel open 只是载体就绪：同 M、同 lease、同 RTC path 的新鲜 opaque probe 得到认证 ACK 后，还要复查当前资格并完成双方确认。

新增 `retain_control_data_transport_path` 与 Ordinary retain 分开。它只接受这条 Control M 的精确已拥有、当前仍合格的 custom 地址，受限阶段把原 M 选到该 RTC path。原私密 stream 的最终确认随后实际穿过 RTC；最终 queued authorization 再检查当前 lease，组合既有允许策略并保留这个精确地址。没有整体允许 RTC 类型，也没有使用 IP-only `commit_data_path`。最终普通原 M 不能把 MQTT/IRC 当数据回退；信令设施仍可保持 ready，供后续独立受限联系使用。

lease/pump/PC 在成功放行后由弱原 M 和当前资格持有，独立于业务句柄，因而 replica 自己的 M 也可以使用同一 hook。成功后的存续不绑定已结束的 setup deadline。原 M 关闭、资格/资源失效或泵退出时退休地址并清理 PC、队列和任务；无 Endpoint/强 Connection/私密 stream 的持有环。

## 一个 caller 时钟、取消与旧端兼容

既有 `ConnectOptions` 可选携带原 absolute deadline 和 per-attempt cancellation；`Incoming` 可选携带本地 caller deadline，均传到握手 hook。没有给所有 native incoming 增加全局默认超时。browser 默认 12 秒、native contact 10 秒、maintenance 2 秒仍由各自 caller 创建一次，阶段只读剩余时间。45 秒是诊断上限，不能延长 caller。browser 现在从创建 `BrowserMemberAttempt` 起计时，等待 storage/TLS/hook 都消耗同一时钟；`cancel()` 只取消本次连接操作。未完成 admission future 被丢弃时弱 RAII guard 关闭这一条原 M，即使别处仍持有私密 stream，也不能留下半放行连接。

没有共同允许的数据路径时，public connect 返回明确 `DataPathUnavailable`/temporarily unreachable，身份或证明拒绝仍保持原 rejected 分类。失败不删除成员身份、Complete stage 或秘密材料；同 Endpoint 可再尝试，页面重载可读取同一份已保存材料。

共同获准的旧 relay/IP 能力继续保留。MQTT 起步、v2 且共同 relay=true 的旧端，经既有严格候选/单字节双边确认协议，在原 M 的获准 relay path 上启动真实新鲜 probe、复查当前许可并继续原私密 stream；然后才普通放行。disabled relay origin 不进入能力或候选。独立旧端/no-joint 则明确失败。兼容实验没有用 RTC、第二条 M 或新密码握手。

## 复现与边界

```sh
rtk proxy python3 scripts/check_browser_paths.py --case wss-member-rtc --output /tmp/mio-rtc-s6
rtk proxy python3 scripts/check_browser_rtc.py --case wss-member-rtc --legacy-no-rtc --output /tmp/mio-rtc-s6-no-joint
rtk proxy python3 scripts/check_browser_rtc.py --case wss-member-rtc --legacy-relay --output /tmp/mio-rtc-s6-legacy-relay
```

fixture 先独立完成既有 relay 入网/保存材料，再关闭准备端点，从同 native DATA_DIR 和 browser IndexedDB 重开。测量阶段普通情形只有自有正常 TLS MQTT/WSS：清除 IP transport、禁用 LAN/port mapping/TCP/UDP/成员 data relay/观测资源，native relay 地址为零，browser 禁用 relay；RTC 自己的真实 UDP sockets 另计。旧端获准 relay 的专门兼容例显式保留该资源，实际 WebSocket 在真实 MQTT 能力交换前保持 CONNECTING 且没有 socket/open/data 事件。

正常回执分别证明业务和 replica 原 M 的严格 ALPN、角色、当前 revision、双方 fresh ACK、retain 前 STREAM/DATAGRAM 为零、stable/exporter 比较布尔值、原私密 stream 连续和正常应用 STREAM 增长。失败包含真正 open DataChannel 上丢 QUIC datagram、实际 12 秒 caller 无句柄、预创建 2 秒时钟已消耗、显式 cancel 后同 Endpoint 重试、迟到/close 不复活、同源页面重载材料保留。清理记录实际 PC/callback/pump/queue 和仍存活 native 进程的晚期 socket 基线；不把即时异步关闭快照说成同步回收。

这仍是同机 Chromium/Linux native、已知当前成员的资格。窄 replica 故事不是第 9 步完整业务/非空历史矩阵。第 7 步数据断路维护、完整第 8 步资格变化矩阵、第 10 步 WSS 首次入网和第 11 步异网/成本都保留后续边界；不由本次正常结果推定完成。
