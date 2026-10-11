# 切换 Wi-Fi 后的 MQTT／RTC 复测

用户切换 Pixel Wi-Fi 后，复用刚才相同的 Android／WASM SDK 构件和原测试
控制器，完整公共 MQTT 信令＋RTC 业务通过。OP13 保持移动网络，未配置
数据 relay／TURN。成员建路 SDK 实测 7.858 秒，非空副本同步 8.580 秒，
PING/PONG 44.510 ms；包括审批、重开、负例及清理的整轮耗时 74.104 秒。

新 Wi-Fi 下的独立 UDP 探针已恢复正常，完整故事也通过：四条副本验签、
保存、应用及重开后仍存在；普通成员实际选择 RTC，fresh ACK、原连接与
绑定保持，业务 PING/PONG 成功。未批准及真实撤销成员仍不能释放业务。
这支持此前定位的 Wi-Fi 转发故障解释，原失败保留为失败。

上一轮不是只有 MQTT 使用旧 Wi-Fi：两种 relay 场景的抓包也出现同一个
旧 Pixel Wi-Fi 地址。它们的业务实际走 relay；IRC 后续业务也选 relay，
RTC role 为 None。因此不能把这些成功理解为“其他打洞碰巧成功”。

[证据、时间口径及路由资格界限](../evidence/rtc-switched-wifi-retest-2026-10-11/README.md)
保留原 SDK 构件／源码 hash、实际 DTO 校验、包记录摘要及清理证据。
本次原成员路径获既有采集器的物理数据／WAN 资格，仍不补齐全部方向矩阵
或完整 SDK socket 终止验收。SDK 与网络配置均未由 agent 修改。
