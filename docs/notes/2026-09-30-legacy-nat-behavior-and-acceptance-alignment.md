# 旧版 NAT 行为、mode 选择与验收对齐

日期：2026-09-30。性质：用户明确裁决的迁移契约及源码/验收对照；实施状态以 Plane 为准。这轮只规划，不修改运行时代码，不恢复暂停中的 MIOPUNCH-59。

## 裁决与范围

用户明确要求「这些规律和细节保持旧版实现」「测试也参考旧版怎么验收测试」，并补充「如何选择这些 mode 也要保持旧版逻辑」。因此迁移须保留旧实际特征规则、完整候选 mode/index、顺序、评分、角色及执行参数，不能只保留 mode 名称和大致方向。困难 NAT 仍是概率尝试，允许正常未命中，不要求 100%、每种拨号角色出现命中，也不挑种子补成功。

本裁决补充并覆盖 2026-09-29 [策略范围](2026-09-29-nat34-strategy-scope.md)和[自动选择记录](2026-09-29-nat-auto-selection-followup.md)中“旧常量不成为新预算”“另开随机 listener 禁止”“按单个预测端口替代旧范围”的较窄迁移假说。适配必须保留身份、网络准入秘密、成员用途及资源禁用契约，保持原认证 QUIC **Connection**。为了保留旧实际 256 个随机 listener，允许明确实施同一 Endpoint 内部的多 UDP socket 接入；不再要求所有 mode 永远只使用原来唯一的物理 IPv4 socket。新增 socket 只有成为原 Connection 的实际已验证数据路径才算成功；独立 Endpoint、另拨连接或简单转发到主 socket 不算等效。此范围补充记录到父规格评论和 61；没有在旧证据上改写“同 socket 成功”。

旧 Go/OpenSpec 仍不自动定义新基线全套工作流。本次只复用与 NAT 行为相关的事实和验收方法，不恢复 KCP、Brutal、旧全项目测试门槛或旧身份协议。ADR 0005 的控制/业务隔离与当前分阶段准入保持有效；ADR 0008 的创世恢复材料不属于本次讨论。

审查基点：HEAD `2be611dbf23710c8aa23685c7d648f745ebfd396`（58 已完成）。56–58 的 Done 只证明各自提交过的有限切片，不证明旧 mode0/1/3 表、range、TTL、评分与所有回退已移植。59 的源码、脚本、证据未提交且暂停；安全负例和完整验证未完成，不标 Done。

下文旧源码路径均相对 `archive/legacy-go-openspec-2026-09-28/`；Rust 路径相对仓库根。以当前归档实际代码为准，FRP 仅用于辨别来源，不能把 FRP keep-tunnel 重试策略当成本项目已使用规则。

## 特征与完整 selector

