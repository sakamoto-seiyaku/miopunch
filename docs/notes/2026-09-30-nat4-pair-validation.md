# MIOPUNCH-58：双侧规律 NAT4 的自动协同预测

2026-09-30。旧 Go `internal/punchdecision/nathole_analysis.go` 在双 Hard 且两侧 `RegularPortsChange` 时先试 mode 3：双方扫近邻端口，按发送/接收角色尝试多种 TTL 与延时。旧特征仅看端口跨度，不能证明有符号步长；10 端口、低 TTL 等旧常量也未在当前共用 QUIC socket 上校准。本票沿用「S 观测选择算法」和双侧角色的已定方向，用 57 建立的同 socket、有序 QAD 证据选一个下一端口，并以同轮受保护屏障给双方 APDF 过滤器排序。没有复制旧的宽范围扫描或逐包 TTL 改写。

## 运行路径和决定

每端先从原 iroh endpoint 的 IPv4 QUIC UDP socket 取得一个基础 QAD 候选，再对四个已配置的 QAD `IP:port` 依次观察。`nat_mapping::classify` 复用 57 的四目的地、严格时序、两秒新鲜度和 `±1/±2` 固定步长限制。双方 TLS/QUIC 身份、SCRAM 准入秘密、当前成员资格成立后，才在原受限 `ControlSession` 中交换本地分类摘要。只有两侧都是 `Sequential` 时进入本票的 `DualFirst/DualSecond`；`Sequential×Stable` 保留 57 的 mode 1 等价选择，`Stable×Stable` 和 `Unknown` 不误选双规律分支。各端从**对端**最后观测端口加对端有符号步长，得出唯一目标公网 `IP:port`，不按实验拓扑标签、人工 NAT 类型或手填步长选策略。

双规律时由连接服务侧担任 `DualFirst`，与 NAT A/B 所处方位无关：首侧从原 socket 向对端预测目标发 1 包，受保护地发同轮 `SENDER_PRIMED`；次侧确认后向首侧预测目标发 1 包，并发 `RECEIVER_PRIMED`；首侧确认次侧已发后才开始 QUIC 路径验证。双方过滤器因此先后打开；后续 QUIC 路径验证按既有协议发包。本票的应用 prime 与 QUIC 验证报文分别计数，不能把后者算作免费。每侧只碰一个预测目标端口；最终首侧和次侧各 1 个应用探测包，额外 QAD 上限每节点 4 次，均扣共享 `NodeProbeBudget`，不因另一尝试重置。每次发送重查成员资格和本地映射证据，两端共享的轮次及受限连接在路径提交时继续核对。客户端在 3 秒窗口内用原连接 `open_data_path_report` 确认 IP path，双方 `PROMOTED/COMMITTED` 后才释放预排队的业务；业务不得落到 MQTT 信令载体。

当配置了四个观察点且有准入材料时，探针的正常运行自动执行观测与选策，不要求设置 `MIOPUNCH_NAT_STRATEGY`。`baseline`/`coordinated` 参数仅保留为 A/B 实验对照。观察服务地址仍属于受信任的产品资源配置；本票没有决定真实默认服务实例。旧 `auto` 分支只支持单侧规律，现在按双方特征进入双规律分支，接缝仍是同一连接、同一原 UDP socket。一次性测试程序当前若把双 Stable 分类交给此增强入口会有界拒绝；mode 0–4 的统一默认调度由 MIOPUNCH-60 验收，不能把本票称为已交付全矩阵。

## 内核映射与过滤预检

隔离 Debian privileged 容器建立 WAN、A/B 两个独立网关和两个节点的 Linux netns。每侧四个 QAD 目的地是同一 WAN IP 的四个不同 UDP 端口；各网关明确编程对应的固定步长 SNAT。peer 的预测目的端口和同一 peer IP 上相邻的另一个目的端口走**不同的显式 SNAT 规则**；其他 peer 端口也走显式替代映射，不落到 MASQUERADE 的偶然端口保留。正例 `+1×+1` 的预检 conntrack 显示 A 从同一 `10.27.1.2:42027` 到 B `:43035/:43036` 分别映为 `100.64.27.2:42035/:42037`；B 从 `10.27.2.2:43027` 到 A `:42035/:42036` 分别映为 `100.64.27.6:43035/:43037`。这与随后四 QAD 端口的连续映射一起证明受控的 APDM，不依赖 NAT4 标签推断。

