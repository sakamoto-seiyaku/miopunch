# 正式 NAT 连接入口验证

2026-10-01，MIOPUNCH-60。依据[旧 NAT 行为迁移裁决](2026-09-30-legacy-nat-behavior-and-acceptance-alignment.md)及父规格14。正式节点现在持有跨连接的 `NatConnector`：身份、当前成员和准入秘密证明通过后，先安全交换实际适用的 LAN/IPv6/IPv4 direct 地址，再由实际 Endpoint 的有序 S/QAD 观测选择一次完整旧 mode/index。正常入口不读取 NAT 标签、预填 mode/step 或策略环境变量；`udp_nat_probe` 默认和 `auto` 都进入此接口，原56–59单候选比较切片须显式选择 `baseline`/`single-candidate`。

`NativeNetwork::open_with_udp_resources` 接收本节点已经配置的观察资源和维护上限，节点内的 connector 在连接及失败之间保留。`upgrade_udp` 返回原 Connection 的数据会话；有界未命中返回原受限 ControlSession，不补建 TLS/Endpoint。direct 成功不推荐、不训练打洞分数。后续尝试是在最多32个配置新轮次中重新观测、重新 Analyze；正常默认一轮，额外维护由调用方配置。每次推荐只减一次1，没有额外失败扣分。成功反馈必须先经同一实际 winner socket/selected path、原TLS peer/stableID、当前授权业务及其实际 echo，之后才一次+2/cap10。客户端也确认其胜者租约，但只有接受端持有的推荐记录记分。

完整六scene、有方向原feature缓存、PublicNetwork接受端优先、first-max同分次序、所有30组参数及角色交换直接复用62；0/1/3与2/4分别进入63/61的同一个真实executor。端口/跨度/PublicNetwork不另进原feature key；外层真实双方身份、网络、UDP协议、秘密/授权代际隔离仍在。daemon2h/10min机会清理保持原语义。缺样本有candidate/assisted仍是默认TTL、无延时的5秒基础计划，双方无目标disabled；已知HH双不规律保留0/1/3，未退化成零包。正式快照从实际primary IPv4绑定和当前本地接口取得最多64个assisted地址，不把QAD/configured/portmapped公网地址冒充本地地址；发送者先assisted后mapped，保留邻接compact和原始分类输入。

节点额度原子准入完整计划的保守包数 envelope；早赢或取消也不退回这份保留，因此并行连接/重试不能重复借用同一额度。不够时明确拒绝，不缩1000/256。发送延时、绑定/发送、控制交换、winner/path等待、commit及业务都守护实际身份、授权、物理网络epoch和实时UDP许可。物理epoch仍与周期QAD报告代际分开；周期报告不会终止合法38秒计划，真实换网/重新绑定会终止并释放。所有成功仍是原Connection及实际物理socket；无需池的mode继续使用primary，允许的随机池仅在同Endpoint内部。

## 实际结果与分母

[完整证据](../evidence/mio60-formal-nat/README.md)冻结旧十对profile、双方拨号方向、配对身份、fixture seed和随机顺序，各对照臂20次，共40次。baseline和正式旧入口交错；运行期产品 thread RNG 未挑种子、未冻结为可逐位重放，也没有追加样本追命中。NAT1=EIM/EIF、NAT2=EIM/ADF、NAT3=EIM/APDF；规律4由实际四个S端口跨度判定，不读label；不规律4使用1024..65534完整池，未换成16端口窗口。

| 固定臂 | 原Connection业务 | 完整有界未命中 | 业务路径 |
| --- | ---: | ---: | --- |
| baseline | 10/20 | 10/20 | 冻结单候选路径 |
| 正式入口generation1/2历史版本 | 14/20 | 6/20 | 9 punching，5 direct |

旧core01–04正式入口两方向均有实际QUIC与payload正例。core05–10保留正常未命中，包括core10的0/2；这不是全场景100%，也不是运营商/设备总体成功率。40个profile/执行gate通过，与24次比较臂业务成功是不同分母。详细每core、每方向、版本、时间及所有失败见 `verification/{summary,strict,costs}.json`。

另有三个预先声明的 WAN 阻断功能链：HH不规律24轮、HH规律9轮、单Hard不规律21轮，共54次新观测/Analyze/完整执行，union覆盖全部30行为。每个都按真实绝对预算停止，1000迭代与256绑定尝试没有缩减，失败不重复扣分；原受限控制仍在同stableID上实际PING/PONG。它们增加0个NAT概率样本。FD观测峰值分别13/13、269/13、269/13；每轮绑定、raw注册和临时socket释放与实际pcap/提交账本核对，功能链未通过保留旧池来扩大下一轮。

