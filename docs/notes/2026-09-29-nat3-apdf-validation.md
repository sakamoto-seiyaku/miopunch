# MIOPUNCH-56：NAT3 端口级过滤下的双端 UDP 协调

本票在隔离 Debian 12 QEMU VM 中，以两个 Linux `netns` 网关、真实 iptables/conntrack、Mosquitto、TLS QAD 观察服务和原生 iroh/noq QUIC 端点运行。入口为 [`nat3_lab.sh`](../../scripts/nat3_lab.sh)、[`nat3_matrix.py`](../../scripts/nat3_matrix.py) 和 [`nat3_failures.py`](../../scripts/nat3_failures.py)。[最终 32 次配对矩阵](../evidence/mio56-nat3-apdf/matrix/summary.json)及逐次 [CSV](../evidence/mio56-nat3-apdf/matrix/trials.csv)保留每次节点日志、WAN pcap、网关计数和 conntrack 快照；[失败矩阵](../evidence/mio56-nat3-apdf/failures/summary.json)另行保存。

入库时仅规范文本计数文件的行尾空白和 CSV 换行，并把汇总中的证据路径改为相对于各矩阵目录的名称；计数值、节点日志内容和 pcap 包流未修改。压缩的早期探索归档保留原始输出。

## 内核拓扑与分类

节点 A `10.27.1.2:42027` 映射为 `100.64.27.2:42027`，节点 B `10.27.2.2:43027` 映射为 `100.64.27.6:43027`。固定 SNAT/DNAT 使同一源端口向 QAD 与对端目的地保持同一公网 IP:port，即 endpoint-independent mapping。NAT3 侧只允许与本地已发送 UDP 五元组相匹配的 conntrack `ESTABLISHED` 回包，即 address-and-port-dependent filtering。两个拓扑分别是 NAT3×NAT3，及 A 为 NAT3、B 用 `recent` 按 IP 放行的 NAT3×ADF 对照；角色 A/B 均轮流拨号。

每个 trial 在端点启动前先向网关发送真实 UDP：在目标 NAT 已向某 `IP:port` 发包后，同一 IP 的**错误源端口**被 NAT3 丢弃、正确端口通过；ADF 对照侧错误端口可通过。网关两侧 [规则计数](../evidence/mio56-nat3-apdf/matrix/nat3_nat3-a-01-coordinated/filter-b-wrong.txt)、[正确端口计数](../evidence/mio56-nat3-apdf/matrix/nat3_nat3-a-01-coordinated/filter-b-right.txt) 和 [WAN 抓包](../evidence/mio56-nat3-apdf/matrix/nat3_nat3-a-01-coordinated/wan.pcap)互证，预检后清空 conntrack 与 ADF `recent` 项，因此它不为应用提前开洞。QAD 返回的两个公网地址见配对节点日志；随后 WAN 抓包可见来自对应已绑定 QUIC UDP 端口的 11 字节探测与双向 QUIC 包。没有数据 relay。

## 探测与准入

基础对照沿用 27 的双方各一次同 socket `prime_udp_path` 加拨号方显式 QUIC 路径。增强策略在同一经 TLS 身份认证、SCRAM 准入证明和双端阶段确认的控制连接上执行 `OFFER → READY(服务端新随机轮次) → START → ACK`。双方仅向对方经受保护控制流给出的一个精确公网 `IP:port` 探测，最多各 8 包、间隔 75 ms；每包重查本地成员资格与自身 QAD 候选新鲜度。拨号方验证 IP 路径后先关闭 MQTT 控制路径，通过同一 QUIC 连接上的 UDP 发送带轮次的 `PROMOTED`；接收方确认后提交并回 `COMMITTED`，然后才交付 `NAT_DATA` 业务。旧轮次、重复 START 和取消不能启动第二轮探测。增强流程没有新增 UDP socket 或 QUIC 连接，也没有使用低 TTL。

实验端点使用 29 的 `AdmissionGate`：本轮特定 ALPN 为受限 Bootstrap，只有同时持有当前秘密且双方是本地已应用视图中的成员，才可透露候选、探测和提交路径。身份取真实 TLS 对端；非成员、撤销、无秘密与错误秘密负例均保留。实验视图和秘密由一次性 fixture 提供；这不替代正式签名管理状态加载或 42 的正式 MQTT 载体集成。业务字节只从提交后的 `DataSession` 发送，两个端点检查业务前后 MQTT 发送计数不增加，且确认选中 IP `PathId(1)` 与原 stable connection ID。

## 配对结果与成本

每个固定拓扑与拨号角色交错运行基础/增强各 4 次，奇偶 trial 交换先后次序。每对使用相同的一次性节点身份与秘密，测试种子 `mio56-final-20260929` 只用于实验 fixture；内核调度并非固定。全部 32 次、每个 trial 的公私侧证据均在最终矩阵中。

| 拓扑 | 基础成功 | 增强成功 | 基础/增强客户端路径交付中位数 | 基础/增强单次完整 fixture 中位数 |
| --- | ---: | ---: | ---: | ---: |
| NAT3×NAT3 | 8/8 | 8/8 | 278 / 1263 ms | 3527 / 4481 ms |
| NAT3×ADF | 8/8 | 8/8 | 279 / 1264 ms | 3576 / 4531 ms |