两个网关的 WAN→LAN 只允许 conntrack 已建立的反向 UDP 五元组。预检中从正确公网端口发回的 peer 包通过 FORWARD，从相邻错误来源端口发回的包落在 FORWARD DROP；两侧均抓取网关计数和 WAN pcap。每次预检后清空两侧 conntrack，再启动被测 endpoint，避免预检提前开业务过滤器。此拓扑验证的是**受控规则下**的 APDM+APDF 和算法适用条件，不证明真实运营商网关自然分配的频率或寿命。


## 最终完整双规律矩阵

[最终逐次 CSV](../evidence/mio58-nat4-pair/matrix/trials.csv)保存 **128 次**试验：两端各支持 ±1、±2 的全部 **16 个有序组合**，每个组合双拨号角色、每格基础/自动各两轮。AB/BA 顺序交错，每对使用相同一次性身份与秘密。最终被测自动路径不设置 MIOPUNCH_NAT_STRATEGY、不读取拓扑标签或手填步长；网关配置标签只作用于实验 fixture。此前的校准、未证明严格 APDM 的拓扑与 harness error 单独留在 [exploratory/](../evidence/mio58-nat4-pair/README.md)，不计入最终分母。

| A 端口步长 | B 端口步长 | A 拨号：基础 / auto | B 拨号：基础 / auto |
| --- | --- | ---: | ---: |
| +1 | +1 | 0/2 / 2/2 | 0/2 / 2/2 |
| +1 | −1 | 0/2 / 2/2 | 0/2 / 2/2 |
| +1 | +2 | 0/2 / 2/2 | 0/2 / 2/2 |
| +1 | −2 | 0/2 / 2/2 | 0/2 / 2/2 |
| −1 | +1 | 0/2 / 2/2 | 0/2 / 2/2 |
| −1 | −1 | 0/2 / 2/2 | 0/2 / 2/2 |
| −1 | +2 | 0/2 / 2/2 | 0/2 / 2/2 |
| −1 | −2 | 0/2 / 2/2 | 0/2 / 2/2 |
| +2 | +1 | 0/2 / 2/2 | 0/2 / 2/2 |
| +2 | −1 | 0/2 / 2/2 | 0/2 / 2/2 |
| +2 | +2 | 0/2 / 2/2 | 0/2 / 2/2 |
| +2 | −2 | 0/2 / 2/2 | 0/2 / 2/2 |
| −2 | +1 | 0/2 / 2/2 | 0/2 / 2/2 |
| −2 | −1 | 0/2 / 2/2 | 0/2 / 2/2 |
| −2 | +2 | 0/2 / 2/2 | 0/2 / 2/2 |
| −2 | −2 | 0/2 / 2/2 | 0/2 / 2/2 |

合计基础 **0/64**，自动 **64/64**。基础的单 QAD 映射与实际 peer 目的地映射不一致；自动策略根据对端四个有序观测的最后端口和有符号步长命中下一映射。自动成功客户端路径耗时中位数 **367.0 ms**，范围 **280–393 ms**；基础无成功路径时延。完整 fixture 墙钟中位数基础/自动 **18.953 / 7.479 s**，fixture CPU 中位数 **1.367 / 1.298 s**，包含环境、预检、broker/QAD、端点和抓包，不当作节点功耗或实际业务 CPU 指标。

[独立验证结果](../evidence/mio58-nat4-pair/matrix/verification.json)核对全部支持组合与双角色覆盖、对端 QAD → 预测目标、同一随机轮次、TLS/QUIC 对端身份、本端原 stable id 到业务的连续性和路径提交。每个成功端点只报告 1 个应用 prime，实际 source/destination 端口由 [64 份 WAN 抓包时间与成本统计](../evidence/mio58-nat4-pair/matrix/timing-and-costs.json)另行核对。首侧 prime 早于次侧 **1–70 ms**，末个 QAD 回包至本端首个 prime **468–932 ms**；所有发送保持两秒新鲜证据边界。SLL2 WAN 入方向计数避免把同帧的 ingress/egress 算两遍，也不按时间相近合并真实报文。