| 旧实际来源 | 要保留的行为 | 当前 Rust 偏差 | Rust 接口适配与验收 |
| --- | --- | --- | --- |
| `nat/discovery.go:33–52,160–184` | 单一发现 socket 顺序访问配置 STUN；支持 OTHER/CHANGED 地址再取样，响应等待 3 秒；输出原始样本顺序 | QAD 目前固定四目的采样 | 用实际 Endpoint UDP socket 形成有序映射快照；QAD 作为地址观察 adapter，不改变分类规则；不要把真实 STUN 的附加目的采样语义误写成强制恰好四个样本 |
| `nat/classify.go:42–103` | 至少两个地址；IP/port 变化形成 NoChange/IPChanged/PortChanged/BothChanged；NoChange=Easy，其余=Hard；任一观测 IP 属于 localIPs 则 PublicNetwork | `src/nat_mapping.rs:105–158` 强制四个不同目的、有序时间、IPv4、两秒有效；IP 变化直接 Unknown，无 PublicNetwork | 分开“有效观测输入”和“旧 NatFeature 分类”；至少两个有效多目的样本，不在分类器额外限定四个或同 IP；两秒时限不能让旧 3/5/10 秒延时执行自相矛盾；网络/socket 代际变化使快照失效 |
| `nat/classify.go:97–101` | **仅 BehaviorPortChanged** 计算 `max(port)-min(port)`；跨度 1..5 即 RegularPortsChange，无顺序/等差/方向要求 | Rust 严格等差 step ±1/±2；小跨度非等差/反转判 Irregular，step2 四样本跨度6反而判规律 | 保留粗特征，不用有符号 next-port 预测替代；[41000,41002,41001] 是规律，四个 +2 样本跨度6是不规律；IPChanged/BothChanged 不赋规律；采样不是 APDF 检测 |
| `internal/punchdecision/nathole_analysis.go:183–219` | 场景生成完整有序 mode/index 列表，见下表 | `choose_single_sequential`、`choose_regular_pair`、`choose_irregular_pair` 互斥选一个分支；双不规律零包 | 决策 module 接收双方旧 NatFeature/角色方向，返回明确 mode/index 与双方计划；一轮只推荐一个行为，但跨轮保留所有候选；双不规律不能零包代替旧 0/1/3 |
| 同文件 `155–164,186–194` | 默认初分0；双不规律初分1；mode0 PublicNetwork 优先当 receiver，c.PublicNetwork 分支先于 v | Rust 无分数、PublicNetwork 优先 | 同一输入完整列表/初分作差分验收；双方公网时仍遵守 c 优先；不是硬删 sender 候选 |
| 同文件 `238–250,222–234` | 推荐第一个最大分（同分保持插入次序），被推荐项减1；成功项加2、上限10；推荐和反馈更新 LastUpdateTime | Rust 无学习与完整列表 | 保留初分、tie、减分、+2/cap10；代码没有 -10 下限，注释“between -10 and10”不是实际规则；不自行钳位。差分序列覆盖至少两遍候选和反馈后重排 |
| 同文件 `275–304` | mode1 Hard sender；mode2 Hard receiver；mode4 Regular sender，按 c 特征交换双方计划 | Rust 固定第一次发包/屏障且仅少数模式 | 正反角色、both-public、HH 双不规律回退均按实际条件交换；mode4 在 HH 双规律回退也存在，不能只允许一规律一不规律 |
| `decision.go:843–876,98–105`；`daemon_memory.go:10–81` | analysis body 按 visitor/c 的首 IP、NatType、Behavior、Regular 形成有方向 key；不含 port/span/PublicNetwork；另有 peer scope 和 UDP/TCP scope。daemon 保留2h、10min机会清理；AnalyzeOnce24h短命实例不同 | 当前无完整 Analyzer；早期表述简写成 IP pair | 新 key 外层绑定本地/远端真实节点身份、网络、准入秘密/授权代际与协议，内层保留旧有方向 feature key 的语义；同一作用域端口变化不自动丢学习，特征/IP改变分记录。跨网络/peer/撤销隔离；闲置大于2h清理，恰好2h保留，活跃推荐/反馈延长 |

完整插入表（每个 mode 内 index 均从0升序；`×N` 指该 mode 的行为数）：

| 场景 | 完整 mode/index 插入顺序 | 初分 |
| --- | --- | --- |
| Easy + Easy | 0×12 | 0；PublicNetwork receiver 对应行为为1 |
| 一个 Hard 且规律 | 1×6 → 2×3 → 0×12 | 0，mode0 同公网规则 |
| 一个 Hard 不规律 | 2×3 → 1×6 → 0×12 | 0，mode0 同公网规则 |
| 两个 Hard 都规律 | 3×6 → 4×3 | 0 |
| 两个 Hard 一个规律 | 4×3 | 0 |
| 两个 Hard 均不规律 | 0×12 → 1×6 → 3×6 | 1 |

