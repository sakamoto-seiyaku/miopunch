# MIOPUNCH-57：单侧规律 NAT4 与 NAT3 的同 socket 预测

2026-09-29。本票把用户已确认的「依据反射探测行为自动选择打洞算法」接回 Rust 路径。旧 Go 的 `nat/classify.go` 从多个 STUN 反射地址给出 Easy/Hard 与粗略 `RegularPortsChange`，`internal/punchdecision/nathole_analysis.go` 将一侧规律 Hard、一侧 Easy 选为 mode 1、Hard 先发、Easy 对候选端口范围探测，`internal/punching/punching.go` 执行。旧 `RegularPortsChange` 仅看最大最小差 1–5，未证明观测顺序、方向或固定步长；旧 mode 1 的 TTL 4/7、2000 ms 延迟和最多 10 端口不作为新默认值。本票保留行为驱动的选法和双方角色，在同一 iroh QUIC UDP socket 上以顺序 QAD 证据计算有符号下一端口，只探测一个预测目标。双侧规律、随机映射和全矩阵默认调度分别留给 Plane 58、59、60；本票 `auto` 是受控试验入口。

## 运行时数据流与边界

`Endpoint::udp_observation()` 仍给出当前 QAD 公网候选、新鲜度和 `mapping_varies_by_dest`，供原 27/56 路径使用。它聚合结果，不能给出反射服务逐目的地、逐时刻的端口序列。新增 `Endpoint::observe_udp_mapping_sequence(&RelayMap)` 使用该 endpoint 底层 `noq::Endpoint` 和 UDP socket，按四个本地配置的 TLS QAD 目的地顺序请求；每项保留真实解析后的目的 `IP:port`、反射地址和接收时刻。这里的 QAD 与旧 STUN 都是外部映射观察机制；QAD 走 QUIC 地址发现，不能从这些回包推断 APDF 过滤。真实过滤另由网关跨源端口预检与 conntrack 证明。

`nat_mapping::classify` 要求四个独立实际目的地、同一公网 IP、严格递增的观测时刻、两秒以内的新鲜样本，且三个相邻端口差一致。差为零是 Stable；非零且绝对值不超过 **2** 是 Sequential（含递增、递减及固定步长）；反转、跳步、换 IP、目的地重复、过期、步长超出本票实测范围或预测越出 1–65535 都是 Unknown。日志中的 `confidence=consistent_3/3_transitions` 是样本一致性，不是跨网络成功概率。网络变化使整份观察立即失效。两秒是经下述 QAD→peer 首包时差选出的本地证据有效期，不代表 NAT 映射实际寿命。可共享的 `NodeProbeBudget` 以原子扣减限制每节点额外 QAD 观察和向 peer 的探测，已用额度不会因另一尝试重置；最终策略为每端分配额外 4 次 QAD 与 **1 个** peer 数据报，并用单测检查跨尝试耗尽。CLI 一次进程只运行一轮，正式多连接维护档位仍要在后续集成中验证。

在同一经 TLS 验证身份、SCRAM 准入秘密证明且双方成员资格均有效的 `ControlSession` 内，双方交换本地分类摘要；仅 `(Sequential, Stable)` 或反向组合进入旧 mode 1 等价角色，接收侧由远端最后反射端口加有符号步长推得一个目标。服务端新随机轮次的 `AUTO_OFFER → AUTO_READY → START → ACK` 防止旧启动消息重用。每次发送仍重查本地成员资格、观察有效性和共享预算；远端步长超过本地上限或请求格式错误不会触发探测。Hard 侧先从原 UDP socket 发一包，再经受保护控制流发送同轮 `SENDER_PRIMED`；Stable 侧收到并确认后才向唯一预测目标发一包。这个因果屏障取代猜测性的双方延时。拨号侧在三秒内通过 `open_data_path_report` 验证同一 QUIC 连接的 IP path，双方 `PROMOTED/COMMITTED` 后才释放队列业务。本票没有实现产品级全矩阵默认路由。

## 参数对照

[32 次单包/双包交错对照](../evidence/mio57-nat4-regular/tuning/summary.json)在四种步长、双拨号角色、相同一次性身份和秘密下运行：每端 **1 包 16/16、2 包 16/16**，成功客户端路径中位数 **307 / 357 ms**，完整 fixture 中位数 **6.654 / 6.729 s**。一包在此内核条件下没有降低成功率，少一次 peer 发送且更快，所以最终选每端一包；第二包只保留为受上限限制的实验对照，不作产品默认。两个档位均用三条预路径控制消息/端来完成已认证的首包屏障。[逐次 CSV](../evidence/mio57-nat4-regular/tuning/trials.csv)和完整 pcap 保存该判断。

