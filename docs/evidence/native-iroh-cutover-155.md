# MIOPUNCH-155 · 原生应用融合切换

`NativeClient`、`network run` 和运维 demo 的普通原生业务从同一 iroh 成员 owner 建连，准入后按流分派给应用。生命周期与 demo 继续使用 p2panda 的记录、授权和独立 Replica 同步。融合装配退出 `NativePathAdapter` 的调度，维护宿主也不注册旧 active-improvement responder 或启动其后台定时器。已有专用协议上的 TCP 承载保留逻辑留在准入处，复用实际已连接的 socket。 [应用入口](../../src/native_client.rs) · [装配](../../src/native_network.rs) · [维护宿主](../../src/network_maintenance.rs)

| 调用方 | 运行入口与现有契约 |
| --- | --- |
| 原生 ping、文件、剪贴板、转发、终端、Web | 同网络同成员对共用 iroh 成员连接。服务流保留自己的授权和取消。原路径失联时，融合 owner 调用相同的受限原路恢复入口。 |
| 原生运维 demo | 双端从融合 owner 接收，诊断请求带自身服务编号。周期检查使用维护流，前台服务持有业务流。单条超时或 demo 停止只结束诊断流，原连接的空闲收尾由 owner 决定。已终止的连接随后拨新连接。 |
| 访客、入网、Replica、恢复专用协议 | 保留独立 ALPN，以及既有 SCRAM、资源许可、p2panda 记录和同步边界。 |
| Chromium/WASM、显式低层库和协议工具 | 浏览器文件与剪贴板继续使用专用协议。浏览器的普通成员会话和受限恢复沿用共享 `connection_coordinator`、`admission_gate`、`session_maintenance` 与恢复协议。显式低层 RTC/contact 和普通 iroh 调用仍有真实消费者，保留相应适配器。 |

受限联系先验证 TLS 与成员身份。普通 QNT 在原连接上先取得路径机会，既有路径 actor 需要增强时才进入同一个 iroh `Planning` 和该节点的观察、发包、冷却预算。双方在原保护连接上交换是否需要该轮，复用现有协商、计划、探测、实际 socket 交接和新鲜 ACK。只有双方选择该 UDP 路径并取得准入许可后才放行第一条业务流，随后退役信令载体。融合原生 ALPN 升为 `/3`，以识别受限阶段新增的保护消息。浏览器和专用协议的 ALPN 保持各自原有版本。 [准入及首拨](../../vendor/iroh/src/endpoint/member_fusion.rs) · [单轮协调](../../vendor/iroh/src/endpoint/member_fusion/udp.rs)

配置了获准 TCP 观察点时，iroh 的直接候选流程先处理已有地址。没有合格直连时，两端能在健康 relay 上的原连接协调增强 TCP。应用适配器沿用 `MIOTCP01`、实际监听端口的 STUN 观察和双端 TCP prime。iroh 持有保护流与原期限，并在双方都取得新鲜 ACK 后才提交该物理承载。前台释放会取消未提交的候选，原 relay 继续承载其余流。持续有业务且仍无合格直连时，同一个流程会在退避到期后重试。已验证的最后一条直连退役后，原连接的路径通知也会唤醒该流程。第 n 次失败延后 F×2^min(n,5)，争用延后一个探索间隔 F。真实网络代际变化重置退避。 [原连接的直接候选](../../vendor/iroh/src/endpoint/member_fusion/direct.rs) · [TCP 执行适配](../../src/tcp_punch.rs)

## MIOPUNCH-142 与 125–132 对照

- 125 和 127 保留跨端 attempt／round 关联和单侧 stable ID。诊断可区分业务结束、对端关闭、传输故障和无法从传输判断的原因。单流超时后其它请求仍从同一原连接返回，原连接终止后才重新拨号。 [旧行为](../notes/2026-11-19-native-iroh-fusion-baseline.md)
- 126 和 128 保留前后台联系隔离、原调用的绝对期限与取消、原连接单修复者和候选失败分类。信令通路成功只进入受保护的建路，健康备用和目标明确拒绝仍按各自结果结算。 [受限首拨](native-iroh-member-contact-148.md) · [慢 relay](native-iroh-slow-relay-149.md) · [原流恢复](native-iroh-network-switch-154.md)
- 129 和 130 延用融合后的单一观察与探测预算。两端在实质工作前同时争用才按身份让行，让行方仍可随后尝试。自动改善和首拨选路无需学习。旧打法学习曾由显式升级 API 和技术探针消费，正式业务没有双方反馈入口。仍可用 150 的显式 API，在同一获准原流双端读取胜路的新业务字节后只记一次成功。 [增强原流与学习](native-iroh-member-udp-150.md) · [双向争用基线](native-iroh-path-contention-156.md)
- 131 和 132 依据原连接、资源许可和新鲜 ACK 判断存活，保留黑洞时原流回落。健康路径按 iroh 的主备档和偏置 RTT 比较，IP 为主档，原生和成员数据中继同为备用档，显式偏好仍生效。既有 TCP 和成员数据中继仍迁移原流。 [UDP](native-iroh-member-udp-147.md) · [TCP](native-iroh-member-tcp-152.md) · [成员数据中继](native-iroh-member-data-relay-153.md) · [原流恢复](native-iroh-network-switch-154.md)
- 重启联系记录保留已验证资格和有效期。首拨允许近期已验证地址与获准 relay 并行，只有公告的地址在后续参与，允许备用资源与缓存并行时仍受同一原期限约束。 [重启实测](native-iroh-member-redial-146.md) · [慢 relay](native-iroh-slow-relay-149.md)