推荐不是“所有 mode 在一次连接中无条件跑完”。每次 Analyze 推荐一个 mode/index 并减分，下一次 Analyze 才重新推荐；保留旧这一联动，禁止凭表增加无限内层循环。失败不另有 ReportFailure 扣分；推荐时已经扣过。成功反馈的**数值算法**保持旧版，触发门槛适配原 QUIC Connection 的路径/授权业务确认。旧 `internal/pocv1/punch/runtime.go:135–141` 在 traversal winner 后记分，早于当前要求的 QUIC 业务确认，不能让 raw 命中冒充身份认证。

PublicNetwork优先只是初分偏好：c公网时mode0索引2/3/5/7/10/11为1，初选0/2；否则v公网时0/1/4/6/8/9为1，初选0/0；双方公网按c分支。双不规律default分支不调用公网偏好helper。已存在的Analyzer记录保留原scene列表，当前特征仅用于role交换；报告不匹配mode/index仍刷新LastUpdateTime，未知key忽略。Rust不能直接使用选择最后同分项的`max_by_key`替代旧first-max。

旧`decision.go:702`分类可使用选定CN/global STUN view，但UDP AnalysisKey使用原始MappedAddrs；端口/跨度/PublicNetwork不在key内。迁移保留该可观察缓存复用，避免执行candidate去重反过来改变分类/最后端口。MD5只是旧缓存索引，不用作安全身份；新外层作用域与结构化字段避免跨身份碰撞混学习，不改变同作用域的推荐/保留规则。

## mode/index 行为表

来源 `internal/punchdecision/nathole_analysis.go:40–123`。S=sender，R=receiver；TTL=0 表示使用 socket 默认，不是线上 TTL=0。

| mode | index 与双方行为（交换前 c / v） |
| --- | --- |
| 0 | 0=S delay200 / R TTL7；1=S / R TTL7；2=R TTL7 / S delay200；3=R TTL7 / S；4=S / R TTL4；5=R TTL4 / S；6=S / R；7=R / S；8=S delay5000 / R；9=S delay10000 / R；10=R / S delay5000；11=R / S delay10000 |
| 1 | 0=S / R TTL7 range10；1=S delay2000 / R TTL7 range10；2=S / R TTL4 range10；3=S delay2000 / R TTL4 range10；4=S / R range10；5=S delay2000 / R range10 |
| 2 | 0/1/2=S random1000 delay3000 / R listen256 TTL7/4/default |
| 3 | 0/1/2=S range10 / R range10 TTL7/4/default；3/4/5=R range10 TTL7/4/default / S range10 |
| 4 | 0/1/2=S random1000 delay3000 / R listen256 range2 TTL7/4/default |

mode4 注释写2000，实际为3000，保持3000。旧 UDP 的1000/256不可由59的1/0、局部16端口池或61此前“按预算自行选择”的假说取代。节点预算是外层准入/取消约束；正常旧计划必须能完整容纳其默认行为。低维护额度可明确拒绝/中止这轮旧计划并报告“预算不足”，不得静默把缩水计划标成完整旧 mode。

## 目标、调度、TTL 与 socket 所有权

