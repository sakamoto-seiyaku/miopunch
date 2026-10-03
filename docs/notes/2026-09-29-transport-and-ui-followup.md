# 拆票后的连接能力与 UI 范围补充

日期：2026-09-29。状态：用户批准后，已将修订稿的 40 张票发布为 [MIOPUNCH-14 规格](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/4700bf29-2a4e-40bc-93fe-7f5148207189)的原生子票（MIOPUNCH-15 至 MIOPUNCH-54），并建立 60 条原生阻塞关系。正文、状态、标签及关系已逐张回读核对；后续执行状态以 Plane 为准。

## 已明确的范围

- 用户确认 UI 可以先不管。本轮聚焦节点核心与平台运行验证；简单测试入口不代表正式产品 UI，原稿中的客户端管理入口应据此收窄。
- 用户接受 TCP 承载 QUIC 的方向。具体 TCP 直连、打洞、报文封装和路径生命周期仍需实现验收；不能把接受方向解释为已经验证性能。
- 用户随后明确同意去掉 KCP；新基线不再把 KCP 作为传输依赖。用户进一步询问 iroh 的 QUIC 调度能否修改，并提到 Hysteria 2 的定制核心；具体拥塞算法尚未选定。
- 用户随后收窄范围：现成 BBR/BBRv3 足够，本轮不做拥塞算法扩展。按当前底座已有 BBRv3 接入，不额外移植 Brutal、自定义控制器或另一版 BBR；下文 Hy2 分析只保留为依据，不构成本轮任务。
- 旧 miopunch 的 TCP/UDP 直连、端口映射及打洞经验需要明确映射到新底座。修订稿已补入 TCP 承载、直连、映射、打洞以及 UDP 观察/映射/打洞的独立交付边界。
- 用户要求实际测试小步推进、快速取得反馈并尽早暴露问题。每个窄场景先跑真实正常路径，再立即测试最关键失败条件；结果合格才扩展下一项行为。
- 2026-10-03，用户明确排除 IWA，认为其正式平台覆盖不足；不安排 IWA 实现。浏览器经 WebRTC 与原生节点直接通信的可行性、适配成本仍在讨论，尚未决定采用或安排实施。
- 同日，用户要求沿现有流程评估 MQTT-WSS / IRC-WSS 引导 WebRTC，允许子代理研究；候选接入、库复用与收益成本已汇总为[综合评估](2026-10-03-browser-webrtc-assessment.md)。本次仅完成评估，原型、正式采用、默认服务和 relay 回退调整仍未决定。
- 用户随后要求保存 Markdown 并标记低优先级；后续候选验证登记为 [MIOPUNCH-68](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/00215ab6-2318-48ef-8667-10f03c5957e5)，当前未启动实施，执行状态以 Plane 为准。
- 2026-10-03 后续 UI 讨论确认：代码复用与长期维护优先，不追求全部 Rust；接受 WebView，同时要求控制内存和响应开销，最终使用一个框架并仅在严重实际问题下考虑更换。用户明确 UI 可以延后，当前只[保存讨论结果](2026-10-03-shared-web-client-ui-direction.md)。Vue 3、TypeScript、Vite 与安装版 Tauri 2 是当前推荐，共享 UI 和两种节点 Adapter 是接入建议，均不据此启动实现或登记为已接受的框架 ADR。
- 同日用户要求启动 WebRTC 重新研究规划，以 gpt-6-astra、low、非 Fast 完成，持续小步实际验证。结果见[重新规划](2026-10-03-browser-webrtc-replan.md)：建议先验证 MQTT 完整报文与受限 QUIC/TLS，RTC 先在已有获准 relay 成员会话上证明原会话换路，再合并 WSS 放行前转换；IRC 慢速载体独立验证，不阻塞 MQTT。已形成 11 个验证步骤并补充一次有界公网 IRC 观察，本次未实施 RTC；UI 延后，正式采用、默认设施及 TURN 决策仍未冻结。

## 本轮核查事实

