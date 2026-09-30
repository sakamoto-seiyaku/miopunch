# MIOPUNCH-59 不规则 NAT 基准与失败验收

2026-09-30，按 [59 修订范围](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ef55c03d-841d-4849-96e2-9556ac00da40)收束真实实验台、冻结单候选比较及安全早失败。用户已确认困难 NAT 的每轮尝试可以正常未命中，不要求 100% 或每种拨号角色命中。本文是实验结果，不设运营商成功率或新默认策略。

正式迁移以[旧行为对齐](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md)为准。62 的旧 NatFeature、完整场景候选和评分缓存，以及63完整 mode0/1/3执行保持不变。59 的四样本、固定小步长与一目标角色只属于历史实验分支；完整旧 random1000/listen256由[61](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d655377c-15a0-47e6-9c9e-4308e4560d96)交付，正式入口全模式/反馈由[60](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/506be324-bc43-444d-a4e1-44624e53e3dc)交付。本票完成不代表旧随机策略或整体困难 NAT 目标已经完成。

## 冻结测量

[完整证据](../evidence/mio59-nat4-irregular/README.md)保留每个基准/自动 trial、seed、实际候选、两端日志、内核规则/conntrack、WAN pcap 和成本。基础与自动交错 AB/BA，同组每角色固定身份和相同 allocator seed，均为3秒路径窗口。正常 selector不读取 fixture标签或 seed，候选来自实际S与线程 RNG，保留实际抽样，但不宣称候选序列可由 fixture seed逐位复现。

| 条件 | 基础原连接业务 | 单候选自动原连接业务 | 解释 |
| --- | ---: | ---: | --- |
| 局部16端口池不规则×稳定 | 0/64 | 4/64 | 自动两拨号角色各2/32 |
| 局部16端口池不规则×规律+1 | 0/64 | 1/64 | 自动 a=1/32，b=0/32；b无需有命中 |
| 全池不规则×稳定 | 0/8 | 0/8 | 完整有界未命中 |
| 全池不规则×规律+1 | 0/8 | 0/8 | 完整有界未命中 |
| 双局部不规则 | 0/8 | 0/8 | 自动8项为零 peer探测回退，不是随机命中尝试 |
| 双全池不规则 | 0/8 | 0/8 | 同上；不等同旧HH推荐/多模式执行 |

核心256，加全池32、双不规则32，共320个比较 trial。独立调参64项另计：padding0=0/32、padding4=1/32；不足以证明 padding4最优或存在统计优势。没有删 miss、挑 seed、加样求命中或把补充检查并入旧分母。局部池 allocator在 base..base+15无放回 shuffle；全池 allocator在1024..65534 shuffle。实验候选分布是观测跨度≤15时 min/max±4（调参另一臂±0），否则只抽1..65535一个端口，和旧1000随机 iteration的1024..65534不等价。

所有原五个核心业务成功及一个调参成功均通过原 TLSpeer、同一 stable Connection ID、原 IPv4物理 socket、实际选定 IP path和 commit确认。业务内容由原 QUIC连接交付，MQTT只承载受限握手/控制，未作业务回退。对普通未命中核验原控制后续往返仍可用，不释放业务。

## 实验台证据与成本

每个新 `(peer IP, destination port)` 的第一包被 NFQUEUE暂留，独立网关 allocator安装精确 SNAT规则后交由真实内核转发；conntrack反向精确 tuple提供 APDF。不是用户态改包/回放模拟。先以同物理 sourceport、同IP换目标port和错误sourceport/合法reverse tuple证明映射/过滤，随后清 conntrack、reset allocator和计数，避免预检替产品打开路径。NAT4标签与真实四个 S样本的特征分别记录，标签不喂 selector。

历史384项（320比较+64调参）只有 WAN抓包，不能声称当时已经保存 LAN pcap。每项 WAN均为完整SLL2记录，captured=received、caplen=original、零kernel drops；结合当时 conntrack证明原端口来源。当前17代表检查及19安全检查补齐两个物理端点 LAN和 WAN抓包、不同目的IP，以及 deliberate wrong-IP/port 被网关丢弃且未到达 LAN、合法逆向包到达 LAN的证据。新 LAN/WAN逐prime匹配原42027/43027；确定性正例还核验原端口 QUIC双向出入。这是兼容性/安全补充，不把新LAN证据倒填为历史采集。