| 旧实际来源 | 旧实际行为 | 当前偏差/保持旧语义的接法 | 验收 |
| --- | --- | --- | --- |
| `decision.go:767–789` | last端口p，跨度d；From=max(p-d-5,p-maxNumber,1)，To=min(p+d+5,p+maxNumber,65535)，闭区间；maxNumber≤0无范围 | mode1/3 目前只取 next-port；改用对端最后样本和旧跨度公式。range10最多21端口，range2最多5端口；旧不是“10个端口” | 普通/端口1/65535边界、跨度0/1/5/>5、ascending/descending/nonmonotonic；在 WAN 核对范围与实际发包序列 |
| `decision.go:709–735` | 总预算=max双方delay+5000ms；任一listen>0再+30000ms；每端ReadTimeout=总预算-自身delay | 当前两秒证据期限、3秒auto路径等待与原计划冲突 | mode0延时0/200/5000/10000、mode1 0/2000、mode2/4总38秒；听取从开始即开启，延时不是暂停接收 |
| `punching.go:347–395` | sender先Assisted后Candidate；receiver有range时不另发显式Candidate，无range时发Candidate；邻接compact，不全局去重 | 当前单地址和显式prime屏障不等效 | 记录实际candidate/assisted输入和角色计划；LAN优先、range receiver跳过explicit，sender仍保留；不靠手填NAT标签 |
| `punching.go:448–626,629–654,700–866` | 先所有recv loop，延时期间可赢；延时后一次立即burst；普通目标每200ms重复，range仅第一轮一次，random仅第一轮启动一次；结束取消并join | 当前每端1prime且任意固定server先后；改为receive-first与旧scheduler | 延时期间真实UDP命中、发包前取消、首burst、200ms重复、不重启range/random；测试外部行为，配pcap时间与本机单调日志 |
| `punching.go:934–967,970–1015` | range按IP→range→端口递增，每次2ms；random在1024..65534均匀抽未使用端口，单次最多10次查重，失败跳过该iteration；1000次iteration，每端口对每IP发，每次15ms；used属于一次random调用 | 当前样本窄池±4/全池1..65535仅抽一次；不得搬局部池默认 | 固定RNG轨迹仅用于功能差分，不挑命中；核对边界排除65535、1024以下、查重作用域、次数/间隔、取消；随机试验保存全部种子/候选/未命中 |
| `punching.go:591–619` | receiver才回复request，3个response默认TTL，间隔50ms；sender只接受response；胜者是实际收包local socket和source tuple | 当前无raw命中定位 | 使用当前安全控制轮次注册的受保护探测/回执；保持角色/响应burst与实际local/remote绑定；raw只给候选，业务等原QUIC path验证 |
| `punching.go:466–502,735–747,784–809,838–859`；scheduler test171–294 | listen256=**主socket+最多256额外udp4随机绑定socket**；每个都发送explicit/range并接收；首个有效winner保留，其他关闭。bind失败warn继续，因此256是尝试次数，不保证成功数 | 不能用单socket发256包代替256独立本地端口。61负责同Endpoint socket pool和胜者接入；一次pool的端口/listener不自动补齐到“成功绑定256” | 真实内核至少2个额外socket证明不同local port/mapping、非primary胜者、往返及业务始终从winner物理socket出入；成功绑定数、失败数、上限、fd/task清理、取消/超时与主socket保留可核验 |
| `internal/udpowner/udpowner.go:319–356,383–393` | 实际MakeHole经demux.SendTo，IPv4且ttl>0时临时SetTTL；失败仍按默认发；成功设置后恢复原值，取原失败回64；恢复错误未上报；demux发送锁不覆盖全部QUIC业务 | `punching.go:256–323`更丰富的writer降级事件**不是本链实际调用**。Rust须保留每包TTL7/4/default与best-effort fallback，同时使用逐包ancillary/独占发送适配，避免影响并发QUIC | WAN/LAN两侧抓包核对低TTL探测、默认TTL回执、同时QUIC业务不受污染；设TTL不支持时明确报告降级并按默认发；不宣称旧代码已证明共享QUIC全局TTL隔离 |
| `decision.go:622–685` | 样本不足但assisted/STUN候选存在：mode0、默认TTL、无delay、5秒best effort；双方无目标则disabled | 当前Unknown与HH双不规律混合零包fallback | 区分“缺/坏样本”和“已知两Hard不规律”；前者保留确定性candidate/assisted基础尝试，后者保留完整0/1/3推荐；不从无候选启动随机扫描 |

旧细节中的实现缺陷须显式识别：score下限仅注释、mode4延时注释不符、TTL未保护全部QUIC写入、send忽略ctx、STUN接收未完整匹配transaction/source。迁移保留上述实际算法/参数；取消/身份/连接门控按新已定契约，在适配发送入口复查，不能复制漏洞或把修复伪装成旧已完成行为。邻接compact和random查重次数等影响候选序列的实际行为保留，不擅自全局去重改变分布/数量。

## 61 的具体多 socket 接缝