每一新canonical trial均由namespace内的 `wait4` 测量实际node child的user/system CPU和walltime。`ru_maxrss`标为child fork/exec全生命周期；另采样exec后的VmHWM和FD峰值，采样峰值不宣称绝对最大。Python wrapper、Docker客户端、broker和allocator CPU不混入node CPU。初始独立core04功能预检曾整读debug binary做hash，RSS可能继承launcher高水位；原记录保留并明确污染，stream hash和exec后采样在canonical冻结前已修正。它未加入概率分母，未借补测覆盖旧61的38个CPU缺失记录。

所有canonical WAN与两侧LAN完整捕获、完整drain、0 kernel drop。独立checker核对多目的映射、不同source port/同IP换port/不同IP、EIF/ADF/APDF真正过滤、TTL<8在对侧conntrack前丢弃、实际range/重复/随机/回复及source映射、原TLS/stableID、pathID/localSocketID/端口和业务。实际QAD/peer QUIC/UDP payload成本及已记录的MQTT adapter snapshot保留；部分正式日志没有逻辑MQTT字节总数，未从密文pcap或broker活动反推。CPU和全部未命中不省略。

## 版本与失败保全

generation1冻结SHA `b489f1db345599b2afc063d47a4d09a7954a2642ae62f592d7a2025e03085dbe`，实际执行前8个canonical cells。第9个core03-a-baseline在节点启动前，EIF产生的conntrack让gateway-local独立预检的50000 source被MASQUERADE改成31552；原失败预检与9行runner保留。仅对gateway-local public UDP加raw OUTPUT NOTRACK，product FORWARD不走此规则；同seed独立profile预检通过后，只续该未启动的固定cell。generation2冻结SHA `c64b06d99db8841143963874d474cb67d291b48642d845f6dcbd16c003ff1fb2`运行剩余32个比较和54轮功能链。

generation2同时加了业务IO开始前的确定性已禁UDP检查；原连续guard保留。新增公开测试前后均PASS，此为polling-order防御加固，不声称复现过泄漏。节点binary因此不同，`runtime-by-cell.json`诚实记录8+32，未将40个都称最终binary结果。初始EIF/ADF preflight的另一个错误是contacted-IP setter在terminal NFQUEUE之后；counter证明被跳过，移到queue之前后同seed独立验证通过，发生在任何比较样本前。

最终Spec审查发现generation1/2的正式快照未附本地assisted，因此上述40次和54轮只能证明其实际历史版本，不能声称包含assisted-first或最终binary的成功率。公开无S回归实际RED为有本地UDP socket却Disabled；generation3从实际绑定/接口取得assisted修正，真实无S公开回归GREEN证明mode0/6默认TTL、无延时及原stableID授权业务。真实S公开回归同时核对本地assisted仍保留，含其地址与反射地址相同的情况。

`verification/assisted-functional-plan.json`预先声明恰好两个窄kernel功能补测，完整SHA `6d9cd414ff6aeb3f43501d3de1a9be7b48f02222fe28d4fb09c5e3d243fd9bc9`，继承既有固定身份与只读keys；它冻结49个实际输入及generation3 binary SHA `2e6901d6d037ea44841ae8ea5d5da3950952f03b9df337cc36efa34a914a7861`。有S/无S的blocked补测都exit0且完整有界miss，实际pcap分别证明sender先LAN assisted后mapped、无S只有本地assisted/defaultTTL/no-delay/5秒，原control实际PING/PONG保留。独立`nat_connection_assisted_verify.py`核对capture/原TLS/stableID/实际source/候选顺序/预算/CPU及FD上限；补测增加0个自然样本，未重跑既有40/54。此前14/20不能当generation3成功率。

checker在HH不规律功能链中误把初始range与一个非顺序explicit候选的交集当额外repeat。保留RED版本及修正依据，按真实首burst分段修正，未改runtime/重跑链。16个独立artifact破坏均被拒绝，含mode/score/source/socket/TLS/path、CPU scope/version、缺/truncated/drop capture、早停、失去原control、缩listener和改seed。

generation3最终真实专用MQTT broker串行workspace suite **241 passed / 0 failed / 4 existing ignored**，实际exit0，没有broker缺省跳过，完整命令与日志为`verification/full-suite-final-generation3-command.json`及`cargo-test-final-generation3.log`。第四ignore是61单独声明的CPU补充入口，未混入本轮。generation2的240项原日志仍保留。浏览器WASM check PASS。公开formal回归包括真实S及无S实际assisted与原业务、反馈跨连接、direct先于selector、低额度和实时禁UDP、业务前禁用，以及仅IPv6直连的原身份/业务。已有实际wrong peer/no secret/revocation/old round/foreign Connection/network/cancel/TTL/partial bind/winner disposal回归在最终套件内通过。验证是Linux kernel/Docker及loopback；WASM编译不证明浏览器UDP，未宣称Android实机或真实router分布。
