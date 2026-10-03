# MQTT-WSS 完整 datagram：第 1 步实施资格

2026-10-03。范围为 [WebRTC 重新规划](2026-10-03-browser-webrtc-replan.md)的第 1 步；本页记录实际实现和实验结果。执行状态由 [MIOPUNCH-68](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/00215ab6-2318-48ef-8667-10f03c5957e5)维护，不改变产品 ADR、默认公共设施或 UI 框架选择。

新增 browser Rust MQTT-WSS datagram adapter，复用从 native 提取的 MIO1 编解码、地址/service 校验与 ACK 资格状态机。可信配置显式对应 logical facility、native MQTT/TLS URL 和 WSS URL；禁用任一入口或 logical ID 都阻止该 facility，不能通过另一别名恢复。成功 CONNACK 与对应 QoS0 SUBACK 之前无 route；失败、关闭或断连撤回 route。native 使用已有 CustomTransport driver 的独立 datagram 入口，browser 当前交付同等 carrier 入口，尚未接入浏览器 QUIC。

可复现入口：

```sh
rtk proxy python3 scripts/check_browser_paths.py --case mqtt-datagram --output /tmp/mio-rtc-s1-new
```

runner 使用独立自有 Mosquitto 容器。自有 TLS/WSS bridge 正常校验 localhost 证书；CA 只导入临时 NSS DB，通过 bwrap 私有挂载供测试 Chromium 使用，不修改用户/系统信任，不传证书绕过参数。bridge 转送真实 broker MQTT 流，只在自有故障场景修改实际 SUBACK 或断开 socket。此次没有公网流量。需要已有 Docker Mosquitto 镜像、Linux bwrap/NSS、Chromium、Playwright、Rust WASM target 和匹配的 wasm-bindgen；此夹具资格不等于新平台或公共集群资格。

已实测 browser/native 各发 1200、2048 字节，共四条正常消息，逐字节一致；两端合计仅四次 broker PUBLISH。两端 2049 字节和错误 service tag 均在本地拒绝，没有额外 broker PUBLISH。两端成功后的真实断连都撤回 route；native 地址 watcher 从 1 变 0。两端订阅拒绝、错误对应 SUBACK、SUBACK 前断连都没有 ready。入口禁用没有 WSS 尝试，正常关闭发出 UNSUBSCRIBE/DISCONNECT，结束时 native 进程和 proxy socket 为零，全部自有监听及容器已清理。

首次真实断连暴露 native RX sender 先关闭、route 稍后撤回的竞态，已改为 driver 先退休地址及状态再关闭 RX。Standards 评审随后发现旧 driver 可以污染立即重绑的新 generation 诊断；真实 close/rebind 测试先红后绿，现将 protocol 和队列计数归各 binding 所有。双轴 code-review：Standards 的两项启发发现已修复并复核，Spec 无发现。

修复后完整协议运行的资格、URL、ACK、generation、wire/datagram 字节、丢弃、队列峰值、ready/withdraw 时间、预算和清理见 [资格回执](../evidence/browser-mqtt-datagram-2026-10-03/qualification.json)；回归与红绿证据摘要见 [验证回执](../evidence/browser-mqtt-datagram-2026-10-03/validation.json)。原始运行文件路径与 SHA256 保留在回执中。attempt 使用 60 秒外层和 15 秒阶段保护，构建单独记时；实验预算不作为产品 SLA。

此次只资格化了自有 Linux Chromium/native 设施上的完整 MQTT datagram。受限 QUIC/TLS、browser CustomTransport 接线、正式 resource 配置整合、成员会话换路、RTC、公共设施完整报文与其他平台仍由后续对应验证承担；没有把本结果外推为 11 步完成。