这是待实现并验证的接法，不是“现有底座已兼容”的结论。当前 `vendor/iroh/src/socket/transports/ip.rs:185–235` 接收只报remote，`transports.rs:819–893` 的 Ip FourTuple 只有local **IP**，`:974–1008` 发送选首个匹配IP的sender；同IP不同本地端口的256个socket无法被精确区分。单纯添加IpTransport或将额外UDP包转发到主socket会丢失local-port/path归属，不能验收。

61 在同Endpoint transport内部增加短寿命socket lease与稳定local-socket ID。收包携带该ID及真实bound address；raw注册绑定(localSocketID, authenticated peer, network/authorization epoch, Connection ID, round)。有效winner返回(localSocketID, actual remote source)，随后原Connection开显式QUIC验证路径。Ip path必须保留localSocketID直到发送选择，**PATH_CHALLENGE、PATH_RESPONSE、重传和业务**均定向到同一实体socket，不按IP选首个。不要另造Endpoint或TLS会话。

Noq目前本地路径通过local IP传递。可在iroh内部仿现有custom/mapped-address注册机制增加私有local-address token→socket-ID映射，保持公开远端TransportAddr::Ip；或者在fork中贯穿local socket绑定字段。前者不能把synthetic token发送为公网源IP，底层sendmsg使用lease的真实绑定socket/IP。无论选哪种，须追通 `Socket::to_local_transport_addr`、recv metadata→mapped tuple、remote-state路径注册、send路由及path移除，避免两个本地端口被合并成一个path。`vendor/iroh/src/socket.rs:2041–2047`网络变化过滤会把Ip path的local字段与真实接口IP比较；私有token必须先映射到lease真实IP再作此判断，不能把仍有效的socket误删或把失效lease保留。此接缝选择由61实证收敛，不扩大产品对外连接接口。

pool从旧计划创建并开启recv；probe TTL逐包隔离。未命中/取消/身份撤销时释放全部临时lease与raw注册；成功时只保留胜者lease供原Connection的数据路径，失败的QUIC path不能保留业务资格。保留原主socket和既有获准路径；多轮不能用未释放pool偷偷加监听数量。记录原stable ID、原TLS peer、path ID、localSocketID、本地端口与网关映射，并在两端pcap证明QUIC及业务从胜者端口发回。若Noq实际不能接受该路径，给出最小复现、失败层和具体修正范围，不凭随机零命中宣布不可能。

## 外层优先级与重试

旧通用 `connectivity/attempt.go:283–364` 实际顺序是tcp6 direct→tcp4 direct→tcp4 punching→udp6 direct→udp4 direct→udp4 punching，受policy筛选；POC v1 `internal/pocv1/punch/config.go:187–196` 却把auto归为udp_only并拒绝tcp_only。因此不存在一个可从注释照抄的“唯一旧全产品优先级”。本次UDP切片保留UDP直接候选先于UDP punching、sender Assisted优先、PublicNetwork receiver初分及mode候选顺序；TCP、relay与质量选路由39/43/47当前契约处理，不恢复旧KCP或覆盖质量优先裁决。

`connectivity/attempt.go:529` 每个Attempt调用MakeHole一次。下一轮重新Analyze才选新mode/index；daemon学习跨会话，SID fallback则保守隔离。FRP keep-tunnel的8次/小时、90秒不是本项目NAT engine重试默认。60须证明有界新轮次重新采样/推荐、失败扣分不重复、确认成功后+2、取消与撤销停发；不得增加无界“跑遍所有mode直到命中”的循环。

## 旧验收与 Rust 逐用例映射

历史“通过”分层：profile证明实验台分类；punching验收证明实际候选/调度/握手；业务验证另证连接用途。下面的旧通过数不能直接成为Rust成功率。

