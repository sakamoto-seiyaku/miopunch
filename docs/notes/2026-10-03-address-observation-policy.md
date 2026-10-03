# 地址观察来源与优先级

日期：2026-10-03。状态：用户已确认的配置与观察源策略。

正式接线由用户随后批准的
[MIOPUNCH-67](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/058e5f38-8632-4dba-92af-f1f28bbae3d5)
交付，工作状态以 Plane 为准。

## 已确认的策略

- 保持现有 number0 官方默认 QAD 服务器列表，不增加或替换其中的节点。
- UDP 地址观察优先使用 QAD，STUN over UDP 作为备用观察来源。
- TCP 地址观察使用 STUN over TCP；UDP 的观察结果不能作为 TCP 映射。
- 既有资源禁用、替换及用途权限规则继续适用。

用户随后确认：TCP 数据路径作为可选方案，在需要时显式开启；当前流程
保持原状。本次不因「备选」的称呼调整已有 TCP、relay、UDP 的尝试顺序。

本次确认针对地址观察来源及其优先级。TCP、UDP、relay 数据路径的尝试
顺序没有在本次讨论中作出新决定；观察来源的优先级也不决定业务路径的
优先级。本次没有指定新的超时、并行度或备用 STUN 服务实例。

## 确认依据

用户在讨论 QAD 与 STUN 的能力和公共服务来源后确认：

> 官方的不动了，qad 优先于 stun 可以，stun udp 作为备用。
> tcp 还是只能 tcp stun。

现有默认 QAD 列表见 [iroh 配置](../../vendor/iroh/src/defaults.rs)，
地址观察与数据中继的用途规则见
[产品主线的资源描述](2026-09-27-product-mainline-draft.md#4-资源来源与统一描述)。

## 正式接线与验证

MIOPUNCH-67 已把两个观察入口接入正式配置节点。UDP 直连验证仍先于
观察和选模；需要映射序列时，QAD 提供有效序列就直接使用，失败或序列
不足时才尝试获准的 STUN/UDP。全部来源不可用时保留既有获准的基本
尝试或有界失败，不使用旧 STUN 候选，也不放行信令业务。

一轮 STUN/UDP 序列共用一个查询代际，最多选择四个获准来源，按实际
解析后的 IP:port 去重。结果保留原始来源、目的地址和采集顺序，来自
原 Endpoint 的 primary IPv4 数据 socket。DNS 与每个 Binding 事务分别
有两秒和三秒上限；网络变化、关闭或更新查询使旧结果失效。序列保留
既有 STUN 的四十秒期限，QAD 的旧长计划和节点共享选模/预算规则保持。
不足两个不同实际目的地址的结果不能形成分类证据。

TCP 正式打洞现在接受域名与字面 IPv4 STUN 服务，并与诊断入口复用
同端口观察、协议校验和查询 supersession。观察 socket 保留到路径验证
和双方提交结束；独立诊断保留到替换或 Endpoint 关闭。观察仍不是可达性、
身份或成员授权。TCP 默认关闭，打洞范围仍为 Linux IPv4；Android 仅验证
地址观察。既有 TCP → 获准 relay → UDP 的消息升级顺序保持。

默认资源表 revision 2 在四个官方 QAD/relay 项之外补充下面的
Observation-only 项，不授予 DataRelay 或 Signaling 用途，不做 TURN
allocation。已有保存的表、替换和禁用不会被自动覆盖。

| 来源 | 声明的用途 | 核实依据 |
| --- | --- | --- |
| `stun://stun.cloudflare.com:3478/` | UDP Binding 备用观察 | [Cloudflare 服务表](https://developers.cloudflare.com/realtime/turn/#service-address-and-ports) |
| `stun://global.stun.twilio.com:3478/` | UDP Binding 备用观察 | [Twilio 官方网络配置](https://www.twilio.com/docs/video/ip-addresses#quick-start-minimum-firewall-configuration) |
| `stun+tcp://turn.cloudflare.com:3478/` | 显式启用 TCP 时的 Binding 观察 | MIOPUNCH-64 实际公网响应，以及本票 Android 回归 |

前两项依据服务方文档声明协议能力，不能据此声称本地公网 UDP 已通。
此前 MIOPUNCH-64 的公网 UDP 超时结果保留。本票完整 UDP 接线通过的是
受控真实协议及 Linux 双内核 NAT；新增公网 Android 结果只证明 TCP
Binding，不是 Android TCP 打洞或公网 UDP 穿透。

可重跑入口、QAD 零备用请求、STUN 多目的序列、原 Connection、真实新
记录同步、优先禁用失败和恢复的证据见
[MIOPUNCH-67 验证记录](../evidence/mio67-stun-flow/README.md)。

TCP STUN 公网观察的证据见
[公网资源集成记录](2026-10-03-public-resource-integration.md)。
TCP 打洞的接入与平台范围见
[TCP 打洞验证记录](2026-10-02-tcp-punch-validation.md)。