成本按每个真实 WAN ingress包计一次，不按1ms时间窗口合并。包含应用prime、其他peer QUIC数据报、payload bytes、初始/有序/周期 QAD请求与响应、保护控制帧/字节、MQTT包、socket数、fixture CPU和耗时。CPU是整个受控trial子进程用量（含网关/allocator/启动/清理），不是单独产品CPU。UDP计量为payload成本，不含IP/UDP链路头；不能把一次随机draw当作全部网络成本。

核心中位数：每端1应用prime（总2），失败典型peer UDP总5；自动 QAD包108/110对基础46，fixture CPU约1590/1592ms对1522/1521ms。成功路径用时中位数分别894ms和1345ms。完整逐trial及分组值见各组 `verification.json` / `timing-and-costs.json`，保留一个网关队列造成的真实有符号 WAN顺序反转；本机提交屏障先于保护信令，并不保证 WAN转发顺序。成功只由后续 QUIC验证确认。

## 当前版本补充与早失败

17代表检查采用历史每条件/角色的第1个固定fixture seed：局部×稳定、局部×+1、全池×稳定、双局部，双拨号、基础/自动，共16项；另1项确定性稳定×+1证明当前原Connection业务。16随机代表全部正常miss或历史双不规则回退；没有为获得新随机业务反复取样。19安全检查覆盖非成员、无/错秘密、已知撤销、TLS认证后错误预期身份、网络失效、2秒样本过期、迟到控制、取消、旧round、旧prime回执、plan后网络generation变化、plan后撤销、IP变化Unknown、非法范围claim、预算耗尽、blocked WAN、错误候选和映射失准。

已建立、仍允许受限控制的失败，使用故障前已开启的原保护控制流做 PING/PONG，确认原 stable ID和终端stream receipt后退出。该实验验收流只在显式 lab开关启用，不携业务。已知身份撤销会正确安全关闭原控制：plan后撤销为精确 `ConnectionLost(LocallyClosed)`、零peer包；不能将其称为撤销后仍可使用控制。其他已获准连接不受无效UDP决策误关：真实无IP Endpoint的 `register_udp_traversal`精确返回 `IPv4 traversal disabled`，同一已准入TCP连接仍收发授权业务且 stable ID不变。

精确准入错误、两端退出码、expected stage/reason、当前策略执行、实际S、随机window、保护round/提交屏障、原物理source、candidate/实际映射、抓包drain/records和全部case inventory均由独立 checker复核。非预期 `Error:`、基础设施异常、缺LAN/WAN、截断/丢包或没有实际selection/prime不能算允许miss。旧 checker曾可误接受这些坏证据；双轴评审发现后修复，并以变异证据重验拒绝。

保留两项无效当前样本并只重跑受影响检查：预算样本先触发freshness超时、未验证预算；确定性业务正例WAN有6kernel drops。前者同case/seed、隔离其他工作后验证真正预算拒绝；后者只将capture buffer改为8192KiB、snapshot4096，caplen=original仍强制检查，原二进制和seed不变。唯一retry map明确引用两份原无效记录和补测记录，无效样本不算NAT正常miss，不改变历史分母。初次扩展安全运行中错误身份/撤销错误契约与尚未处理的网络变化通知也完整保留、排除canonical；网络fault最终等待真实generation失效后验证停止发包。

## 版本与验证

历史基线源于提交 `2be611dbf23710c8aa23685c7d648f745ebfd396`及当时59 WIP；测量版本按保存的独立二进制/脚本SHA识别。核心、补充、调参的binary不同，各自记录，不归为当前HEAD试验。保存的两源码及patch有独立hash；实验后收紧跨度15/提取helper在所有冻结实际输入上保持观测window和行为一致，独立检查384项已确认，不伪称历史binary来自当前三方合并源码。

接续基线 `bc90f8741dccbd062a94cc3e8abdae1c30d6296b`；从旧base、保存WIP和新HEAD三方恢复，只解决probe wait重叠，完整保留62/63 legacy分支。当前二进制先编译再复制冻结，lab只执行副本；cargo test不会覆盖它。当前源码、三套历史和三套接续runtime、旧patch/原基线和重跑原因见[证据索引](../evidence/mio59-nat4-irregular/README.md)。

最终真实专用MQTT broker完整workspace suite：**220 passed，3既有ignored，0 failed**，无MQTT skips；check --locked --workspace --tests、fmt、脚本语法和diff whitespace通过。首次并行suite的已有 wildcard MQTT observer收到同broker另一测试topic，失败日志保留；同broker最小串行复现两测试通过，最终完整suite采用 `--test-threads=1`通过，未改无关产品实现。双轴 GPT-6.1 Sol high独立核验core和artifact，最终报告链接见证据索引。