[实际 UDP 成本中位数](../evidence/mio58-nat4-pair/matrix/measurements.json)以每次试验两端合计：基础/自动均为 **2 个应用 prime**；prime 之后所有 peer UDP 数据报（包括 QUIC 路径验证、确认、业务及关闭）分别 **9.0 / 34.0**，UDP payload **8422.0 / 15965.0 bytes**。QAD 请求及响应全部生命周期数据报分别 **46.0 / 109.0**，UDP payload **31388.0 / 81260.0 bytes**。自动策略增加四次观察连接/端和受保护协调开销；“每端一包”只描述应用 prime 预算，不等于打洞的全部网络开销。自动两端各 5 发/5 收实际协调帧，基础候选交换各 1 发/1 收；MQTT 只承担受限控制，业务交付前后计数检查不允许 NAT_DATA 落入 MQTT。

## 参数对照与预算选择

[32 次首侧单包/双包交错对照](../evidence/mio58-nat4-pair/tuning/summary.json)覆盖四个代表有序组合（+1×+1、+1×−1、+2×−2、−1×+2）、双拨号角色、每格两轮。同一对照使用相同一次性身份及准入秘密，顺序交错。首侧 1 包、次侧 1 包为 **16/16**；首侧 2 包、次侧 1 包也为 **16/16**。成功客户端路径中位数分别 **365 / 323.5 ms**，完整 fixture 中位数 **7.483 / 7.482 s**，fixture CPU 中位数均 **1.365 s**。第二包在本拓扑更快约 41.5 ms，但未改善成功率；最终选择应用探测成本更低的 **1+1**。不宣称单包更快，也不从 16 次样本推断真实运营商故障率。受严格上限限制的第二包只保留为实验对照。

双方应用 prime 打开受控 APDF 过滤器之后，原 QUIC 连接继续发路径挑战、响应、确认及业务报文，不能将它们从成本中抹去。每端有原候选的 1 次 QAD 连接加 4 次额外有序 QAD 连接，只有同一个原 IPv4 UDP socket 向 peer 发包；iroh 原来还绑定 IPv6 socket，不能把“共用一个 IPv4 socket”误写成整个 endpoint 只绑定一个 socket。最终成本由真实 WAN 入方向抓包统计应用 prime、其后 peer QUIC 数据报和 UDP payload 字节，另计 QAD 双向数据报及 payload 字节；IP/Ethernet 头、MQTT/TCP 信令开销不包含在这些 UDP 字节中。

成功双规律轮次每端实际发送并接收 **5** 条受保护协调帧：特征交换、启动确认、首侧发包屏障、次侧发包屏障和路径提升/提交。失败路径另有收妥确认，实际可达 **6** 条/方向；没有进入发包阶段的失败按真实帧日志统计。旧调参日志中的 CONTROL_MSGS=4 仅是计划的提升前协调交换数，不能当作包含终态的总成本。最终日志逐条记录 CONTROL_FRAME_SENT/RECEIVED，逐次矩阵从这些事件计数。

## 失败与自动分支边界

[16 项独立内核失败检查](../evidence/mio58-nat4-pair/failures/summary.json)全部通过。错候选、一侧下一端口跳变、观测后方向反转、下一端口超出唯一预测目标和双向禁 UDP 共 **5** 项，双方实际各最多 **1** 个应用探测包，没有释放业务；原认证 QUIC 控制连接在同一 stable id 完成 PATH_FAILED/ACK 和收妥确认。旧候选、观测过期、控制迟到、取消、错轮次、观察序列不规律、远端 step=100 越权扩大声明、非成员、撤销、无秘密及错秘密共 **11** 项，独立 pcap 为 **0** 应用探测包。完整 fixture 耗时 **9.634–17.400 s**（包括环境建立、预检与关闭），全部未触发外层超时。故障映射在真实 APDM/APDF 预检并清 conntrack 后才注入，不能用预检自身的失败冒充应用有界失败。

[6 项自动分支检查](../evidence/mio58-nat4-pair/boundaries/summary.json)覆盖 Stable×Stable、Sequential×Stable 和相反组合。双 Stable 在增强入口中有界拒绝且不选 DualFirst/DualSecond；同拓扑原基础路径可交付业务。单规律的两种 NAT 方位和两个拨号角色都保留 MappingSideSender/StableSideReceiver 并交付业务。双规律的全部支持组合由最终完整矩阵单独统计。mode 0–4 的统一默认调度由 MIOPUNCH-60 接续，本票不把双 Stable 拒绝包装成完整自动路线。