历史办公室、热点、家宽样本仍按当时的版本和网络位置报告。本票的双 NAT 对照只证明可控输入下的行为，家宽 WAN 回程仍未知。 [历史 WAN 记录](network-demo-140-2026-11-18.md)

## 实测

两个网关分别改写实际 UDP 绑定。测试仅提供 MQTT 信令目的和两个独立 STUN 观察目的，禁用数据 relay 与 TCP。WAN 的正例仅在观察到该方向的原生增强探测报文后放行同方向 QUIC。旧 fork 在同一实验完成 TLS 与双端成员认证，目标在 12 秒内无业务请求。融合版双向在 2.093 和 2.095 秒收到首份 `first reply`，目标实际读出 `first request`。双方的 TLS attempt、增强 round 和业务流编号两端相同，本地 stable ID 分别与自己的首拨认证匹配。实际 NAT 源端口、物理绑定、选中路径和 fresh ACK 可独立读取，信令路径在交付时退役。控制组保持 MQTT 并封锁所有对端 UDP，两个方向均在 8.001 秒结束，目标未获业务。四组独立 broker 各记录一个首拨 Initial。 [实验脚本](../../scripts/check_member_fusion_nat.py) · [业务工人](../../tests/support/native_service_nat.rs)

维护应用 demo 验证了共享原连接上的流错误隔离与真实关闭来源。3 秒请求超时后，同一原连接的后续请求读到有效回复，明确关闭原连接后才拨通新连接。独立前台流由双方持有时，原连接保持开启。释放前台流后，原连接在约 3 秒关闭，期间成功回读三条周期检查。普通端点 2/2、受限端点 19/19、TCP-only CLI 和同承载独立库接口均通过。150 的显式融合学习用原流成功读回。132 的持续黑洞组在低／高 RTT、持续／单次写入四组分别在 1.162、1.154、3.004 和 2.570 秒读回原流，早于既有 6 秒期限。 [demo 入口](../../src/bin/miopunch-network-demo.rs) · [黑洞测试](../../tests/support/member_fusion_udp.rs)

增强 TCP 实测禁用 UDP，普通 TCP 公告只含不可拨的 `0.0.0.0`。两端在健康 relay 上开前台流并读回第一笔数据。STUN 观察者从两个真实监听端口读到 `127.0.0.1` 映射，两端按对应地址 prime。抑制 ACK 时原流仍有正确回读。放行后两端在同一原连接上取得新鲜 ACK，并从胜出路径消费同一原流的新字节。正例两端各观察一次，融合拨号总共一次。取消组在完成观察和路径建立后释放最后前台流，两端退役待验证候选，原连接上的维护流继续经 relay 读回。重试组由实际观察者拒绝首轮两次 STUN 请求，运行业务继续走原 relay，收到网络代际通知后两端在同一原连接取得新的物理观察和 ACK，原业务流在 TCP 上读回正确数据。已有普通融合 TCP 测试 6/6、demo 4/4，WASM 复编通过。 [真实维护宿主测试](../../tests/network_maintenance.rs)

同一代际对照拒绝首轮两次 STUN 请求后，原实现持续传输业务但在 4 秒内未发出第二轮观察。修订后的入口配置 600 毫秒探索间隔，1.210 秒后重试，期间从同一原双向流读回 48 笔请求。两端复用原 stable ID 和流编号，取得新鲜 ACK 后确认同一原流的新字节实际经过 TCP。 [同代际退避测试](../../tests/network_maintenance.rs)

退役已取得新鲜 ACK 的 TCP 路径 1 后，同一个双向流经 relay 确认序号 4。旧实现 5 秒内没有再发出 TCP 观察。修订后，在同一网络代际重新观察两个监听端口，从 TCP 路径 2 取得双端新鲜 ACK，0.959 秒后从该路径确认原流序号 5。全程只拨号一次。 [直连退役测试](../../tests/network_maintenance.rs)

现有 `NativeClient` 原流在受限恢复后继续向受保护转发目标交付序号，两端各自核对原 stable ID 与原流。普通 MQTT-only 首拨和四个慢 relay 的 MQTT 后备在原实验宿主均通过，后者在 2.122 秒返回，符合原有 4 秒期限。单独的 loopback-only 命名空间会使普通 MQTT 首拨在 IP 路径建立后报告 `closed path`，148 的冻结产物在同一命名空间复现同样的路径关闭和业务超时。两个用例复用真实宿主上的原断言。 [原生入口回归](../../tests/native_service_streams.rs)

143 和 151 记录的 WASM E0433 和 E0061 由公共 iroh 错误码和既有维护 lease 的同一 Hello 封存修复。Chromium 与原生在受保护 WSS 上完成双向业务，校验了 26 字节样本以及撤权前 17 字节样本。撤权关闭原连接及拒绝新的拨号，重载后继续生效。额外的 MID1 恢复用例在候选和只含三处编译修复的 `086cf02d` 对照都保持原 stable ID，按 2 秒调用期限返回 `unreachable` 和 `no permitted direct mechanism`，分别耗时 753 与 813 毫秒。同一个测试在两版都停在预期 `DataPathUnavailable` 的错误文案断言。未修改恢复协议或该断言。 [143 浏览器基线](native-iroh-member-fusion-143.md) · [现行 Chromium 入口](../../browser-probe/harness/run-network.mjs)