[抓包时间计算](../evidence/mio57-nat4-regular/tuning/timing.json)读取 32 次调参试验的 Linux SLL2 pcap：末个 QAD 目的地回包至本端首个 peer 探测为 **431–782 ms**，Hard 首包比 Stable 首包早 **1–69 ms**；测量代码为 [`nat4_pcap_timing.py`](../../scripts/nat4_pcap_timing.py)。因此取两秒证据有效期，给本拓扑最慢观测年龄约 2.5 倍余量，同时让故意晚于两秒的真实控制会话在发包前拒绝。所有成功路径交付的客户端耗时最长 **377 ms**，实验 auto 路径等待选三秒；协调消息三秒上限通过洁净正例和故意延迟四秒的负例检查。这些是本实验的安全/维护界限，不推断移动网 NAT 寿命或任意跨网延迟分布。步长只启用真实内核已覆盖的 ±1/±2；其他固定步长仍归 Unknown 并交给后续票验证。

## 内核拓扑和可重跑证据

隔离 privileged Debian 容器内建立 WAN、两网关、两节点 Linux netns。A `10.27.1.2:42027` 的 iptables SNAT 按目的地为四个 QAD UDP 端口（46761–46764）及 B peer 目的地赋不同外部端口；B `10.27.2.2:43027` 对所有目的地固定映射。两侧仅允许已建立的反向 UDP 五元组通过 WAN→LAN，因此过滤为 APDF。SNAT 明确编程为 `+1`、`−1`、`+2`、`−2` 四种控制型 APDM 分配序列；这是内核 NAT 功能和算法适用条件验证，不等同运营商网关自然分配频率。每个 trial 的 [网关 SNAT/conntrack](../evidence/mio57-nat4-regular/matrix/up1-a-01-auto/nat-a-conntrack.txt)、[错误端口丢包计数](../evidence/mio57-nat4-regular/matrix/up1-a-01-auto/filter-a-wrong.txt)和 [WAN pcap](../evidence/mio57-nat4-regular/matrix/up1-a-01-auto/wan.pcap)互证；预检后清空 conntrack，未给应用提前开洞。

最终 [逐次矩阵](../evidence/mio57-nat4-regular/matrix/trials.csv)和[汇总](../evidence/mio57-nat4-regular/matrix/summary.json)包含参数校准后的 **32 次**试验：四种步长 × 两个拨号角色 × 每格基础/增强各两次。每对的基础/增强顺序交错，使用同一次性身份与秘密。客户端和服务端日志记录各 QAD 实际目的地、相对接收时间、实际映射、分类、预测目标、轮次、原始 socket、本连接 stable id、选中 `PathId(1)` 和 `NAT_DATA`；业务发送前后 MQTT 计数不增加。每次 WAN pcap 和网关计数、conntrack 都保留。[首包屏障前矩阵](../evidence/mio57-nat4-regular/exploratory/pre-role-order-matrix/summary.json)和[两包参数校准前矩阵](../evidence/mio57-nat4-regular/exploratory/pre-calibration-matrix/summary.json)单独保留，不计入下表最终统计。最终双角色首包顺序见调参抓包与上节时间计算；预检包和 11 字节应用探测可按长度区分。

| NAT4 序列 | NAT4 拨号：基础 / auto | NAT3 拨号：基础 / auto |
| --- | ---: | ---: |
| +1 | 0/2 / 2/2 | 0/2 / 2/2 |
| −1 | 0/2 / 2/2 | 0/2 / 2/2 |
| +2 | 0/2 / 2/2 | 0/2 / 2/2 |
| −2 | 0/2 / 2/2 | 0/2 / 2/2 |

合计基础 **0/16**，auto **16/16**。基础单 QAD 候选在这些 APDM+APDF 网关中不等于 peer 映射，双方均无法打开所需过滤；增强的下一端口预测命中。auto 成功 trial 的客户端路径交付中位数约 **305 ms**；基础没有成功路径时间。完整 fixture 墙钟中位数基础 **18.228 s**、auto **6.628 s**，其中包括 netns、broker、QAD 和抓包进程；fixture CPU 中位数约 1.243/1.161 s，不能解释为节点 CPU 改善。基础每端 1 个 QAD 连接、1 包向 peer 的应用探测、1 条预路径控制消息；auto 每端先有 1 个原候选 QAD 连接，再运行 4 个顺序 QAD 连接（含一次访问原观察点），向一个预测目标也只发 **1 包**，预路径控制每端 3 消息，另有 UDP 提交确认。两策略均只绑定原 iroh 的 IPv4/IPv6 sockets；实际 IPv4 QAD、探测及业务 QUIC 共用绑定的 IPv4 socket。auto 多四次 QAD QUIC 连接和两条控制消息的网络成本不可从应用探测包数抹去。