[获准 TCP 连接回归](../../tests/admission_gate.rs)先在真实成员 TCP QUIC 连接交付业务，再令双规律决策遇到 Unknown 且共享探测预算耗尽，最后仍在该 TCP stable id 交付业务。这证明 UDP 选策失败不会撤销另一条既有获准数据连接；内核负例证明原失败连接的受限控制仍存活。两项证据不等于那条失败连接后来切到 TCP 数据路径，正式同连接备用路径竞争仍由后续集成验证。

## 验证范围和重跑

本票只验证受编程控制的 IPv4 APDM+APDF、两端固定步长 ±1/±2、两秒映射证据和三秒 QUIC 路径等待。没有测真实家庭/移动网的分配竞争、端口规律频率、映射寿命、跨运营商成功率、长期路径维护和节点功耗；不规则/随机映射交给 MIOPUNCH-59，完整 mode 0–4 默认调度交给 60。一次命中不使映射摘要变成身份或成员授权；失败不得扩大扫描、建立额外随机监听 socket，或把仅信令 MQTT 当业务备用。默认真实观察资源实例和多连接维护配置仍由后续已授权集成落实。

最终 `cargo check --locked --workspace --tests`、`cargo fmt --all -- --check`、Python/shell 语法与差异空白检查通过。使用专用真实 MQTT broker 串行运行的 [`cargo test --locked --workspace -- --test-threads=1`](../evidence/mio58-nat4-pair/verification/cargo-test.log)为 **189 passed、3 ignored、0 failed**。三项是既有上游忽略项：一个标为 flaky 的 gossip 测试和两个授权文档示例，不算本票完成能力。专用 broker 随后停止。双轴 code-review 发现的报文去重、缺日志误判零包、默认轮数不一致和控制成本遗漏已修复并复审；正式结论见 [评审记录](../evidence/mio58-nat4-pair/verification/code-review.md)。

可在允许 privileged Docker、本地 Rust 构建与真实 sockets 的 Linux 重跑。冻结脚本/二进制后再启动，避免运行中的 shell 文件被修改；每次试验的密钥及准入秘密只在临时目录保存，证据不含私钥或秘密。

```bash
cargo build --locked --bin udp_nat_probe --bin udp_observation_probe
docker build -f scripts/nat4_lab.Dockerfile -t miopunch-nat57-lab:bookworm scripts
mio58_snapshot=$(mktemp -d /tmp/mio58-source.XXXXXX)
mkdir -p "$mio58_snapshot/scripts" "$mio58_snapshot/bin" /tmp/mio58-rerun
cp scripts/nat4_pair_* scripts/nat4_pcap_timing.py "$mio58_snapshot/scripts/"
cp target/debug/udp_nat_probe target/debug/udp_observation_probe "$mio58_snapshot/bin/"
docker run --privileged --rm --network none \
  -e MIOPUNCH_NAT_REPEAT=2 -e MIOPUNCH_NAT_TEST_SEED=mio58-final-20260930 \
  -v "$mio58_snapshot":/lab:ro -v /tmp/mio58-rerun:/evidence \
  miopunch-nat57-lab:bookworm python3 /lab/scripts/nat4_pair_matrix.py \
  /lab/bin/udp_nat_probe /lab/bin/udp_observation_probe /evidence
python3 -B scripts/nat4_pair_verify.py /tmp/mio58-rerun
python3 -B scripts/nat4_pair_pcap.py /tmp/mio58-rerun
```

将容器中的脚本替换为 `nat4_pair_tuning.py`、`nat4_pair_failures.py` 或 `nat4_pair_boundaries.py`，并挂载各自独立证据目录，可重跑预算对照、16 个负例及 6 个分支边界。调参脚本仅允许首侧 1/2 包的有界实验输入，最终默认不依赖该输入。运行快照哈希保存在 [runtime-sha256.txt](../evidence/mio58-nat4-pair/verification/runtime-sha256.txt)；报表检查器可以在实验结束后独立执行。
