# iroh 融合完成后复查 WebRTC 接入

2026-11-19，用户决定先完成原生 iroh 穿透融合，再讨论 Web 节点接入。下面是融合前工作树的源码观察与历史实测。本轮未构建或实跑 WASM。恢复讨论时，先用新版的正式节点入口、共享连接和流分发方式复核这些问题，再确定实际需要的改动。

现有目标是浏览器↔原生、浏览器↔浏览器沿用各自的 WebRTC 适配，由 ICE 建路，DataChannel 承载原 QUIC 连接的报文。已验证的直接 RTC 路径进主档，原生 relay 和获准的成员数据中继同属备用档。见[穿透与选路决议](../adr/0019-miopunch-application-on-p2panda-and-iroh.md)和[共享成员业务连接决议](../adr/0020-shared-member-business-connection.md)。

## 融合前观察与复查入口

| 环节 | 当时的事实 | 融合完成后核对什么 |
| --- | --- | --- |
| 正式节点启动 | 浏览器创建实际 PeerConnection 和 DataChannel 检查运行能力。原生 demo 显式装入 RTC，普通 `NativeClient::start` 的绑定入口传入 `None`。[浏览器能力检查](../../browser-probe/src/rtc_capability.rs#L55) · [demo](../../src/bin/miopunch_demo.rs#L178) · [普通客户端](../../src/native_client.rs#L180) · [绑定参数](../../src/native_network.rs#L946) | 从正式客户端双向发起连接，核对实际安装、用户启用和双方公布的能力。 |
| 首次只有信令 | RTC 地址属于一条原连接，不能直接用于首拨。已跑通的流程先经受限联系完成握手，在获准范围内协商 RTC。浏览器的成员数据中继还要挂在已经准入的外层成员连接上。[RTC 地址用途](../../src/rtc_transport.rs#L1254) · [浏览器首次协商](../../browser-probe/src/rtc_member.rs#L700) · [成员中继](../../browser-probe/src/member_data_sdk.rs#L1) | 沿新版 MQTT／IRC 或已可用的成员联系入口，确认何时能取得第一条获准数据路径；只剩信令时应能报告“联系得到，业务路径不可用”。 |
| 从健康 relay 改走 RTC | 两端的改善入口都会另拨受限控制连接，并用这条连接的发起方向确定 RTC offer/answer。原 QUIC 拨号方可能是另一端。[浏览器拨号](../../browser-probe/src/member_connection.rs#L139) · [原生拨号](../../src/connection_coordinator_native.rs#L180) · [角色与绑定](../../src/connection_coordinator.rs#L2478) | 在新版共享连接里分发换路控制消息和业务流。健康 relay 上额外控制拨号应为零，同时保留 RTC 尝试与原连接的身份绑定。 |
| 何时改善与怎样选路 | Web demo 自行调用 `improve_path`，并显式选用 `PreferDirect`。iroh 周期检查只识别原生 IP 路径。[demo 触发](../../demo/client/src/window-node.ts#L36) · [平台能力分流](../../src/connection_coordinator.rs#L2414) · [iroh 周期检查](../../vendor/iroh/src/socket/remote_map/remote_state.rs#L1587) | 从普通业务触发改善，按两端实际能力选择 RTC。健康 RTC 应被识别为合格直连；浏览器连接上不应因缺少原生 IP 而反复触发原生打洞。按融合后的默认选路验证。 |
| 同时改善与路径验证 | RTC 适配器按原连接保留一个 association，校验本轮协商、具体 DataChannel 路径和新鲜 QUIC ACK。[实例归属](../../src/rtc_transport.rs#L373) · [路径验证](../../src/rtc_transport.rs#L885) · [浏览器绑定](../../browser-probe/src/rtc_member.rs#L316) | 两端同时有业务、同时寻求直连时，确认双方推进同一轮 offer/answer，取消或迟到的回执只影响对应尝试。沿用新版原连接上仍有效的验证结果。 |
| 流、路径和连接各自的关闭范围 | 旧浏览器文件和终端代码有在服务撤权或操作结束时关闭整条连接的调用。RTC 已有撤下确切路径和队列的机制。[文件撤权](../../browser-probe/src/files.rs#L550) · [终端结束](../../browser-probe/src/terminal.rs#L41) · [RTC 路径撤销](../../src/rtc_transport.rs#L456) | 分别观察关闭终端后的文件流、RTC 中断后的获准备用路径、成员撤销后的整条连接，以及真实切网时原流是继续还是明确终止。浏览器宿主窗口退出另按其生命周期验证。 |
| 浏览器与共享源码接口 | 当时浏览器调用 `improve_existing` 传六个参数，共享函数签名有七个，第七个是 `seal_hello`。这是源码对照结果。[浏览器调用](../../browser-probe/src/member_connection.rs#L145) · [共享定义](../../src/connection_coordinator.rs#L2747) | 先以融合版本实际保留的接口构建 WASM，再通过正式入口运行。旧参数和旧的独立控制连接都可能已随融合改动。 |

## 恢复讨论时可复用的实测

- 浏览器与原生节点曾在不同公网地址间选中 RTC。关闭 relay WebSocket 并禁止重连后，原成员连接仍返回了正确文件字节。该 demo 的 RTC 宿主是持有节点的 Window。[实机与窗口生命周期](../evidence/client-demo-2026-10-21/rtc/README.md)
- 两个浏览器曾经 MQTT 双向发起并收发业务；另有同机 relay 切 RTC 保留原流，以及关闭 RTC 载体后修复的记录。各用例的设施与覆盖范围见[主线验收](../evidence/current-connection-mainline-2026-10-11/README.md)。
- Android 原生端和 Pixel 浏览器在蜂窝／Wi-Fi 环境下，曾用公共 MQTT、无数据 relay 和 TURN 完成成员业务与同步。这是换网后新建连接的成功样本；持续原流跨网迁移要单独验证。[移动网络复测](../evidence/rtc-switched-wifi-retest-2026-10-11/README.md)
