# MQTT-WSS 真实受限 QUIC/TLS：第 2 步实施资格

2026-10-03。范围为 [WebRTC 重规划](2026-10-03-browser-webrtc-replan.md)第 2 步的 MQTT 版本；执行状态仍由 [MIOPUNCH-68](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/00215ab6-2318-48ef-8667-10f03c5957e5)维护。第 1 步的完整载体资格见 [MQTT datagram 回执](2026-10-03-browser-mqtt-datagram.md)。本页是实现证据，没有新增产品默认设施、RTC 库或 UI 决定。

BrowserNetwork 现在可通过显式受信任的 `mqtt` facility 与 `contact_routes` 配置启动、联系邀请指定身份；序列化 route 在启动时校验并编译为 EndpointId/CustomAddr。native MQTT/TLS 与 WSS URL 通过 logical facility 关联，禁用任一别名或 logical ID 都禁止该设施；没有猜测 hostname 互通。启动等待第一个实际就绪的允许 carrier/relay，ready watcher 只在成功 CONNACK/对应 SUBACK 后发布地址，关闭后撤回。浏览器 CustomTransport 复用第 1 步的真实 Rust/WebSocket carrier；SendWrapper 保证 iroh 所需 Send/Sync 对象只在 JS 线程访问和销毁。

native 夹具从显式 facility 选择已有 MqttTransport，再通过 NativeNetwork 原入口使用持久资源表的 Signaling 许可、持久身份与共享 AdmissionGate。节点入口会覆盖 builder 的资源策略，所以入口许可来自保存的资源配置，没有通过放宽 builder 绕过门禁。

可复现入口：

```sh
rtk proxy python3 scripts/check_browser_paths.py --case wss-restricted-handshake --output /tmp/mio-rtc-s2-new
```

runner 复用第 1 步自有 Mosquitto、真实 MQTT/TLS 与 WSS bridge、临时 CA/私有 NSS 浏览器信任；正常证书及 hostname 校验始终开启，没有公网流量。依赖与夹具限制见第 1 步回执。native 夹具没有批准命令。

实际无网络秘密的 browser 申请者，经原 `miopunch/admission-request/1`、真实 iroh QUIC/TLS 和共享阶段交换建立受限 A，发出原申请控制消息并收到 `AwaitingApproval`。browser 握手约 176ms，整个申请约 212ms，实际 caller 剩余约 11789ms；两端记录身份、ALPN、各自 A stable_id、QUIC frame/packet/loss 计数与 carrier wire/datagram 字节。没有 TLS key、网络秘密、批准或材料交付。native Member gate 为 false；browser 普通业务调用被拒绝。A 没有成为普通 M。

真实失败故事全部通过：

- 同一实际 route 改为错误预期 contact，真实 TLS 证书校验拒绝，尚未取得 A 或申请结果。
- 第二个独立 browser 身份持同一邀请，故意复用原 request ID，仍成为独立的未批准申请；成员数保持只有原管理员，两个申请均无批准结果，第二身份没有材料交付句柄。已有共享 manual_admission 的真实 MQTT 回归另外验证了另一持邀请身份不能冒充申请者或取得已批准结果；本次 WSS 实验自身不创建批准结果。
- 三种 facility 禁用方式均没有新 WSS 连接。
- 自有 bridge 实际丢弃 QUIC PUBLISH，申请在 12008ms 截止；另次相同故障经独立取消句柄在 103ms 拒绝。

每次申请使用单一 monotonic 12 秒 caller 预算，持久提交、握手、阶段交换、控制消息共用余量。45 秒是整个 runner 的诊断保护上限，没有覆盖产品的 12/10/2 秒语义。申请连接有 RAII 关闭边界，取消断开当前 endpoint，重新打开保留持久身份/申请；取消不会作者化离网。

最终协议尝试（含夹具和清理）20.276 秒；build 单独计时。native 回执等实际 driver DISCONNECT 后记录，关闭的两端 route/RX/重组队列均退休。同步 browser close 快照可能仍见待发的 61 字节退订/断开消息，真实 bridge 随后对每条连接均收到 UNSUBSCRIBE/DISCONNECT，最终自有 native 进程/代理 socket 为零、监听关闭、容器移除。清理等待有独立上限，没有增加 contact 预算。

完整身份/时序/载体计数/失败/清理及源码、原回执 SHA256 见 [资格回执](../evidence/browser-wss-restricted-handshake-2026-10-03/qualification.json)。[验证回执](../evidence/browser-wss-restricted-handshake-2026-10-03/validation.json)记录 85 项受影响 native 回归、原 browser relay 正常申请/业务故事、第 1 步真实 datagram 回归、workspace/all-targets 与 WASM typecheck、格式和语法检查。双轴评审：Standards 的 typed-route 建议已修复并复核，最终 Standards/Spec 均无发现。

本次只资格化自有 Linux Chromium/native MQTT 设施上的受限申请，不证明 IRC、公共设施或其他平台；没有材料持久化完成、RTC 路径、成员换路或 UI 能力。后续 MQTT 第 6 步还需与第 5 步结合，显式升级能力协议，在普通句柄放行前完成原 M 的 RTC 新鲜路径验证并封禁 MQTT 业务承载；首次批准/材料完成由第 10 步验证。
