# 当前 MQTT/RTC 无数据路径的网络诊断

已复现一个足以阻断本次 ICE 的网络问题：Pixel 当前 Wi-Fi 经运行 Mihomo
的代理网关，同一 UDP socket 从 STUN A 改发 STUN B，收到的仍是 A 的回复。
反向 B→A 也复现；新 socket 则正常。OP13 移动网络的六项对照全部正常。
这个独立探针不使用 SDK、MQTT、SDP 或 RTC。

此前 10 月 8 日同方向直连、非空副本通过时，Pixel 使用的是另一张 Wi-Fi
网络。手机无 VPN 不代表 Wi-Fi 网关没有代理。不能将本次失败归结为移动网
无法直连，也不能据此宣称 SDK 回归。

两份当前失败抓包均没有 DTLS；浏览器向 OP13 候选地址发送的 ICE 检查，
分别有 240／239 个事务收到配置 STUN 服务的回复，缺少 ICE 校验所需的
MESSAGE-INTEGRITY。这与独立探针的目的地固定现象吻合。

[证据与复现输入](../evidence/rtc-wifi-udp-destination-diagnosis-2026-10-11/README.md)
保留正反顺序测试、旧成功与当前失败抓包的关联结果、原始文件 hash。
手机及网关配置未改，临时探针目录已删除；具体 Mihomo adapter／上游代理
组件尚未隔离，修正网络后的原 SDK 业务复测尚未执行。