增强在这组洁净、固定端口 EIM/APDF 内核条件下**没有提高成功率**；每端精确候选探测从 1 增至 8，预路径控制消息从每端 1 增至 2，另有 UDP 路径上的一次 `PROMOTED/COMMITTED` 往返。两侧仍各只绑定原 iroh 的 IPv4/IPv6 socket，IPv4 QAD、探测和 QUIC 共用同一 socket。完整 fixture 的中位 CPU 约从 1.0–1.1 s 小幅上升；它包含 namespace、Mosquitto、QAD 和抓包进程，**不是节点 CPU 基准**。路径时间从候选交换结束后计到 `DATA_OK`，包括增强探测窗口和提交确认；额外约 1 s 是本预算在成功场景的实际代价。参数是可重跑实验预算，不是自动启用的产品默认值。

早期未配对 32 次探索矩阵的[汇总](../evidence/mio56-nat3-apdf/exploratory/unpaired-summary.json)与[全量归档](../evidence/mio56-nat3-apdf/exploratory/unpaired.tar.gz)，及串行提交前 32 次配对矩阵的[汇总](../evidence/mio56-nat3-apdf/exploratory/pre-fix-paired-summary.json)与[全量归档](../evidence/mio56-nat3-apdf/exploratory/pre-fix-paired.tar.gz)分别保留；它们不是上表结果。其中一次 [真实连接关闭竞态](../evidence/mio56-nat3-apdf/diagnostics/control-close-race/)显示双方已看到 IP 路径后，对端先关闭原 MQTT path，本端 `path.close()` 返回 `ClosedPath`，iroh fork 曾错误关闭整条 QUIC 连接。现在只把这个已经关闭的返回值视为完成，`LastOpenPath` 等其他错误仍使提交失败；修复见 [`UPSTREAM.md`](../../vendor/iroh/UPSTREAM.md)。探索矩阵与最终矩阵均保留，不以早期偶发差异覆盖最终零增益结论。

## 失败范围与复现

受控失败检查包括错误端口、旧 QAD 候选、迟到控制、取消、上一条同身份/同秘密连接的真实轮次、当前轮次重复 START、WAN 双向禁 UDP、非成员、已撤销成员、无秘密与错误秘密。它们分别结束为 `PATH_UNAVAILABLE`、`COORD_UNAVAILABLE` 或准入拒绝；没有 `DATA_OK`。禁 UDP 是明确的内核故障注入，迟到控制是明确的应用延迟注入，不作为自然故障频率。逐项时限、探测数和日志以失败矩阵为准。

| 失败类别 | 场景 | 完整 fixture 耗时 | 双端探测数 |
| --- | --- | ---: | ---: |
| 精确候选或 UDP 不通 | 错端口 / WAN 双向禁 UDP | 15.9 / 15.9 s | 各 8 / 各 8 |
| 轮次和控制 | 旧候选 / 控制迟到 / 取消 / 旧轮次 / 重复 START | 3.4 / 7.1 / 3.2 / 3.3 / 3.2 s | 全部 0 |
| 准入 | 非成员 / 已撤销 / 无秘密 / 错误秘密 | 3.2 / 7.7 / 7.7 / 3.1 s | 全部 0 |

所有负例中的节点命令均未达到 40 秒外部超时；错误端口只向一个错误端口发包，没有展开扫描。旧轮次测试先用**同一对身份和秘密**完成一条真实连接，再把其服务端挑战值送入新连接；重复 START 则在当前轮次收到 ACK 后发第二条控制流。两种重放均未建立第二个探测窗口或交付业务。授权负例的本地 QAD 可以先运行，但候选未向未获准对端披露，应用探测数为零。

在具备 root、`iproute2`、iptables、conntrack、tcpdump、Python 3、Mosquitto 的一次性 Linux VM 中，构建 `udp_nat_probe` 与 `udp_observation_probe`，复制两二进制和上述三个脚本，然后运行：

```bash
sudo env MIOPUNCH_NAT_REPEAT=4 MIOPUNCH_NAT_TEST_SEED=mio56-final-20260929 \
  python3 nat3_matrix.py ./udp_nat_probe ./udp_observation_probe /tmp/nat3-matrix
sudo python3 nat3_failures.py ./udp_nat_probe ./udp_observation_probe /tmp/nat3-failures
```

该结果只覆盖这两种固定映射拓扑、本地 QEMU 调度与一次性成员 fixture；没有跨运营商、真实移动 NAT 超时分布、可变公网端口或 NAT4 成功率证据。自动触发条件应限于已认证且当前成员、双方都有新鲜精确 EIM 候选、路径仍需改善、节点预算允许的一轮；这组无增益数据本身不支持默认多发 7 包。后续 57–59 分别验证规律及不规律 NAT4，47 再处理多路径质量和获准回退。

若要为 NAT3 启用重复窗口，下一项可执行验证是先在真实家庭网关和移动接入上测得**实际** UDP 过滤寿命、MQTT 控制送达延迟及单次探测失败模式，再按这些观测值复现对应内核规则，配对重复基础/增强试验并按节点预算核对收益。旧 F-001 时序经验没有给出可移植的寿命或延迟数值，因此本票没有为了制造正收益而调短 conntrack 超时或注入任意长延迟。