## 失败检查与适用范围

[14 项失败汇总](../evidence/mio57-nat4-regular/failures/summary.json)及各项 WAN pcap/两端日志保留：错候选、观测后方向反转、实际下一步越出预测目标和双向禁 UDP 均无业务，各端最多 **1 包**、完整 fixture 约 9.3–9.5 秒；旧候选、观测过期、控制迟到、取消、观察序列内反转、恶意 `step=100` 扩大要求、非成员、撤销、无秘密和错秘密均为 **0 应用探测**，约 6.2–10.9 秒结束。四种 UDP 路径失败后，原认证 QUIC 控制连接仍能在同一 stable id 上完成 `PATH_FAILED/ACK` 与收妥确认；客户端严格接收末帧，服务端等 QUIC 确认末帧送达才退出，两端日志均有 `CONTROL_ALIVE_AFTER_UDP_FAILURE`。它只承载受限控制，未释放业务。禁 UDP、端口反转与越界是明示的故障注入，不被计作自然故障率。额外 [EIM 回归](../evidence/mio57-nat4-regular/regression/eim-baseline-a/client.log)证明单候选基础路径仍可交付；两侧都分类 Stable 时 auto 拒绝 mode 1、0 探测，等待后续全矩阵路由。错误控制声明不能把目标扩大成端口扫描，准入失败未披露候选或交付业务。

另有[真实成员 TCP 承载回归](../../tests/admission_gate.rs)：同一已获准 TCP QUIC 连接先交付业务，NAT4 决策返回 Unknown 且共享预算耗尽后仍在**同一 TCP stable id** 再次交付业务。它证明失败的 UDP 策略选择不会撤销另一条既有获准数据连接；四个内核失败场景则证明原失败连接的控制阶段仍可用。两项证据不能证明那条失败的 QUIC 连接后来切到 TCP 数据路径；正式同连接备用路径竞争仍由后续集成验证。

本结果只证明该控制型端口分配和 APDF 内核拓扑；未测真实家庭/移动 NAT 的分配竞争、映射寿命、跨运营商成功率、长时间身份状态或多连接维护档位。规律条件失效时只作有界尝试，不改变其他已获准连接的资格；不能据本票宣称所有 NAT4 都可直连，也不据此默认启用额外 QAD 观察。后续 58/59/60 将补双侧规律、不规则和全矩阵自动选法。

最终代码已通过 `cargo check --locked --workspace --tests`、`cargo fmt --all -- --check`、shell/Python 语法和差异空白检查。使用专用真实 MQTT broker、串行运行的 `cargo test --locked --workspace -- --test-threads=1` 结果为 **187 passed、3 ignored、0 failed**（相对 56 新增四个映射/预算单测）；含上述成员 TCP 继续通信测试。默认沙箱会拒绝既有 TCP 测试的本地 bind，完整套件在获准的本地网络环境运行，broker 随后停止。

在允许 privileged Docker 和本地构建 Rust 二进制的 Linux 上可重跑：

```bash
cargo build --locked --bin udp_nat_probe --bin udp_observation_probe
docker build -f scripts/nat4_lab.Dockerfile -t miopunch-nat57-lab:bookworm scripts
mkdir -p /tmp/mio57-rerun
docker run --privileged --rm --network none \
  -e MIOPUNCH_NAT_REPEAT=2 -e MIOPUNCH_NAT_TEST_SEED=mio57-final-20260929 \
  -v "$PWD":/lab:ro -v /tmp/mio57-rerun:/evidence \
  miopunch-nat57-lab:bookworm python3 /lab/scripts/nat4_matrix.py \
  /lab/target/debug/udp_nat_probe /lab/target/debug/udp_observation_probe /evidence
```

用同一容器与挂载，将命令末尾换为 `python3 /lab/scripts/nat4_failures.py ... /evidence` 即可复现失败矩阵。脚本不使用旧 Go/OpenSpec 测试门槛。