| 旧验收来源与实际范围 | Rust验收适配 | 归属 |
| --- | --- | --- |
| `nat/classify_test.go:5`；`internal/punchdecision/nathole_analysis_test.go:11–28`：分类示例和一个Easy/规律Hard组合仅检查mode/role属于supported、index非负；并未断言完整顺序/评分；非真实NAT | 真实S快照→完整六场景列表/初分/每个index/交换角色，差分测试补旧未覆盖的IP变化、非等差小跨度、PublicNetwork、tie、无限下行score、cap10和隔离/清理；不当网络成功证据 | 62 |
| `internal/punchdecision/decision_test.go:57–231`：assisted fallback、selected view不改变assisted、Compact clone不破坏原分类 | 真实同LAN/公网/缺样本情况下生成实际候选计划，既有输入不被执行去重改写；第一项为LAN/direct可用，早失败为无任何候选disabled | 62/60 |
| `internal/punching/punching_scheduler_test.go:43–115,117–169,171–294`：真实本机UDP延时前命中、发包前取消、临时listener胜者保持开启；不是NAT/TTL实网验收 | 同Endpoint原Connection复现receive-before-delay、取消无发包、非primary胜者持续往返/业务与loser清理；额外补TTL共享业务隔离、range/random时间/数量验证 | 63/61 |
| `lab/guest/lib/profiles.sh`、`cases/core-01.sh`至`core-10.sh:2–4` 与 `lib/validate.sh:33–40,117–141`：NAT1(EIM/EIF)、NAT2(EIM/ADF)、NAT3(EIM/APDF)、NAT4(APDM/APDF)十对核心profile；验证是简化两目的EIM/APDM，不完整RFC4787 ADM区分 | 先独立证明按目的映射、同IP换端口、不同IP、wrong IP/port过滤，包含全十对、LAN/公网与拨号角色交换；真实kernel/VM/NFQUEUE+conntrack+计数+两侧pcap；实验台标签只用于报告不喂selector | 59/60及62/63/61的首检查 |
| `docs/reports/2026-03-18-xtcp-fulltest.md:6,9–10,34–43`：core01–04要求连接成功，core05–10允许诊断失败，core10 QUIC/KCP均FAIL(punching) | 易场景要求真实QUIC/业务正例；困难场景逐trial允许失败，验收按自动策略覆盖、资源/身份和诊断证据；不能把core10 profile PASS写成成功穿透；不要求每一困难组合或每一拨号角色命中 | 59/61/60 |
| `docs/notes/mainline-network-test-findings.md:31–33,66–79`；`lab/guest/bin/mlab-mnt01-run:114–134`：MNT01 64/64是gate通过，部分TCP默认diag-fail-allowed；F-001记录NAT2/3普通sender先到、receiver低TTL错过窗口的失败与scheduler修正 | NAT2/NAT3 receive-first、lowTTL/普通TTL包序都要实际capture；统计gate pass和connection success分别给分子/分母，任何允许失败项不得写成连通率100% | 63/60；TCP证据交43 |
| `openspec/changes/archive/2026-06-03-restore-pocv1-archived-udp-semantics/tasks.md:68–76`：same-host smoke选direct_ipv6，未跑真实punching_ipv4；六月Android/WSL demo只direct_ipv4 | 把singlehost UDP/IPv6 smoke当接口回归；NAT正例必须真实不同网段、有网关映射过滤证据、实际punching路径及业务，不用smoke替代；Android额外平台验证不设为所有NAT修正前置 | 60，平台53 |

每次实施先做有判据的首项真实检查，再做优先失败：错身份/无秘密/撤销不披露候选和探测；旧round/跨Connection回执不建立路径；取消、迟到控制、禁UDP、预算耗尽停止并释放；网络变化使lease/快照失效；TTL降级不污染QUIC；无命中给有界结果，MQTT不承载业务。