QUIC 已有丢包恢复与拥塞控制，允许使用不同的发送端拥塞控制算法；iroh 的 `QuicTransportConfig` 暴露控制器工厂接口。[RFC 9002 §7](https://www.rfc-editor.org/rfc/rfc9002.html#section-7)、[iroh 1.2.0 接口源码](https://github.com/n0-computer/iroh/blob/17c0612f80f78f5288e97b818b1360ae6ea0a51a/iroh/src/endpoint/quic.rs#L407)

旧 Go 会话使用 KCP 提供 UDP 上的可靠传输，再组合 TLS 与 yamux。其 `SetNoDelay(1, 20, 2, 1)` 关闭了 KCP 拥塞窗口约束，但不等于关闭接收窗口等全部限制；创建连接使用 10 个数据分片、3 个校验分片的 FEC。转为 QUIC 不自动保留这种激进发送行为和 FEC。[旧会话配置](../../archive/legacy-go-openspec-2026-09-28/internal/pocv1/session/session.go)、[旧 KCP 构造](../../archive/legacy-go-openspec-2026-09-28/internal/netutil/kcp.go)、[KCP 配置说明](https://github.com/skywind3000/kcp#协议配置)、[kcp-go FEC](https://github.com/xtaci/kcp-go)

## 待讨论的建议

由 iroh/QUIC 承担可靠流、端到端认证加密、多路复用与拥塞控制；UDP 路径直接承载 QUIC，TCP 路径通过报文封装承载 QUIC。去掉 KCP 已获用户确认；旧 KCP 可以保留为性能对照材料。

旧 KCP 弱网行为保留为历史参考。本轮不为它新开算法或纠错扩展；现成拥塞配置的基本运行与必要观测随实际连接验证完成，不把完整性能基准作为首个节点行为的阻塞项。

本轮仅核查源码与标准，没有运行新传输组合或性能实验。

## Hysteria 2 与拥塞控制接入核查

再次下载官方 `noq-proto-1.3.0.crate`，SHA256 与 iroh 锁文件一致：`7c1e5b6fe668491eca022f745a0a9402585626c73a7b839b3424ace15d6a9c8f`；包内版本提交为 `c1f411562e6078852749b8bcf1190523096a107f`。

- **源码事实：**该版本已有 NewReno、CUBIC、BBRv3，默认 transport config 选择 CUBIC。iroh 可通过 `congestion_controller_factory` 注入控制器。[内置控制器与 trait](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/congestion.rs)、[默认配置](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/config/transport.rs#L586)
- **源码事实：**`Controller` 收到发送、ACK、逐包丢失、拥塞、RTT、MTU 等信息，提供 `window()`；`ControllerMetrics` 还提供 `pacing_rate` 和 `send_quantum`。实际 pacer 使用显式 pacing rate；发送循环用 send quantum 限制批量，不是只有日志字段。[pacer](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/pacing.rs#L95)、[发送批量](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/mod.rs#L1337)
- **源码事实：**每条路径创建控制器，但 factory 的 `build` 只接收时间和 MTU，不接收路径身份或载体类型。为 UDP/TCP/relay 分别选择策略、切换时调整以及节点总带宽分配，仍需明确策略接线；不能仅看到 per-path controller 就宣称现成支持任意按载体配置。[创建控制器](https://github.com/n0-computer/noq/blob/c1f411562e6078852749b8bcf1190523096a107f/noq-proto/src/connection/paths.rs#L290)
- **上游事实：**Hysteria 2 的 Brutal 是带目标带宽的自定义拥塞控制，源码以目标 bytes/s 除以 ACK 比例得到 pacing rate，窗口结合带宽与 RTT 计算；启用丢包补偿时提高发送速率，另有开关关闭补偿。它使用 Go 的 quic-go 扩展接口，不能把其 Go 实现直接装进 Rust noq。[Brutal 源码，读取于 2026-09-29](https://github.com/apernet/hysteria/blob/master/core/internal/congestion/brutal/brutal.go)、[上游配置与机制说明](https://v2.hysteria.network/docs/advanced/Full-Server-Config/#congestion-control-details)
- **推论与建议：**现成接口足以作为 Brutal 核心算法的首个移植接缝；精确发包节奏、事件统计和生命周期仍需对照验证。先保留 CUBIC 基线、比较现成 BBRv3，若需要再增加明确目标带宽的 Brutal 可选模式，不先定为全部路径默认。TCP 承载下的外层 TCP 拥塞与排队仍然存在。流优先级、路径选择、载体用途资格与拥塞控制分别负责不同问题，不能通过换控制器解决信令载体业务隔离。

上段记录的是范围收窄前的技术建议。用户已明确本轮不做拥塞算法扩展，后续拆票以本页「已明确的范围」为准。

## 已有测试环境与推进方式

用户提供：ADB 可连接 Pixel6a；旧 miopunch 有 QEMU、Docker 下的 NATX 测试环境；本地可以启动 Docker 测试。只读检查已确认 Docker daemon 可访问、Pixel6a 在线。本轮没有启动测试容器、VM，也没有安装或运行手机测试程序。QEMU/NATX 当前实例、镜像位置和旧工具适配仍需在首个使用它的验证票中核对。

- **快速内循环：**本机双进程或隔离 Docker 项目，真实 iroh/noq、密钥、消息服务和存储；先一个正常场景，加一个关键错误场景。两个容器互通不自动代表两端 NAT 打洞成立。
- **网络机制：**使用已有 QEMU/NATX/netns 材料，先确认场景实际 NAT/转发规则，再以真实 UDP/TCP 数据面验证一个可打通和一个不可打通场景；遇到新问题才扩大拓扑矩阵。不继承旧 Go/OpenSpec 的全量测试门槛。
- **平台问题尽早暴露：**首个最小原生节点通过后，就在 Pixel6a 上跑同一握手、往返和保存身份后的重启；浏览器也先验证真实运行与存储。手机在同一 LAN 的结果只作为平台证据，跨网/蜂窝穿透单独标注。
- **故障回归：**每张实施票继承短小的已通过场景；新问题保存最小复现。持久化功能尽早在关键提交点重启，载体功能尽早断路和取消。当前场景失败时先收窄诊断，不继续把未验证假设传给依赖票。
- **环境独立：**新 Docker 实验使用独立项目/网络/卷，避免改动已有其他项目容器。Windows 与真实跨运营商执行环境尚未在本轮确认，相关验收如实保留待运行状态。

修订稿共 40 张：13 张前置验证与 27 张实施。消息握手与载体隔离分开；权限模型、持久恢复和根权恢复证明分开；补入 TCP 承载、UDP 观察、NAT 打洞对照、Pixel6a 早期运行，以及 UDP 映射、TCP 直连、TCP 映射和 TCP 打洞。验证票不组成统一瀑布门槛：按各自依赖通过情况立即推进对应实施。