旧十对的具体matrix：core01=NAT1×NAT1，02=NAT1×NAT2，03=NAT1×NAT3，04=NAT3×NAT3；05=NAT2×规律NAT4，06=规律NAT4×NAT1，07=不规律NAT4×NAT3，08=双规律NAT4，09=不规律×规律NAT4，10=双不规律NAT4。旧XTCP默认repeat=1并各跑QUIC/KCP（`mlab-xtcp-fulltest:18,32,45–56`），未发现冻结RNG/NAT种子或统计成功率阈值；Rust的交错重复/固定seed保全是当前验收补充，不伪称旧已有统计门槛。

旧fixture还有必须交代的细节：`profiles.sh:58–64`在WAN入站raw PREROUTING、conntrack之前丢TTL<8，模拟低TTL探测在公网中途死亡。Rust必须保留这个fixture条件或用真实多跳替代，并以两侧capture证明探测创建本地NAT状态而未提前到达对侧，不能在单L2 WAN直接跑TTL4/7后怪算法。`:165–183`的“规律”使用40000–45000等SNAT范围，“不规律”用random-fully；标签本身不证明观测端口跨度1..5。`:186–201`的TCP irregular fixture刻意给40000/45000样本并DNAT至P+100，仅证明该fixture，不是任意TCP困难NAT。`topology.sh:78–94`的loss是每包DROP概率，不是打洞成功概率。

`mlab-mnt01-run:286–326`要求成功有ping=ok和selected path、允许失败有stage/reason/stop condition；采用这一证据纪律。`mlab-xtcp-fulltest:91–105`把NAT4 runner error也计允许失败的历史漏洞不继承：基础设施报错、缺capture、样本不合法和策略未执行应让验收失败，只有完整计划真实执行后的有界未命中才是允许的trial结果。

随机验收在实验前冻结源码、二进制、拓扑、seed列表、样本数和候选分布，基础与完整旧计划交错重复，保留每个失败和全部分母。记录candidate/port/socket/probe/response/QAD/QUIC/control/CPU/耗时；同条件正例必须原TLS peer、原stable ID、实际winner socket path及授权业务。全端口random与局部16端口allocator分别报告；后者不是旧随机算法的默认候选窗口，也不是运营商分布。允许0/N的真实报告，不因此删旧模式、调seed或将零包回退称旧行为。

## 修正切片与调度

已发布的切片：[62旧特征与完整selector](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/7985934c-21a1-41a7-a717-6f097b1dada8)串通实际S→旧NatFeature→完整selector/score→受保护计划交换。至少一个已有低成本adapter可执行的推荐计划须继续真实受限UDP探测、原Connection路径确认或有界失败并记录结果；仅打印推荐状态不验收。未恢复的参数须明确partial_execution/execution_unavailable，不能把部分执行标成完整旧mode。[63旧范围与调度](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/f943cc89-199e-447f-b353-a0bead63892a)接62计划恢复mode0/1/3全部range/TTL/delay/receive-first/repeat与同Connection业务。[59](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/ef55c03d-841d-4849-96e2-9556ac00da40)保留为冻结单候选基准与真实随机lab/负例验收，不声称该1/0切片等效旧mode2/4。[61](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/d655377c-15a0-47e6-9c9e-4308e4560d96)恢复完整random1000/listen256与winner→原Connection路径接缝。[60](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/506be324-bc43-444d-a4e1-44624e53e3dc)负责正式入口的所有模式、跨轮反馈、直接候选优先和完整验收。

建议顺序62→63→59剩余验收→61→60；59仍暂停，须主线程另行调度才恢复。既有59→61→60原生边不倒置、不产生循环；61和60另加62/63阻塞，60保留58阻塞。各票独立implement→同配置review→commit→handoff，模型GPT-6.1 Sol high；负载高等待，不换模型。本文不是第二套开放任务台账，实时状态以Plane为准。

本轮两位GPT-6.1 Sol high只读审查分别核查旧selector/执行链与历史验收，再复审文档和已发布票的完整回读。结论为规划/静态契约通过；62真实执行闭环和61网络变化token映射的复审补正已发布回读。未运行新随机实验或运行时测试，256listener、Noq本地socket路径与TTL并发隔离尚待各实施票证明。
