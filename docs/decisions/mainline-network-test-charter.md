# 主线网络测试纲领

日期：2026-04-25

本文定义 `miopunch` 主线网络测试的目标、分层、第三场景拓扑与验收口径。它用于把现有 NAT lab、POC e2e 与后续主线组网测试重新统合，但不替代后续 OpenSpec change，也不展开脚本实现细节。

## 背景

当前测试体系已经有两类强能力：

- NAT lab 能在单个 QEMU VM 内用 `netns/veth/iptables/tc` 构造可复现的 NAT、IPv4/IPv6、STUN、portmap、TCP/UDP 和弱网条件。
- POC e2e 容器实验台能在 Docker/systemd 节点中跑真实 `miopunch` daemon、LocalAPI、产品 CLI 与自建 MQTT broker。

缺口在于：主线节点尚未在一个可控多 NAT 小网络中完成“空白启动、入网、bootstrap、邻居维护、直连打洞、扰动恢复”的完整验证。后续主线稳定性不应只依赖单元测试、双节点 lab case 或 Docker 平网 e2e。

## 核心决策

- `lab` 是网络夹具，不是主线被测对象。NAT、STUN、MQTT、pcap、conntrack、netem 等由 lab 提供；被测节点必须是真实主线 `miopunch` daemon 和产品命令。
- 主线网络测试分三大场景推进：连接性矩阵、控制面 e2e、12 节点 NAT 综合网络。
- 场景 1 和场景 2 是进入场景 3 的前置门槛；二节点连接能力和产品控制面闭环都通过后，才推进 12 节点 NAT 综合网络。
- GUI、shell 透传等上层功能不进入本文主范围；它们依赖主线网络稳定后另行测试。
- 第三场景必须从空白节点开始建网；测试 harness 不得预先灌入 membership、peers、governance、decls 或邻居状态。
- 主线网络测试的信令和控制面入口统一为测试环境自部署 MQTT broker；`coord` 属于历史 lab/POC 遗留路径，不进入主线验收，也不作为 fallback。
- 公网 MQTT broker 不进入主线 required gate；自部署 broker 是测试夹具的一部分，用于保证可控、可复现、可抓日志和可注入 outage/recovery。
- MQTT 只作为控制面、信令和 mailbox 入口；测试不得引入中心化数据面 relay。
- 不是所有 peer pair 都必须直连成功。不可达 pair 的失败也可以通过，但必须有明确阶段、原因和证据。
- 测试实现和运行过程中发现的项目代码问题，不应在测试重构中顺手修复；统一记录到 `docs/notes/mainline-network-test-findings.md`，后续按问题清单单独修复。

## 三大测试场景

### 1. 主线连接性矩阵

定位：替换旧 lab peer 作为主线连接性验收的被测对象。

目标：

- 用真实主线节点验证基础连接路径，而不是只验证 lab runner。
- 覆盖 `UDP/TCP`、`IPv4/IPv6`、STUN、portmap、NAT1-4、TCP spraying、loss/netem 和诊断失败。
- 将 UDP 与 TCP 作为不同画像轴分别验收，避免用单一 NAT label 同时代表两种协议行为。
- 对每个成功 case 要求 payload exchange 证据；对每个失败 case 要求 explainable failure。
- 对所有组合要求“执行、分类、诊断、符合预期”，不要求所有组合都能连通。

覆盖义务：

- UDP 使用 `udp-nat1`、`udp-nat2`、`udp-nat3`、`udp-nat4-regular`、`udp-nat4-irregular` 五类画像。
- UDP 矩阵按无向 pair class 覆盖，即 `5×5` with replacement，共 15 类组合。
- TCP 使用 `tcp6-direct`、`tcp4-direct`、`tcp4-portmap-direct`、`tcp-easy-stable`、`tcp-hard-regular`、`tcp-hard-irregular`、`tcp-blocked-unknown` 七类画像。
- TCP 矩阵按有向 pair class 覆盖，即发起方画像到目标方画像的 `7×7`，共 49 类组合。
- `auto` 不进入全矩阵，只作为路径优先级和 fallback 专项；默认验证顺序为 `tcp6 → tcp4 → udp6 → udp4`。
- `IPv6`、portmap、loss/netem、blocked、STUN unavailable 不与 UDP/TCP 主矩阵无限笛卡尔积；它们作为专项轴覆盖。
- `QUIC/KCP/BBR/Brutal` 不乘入主矩阵；它们作为 transport smoke 或专项覆盖，避免组合爆炸。

画像含义：

- `udp-nat1`：UDP endpoint-independent mapping 和 endpoint-independent filtering，代表最易达 UDP 画像。
- `udp-nat2`：UDP endpoint-independent mapping 和 address-dependent filtering。
- `udp-nat3`：UDP endpoint-independent mapping 和 address/port-dependent filtering。
- `udp-nat4-regular`：UDP address/port-dependent mapping，端口变化相对规律。
- `udp-nat4-irregular`：UDP address/port-dependent mapping，端口变化不规律或随机化。
- `tcp6-direct`：IPv6 TCP 直连优先路径。
- `tcp4-direct`：IPv4 TCP 直连优先路径。
- `tcp4-portmap-direct`：借助 portmap 获得的 IPv4 TCP direct 候选。
- `tcp-easy-stable`：TCP 外部端口稳定，可作为 easy/stable punching 或 direct 候选。
- `tcp-hard-regular`：TCP 候选可推导但存在 hard NAT 约束。
- `tcp-hard-irregular`：TCP 候选难以稳定推导，需要 spraying 或更强诊断。
- `tcp-blocked-unknown`：TCP 被阻断或画像不可判定，用于负例和诊断失败。

夹具与信令口径：

- 场景 1 必须通过自部署 MQTT broker 完成信令；每个 case 的证据链必须包含 MQTT signaling evidence。
- 测试夹具允许为两个主线节点提供最小 identity、peer、MQTT endpoint、STUN endpoint 和本机网络画像。
- 测试夹具不得预置 NAT 判定结果、候选路径结论、邻居状态、成功缓存或 payload 结果。
- TCP 有向矩阵必须保留 `dialer → target` 角色；反向不是同一个 case，不能合并。

结果分类：

- `success-required`：必须连通，且必须有 payload exchange 证据。
- `success-preferred`：期望连通；若因 TCP hard 条件失败，必须给出完整诊断。
- `diag-fail-allowed`：不要求连通，但必须证明尝试路径正确、预算有界、失败原因可解释。
- `fail-required`：策略、配置或阻断条件下必须失败，且失败原因必须稳定。

TCP 粗粒度期望规则：

- direct/easy 到 direct/easy 的 TCP 组合默认归入 `success-required`。
- portmap-direct 组合在 helper 正常时默认归入 `success-required`，helper 被禁用或不可用时归入 fallback 专项。
- hard-regular 或 hard-irregular 参与的组合默认不承诺稳定成功，按风险归入 `success-preferred` 或 `diag-fail-allowed`。
- blocked/unknown 参与且阻断条件为测试目标时，应归入 `fail-required` 或 `diag-fail-allowed`。

TCP hard NAT 口径：

- TCP 打洞比 UDP 更复杂，hard/irregular 组合接受不稳定因素。
- 允许以可解释失败通过部分 TCP hard case，但不允许静默跳过、无限重试或无诊断失败。
- 每个 TCP hard case 至少应证明候选发现、尝试路径、停止条件和最终原因。
- TCP hard/irregular case 应通过 bounded repeat/retry 观察稳定性；通过依据是预算内结果、失败原因一致性和诊断完整性，而不是单次偶然结果。

证据与稳定性口径：

- 每个 case 至少应证明 MQTT 信令连接、候选发现、attempt path、最终 selected/failed path。
- 成功 case 必须证明 payload 真实穿过数据面；失败 case 必须证明 failure stage、reason 和停止条件。
- MQTT broker pcap/log 不应出现数据面 payload。

边界：

- NAT lab 自检继续保留，但它只证明夹具可信，不代表主线验收通过。
- 本场景不测试 invite/join/governance，也不要求形成多节点 overlay。
- 本场景不承担多节点邻居选择、bootstrap 收敛或扰动恢复验收。

### 2. 主线控制面 e2e

定位：验证产品命令、daemon、LocalAPI、状态机和 MQTT 控制面闭环。

目标：

- 在 Docker/systemd 节点中跑完整主线 `miopunch`。
- 默认至少使用 6 个节点覆盖 issuer/admin、多个正常 member、wrong/competing actor 和 lifecycle/recovery 节点。
- 覆盖 `up`、`invite`、`approve`、`join`、`ping`、`sh` smoke、revoke、restart persistence、broker outage/recovery、invite expiry、max uses、wrong actor、report/redaction 等控制面语义。
- 网络可保持简单，重点是主线命令和状态落盘正确。
- 把空白启动、建网、入网和 join/approve 过程本身作为被测对象，而不是仅作为后续测试准备。

覆盖义务：

- 空白启动必须验收：节点独立生成 identity，不能预置 net、membership、peers、governance、decls 或邻居状态。
- `up` 必须验收：daemon、LocalAPI、状态目录和本机配置可重复启动且状态一致。
- `invite/approve/join` 必须验收：合法流程成功落盘，失败流程返回稳定 `reason_code` 和可读 facts。
- 多成员一致性必须验收：所有合法 joined member 应看到同一 `net_id`、governance head、decls 和成员视图。
- 失败合约必须验收：malformed invite、missing approve、wrong approver、invite expiry、uses exhausted、broker down 等都应有稳定报告。
- 生命周期必须验收：restart 后状态恢复，revoke 后权限变化正确，broker 恢复后仍能继续建网或执行控制面动作。
- 报告和隐私必须验收：human/json report 可读，敏感字段 redaction 生效，MQTT 不承载数据面 payload。

最小通过基线：

- required gate 至少应完成一次正向闭环：blank `up` → `invite` → `approve/join` → multi-member consistency → `ping` → restart → ping after restart。
- broker outage/recovery 至少应完成一次恢复闭环：broker down 触发预期失败，broker restore 后 `invite/join` 或 `ping` 重新成功。
- `sh` 只作为产品链路 smoke，用于证明控制面授权、任务创建和最小数据面 payload 能贯通；本场景不展开 shell 功能矩阵。

节点角色：

- issuer/admin：负责创建 invite、approve、revoke 和基础管理动作。
- member-1 / member-2 / member-3：验证多成员 join、一致性、横向访问和 revoke 不误伤。
- wrong/competing actor：验证 wrong approver、非法 actor、并发抢占和失败报告。
- lifecycle/recovery node：验证 restart、broker outage/recovery、late join、rejoin 和状态恢复。

状态、幂等性与并发口径：

- 每个控制面动作都应有 before/after 状态快照；验收对象包括状态是否发生、是否只发生预期变化、是否可恢复。
- 状态快照至少覆盖 identity、net、governance head、decls、state、report 和关键命令输出。
- 幂等性必须纳入验收：重复 `up`、重复 daemon install、重复 approve/join、重复 report、revoked access 重试都必须稳定且可解释。
- 并发/竞态必须纳入验收：approve 与 join 并发、多个 join 抢一个 invite、uses exhausted、expired pending、broker 短暂不可用都必须给出稳定结果。
- broker outage case 必须同时证明失败可解释，以及 broker 恢复后系统仍可继续建网或执行控制面动作。
- `reason_code`、facts、human/json report 和 redaction 视为稳定产品接口。
- admin、member、revoked、wrong actor 的权限边界必须可验证；revoke 不能误伤未 revoked 成员。

边界：

- 本场景不要求 NAT1-4 矩阵覆盖。
- 本场景不证明复杂 NAT 下的邻居选择和数据面路径选择。
- 本场景不替代第三场景的 12 节点 overlay、logN 邻居维护和扰动恢复验收。

### 3. 12 节点 NAT 综合网络

定位：验证真实主线多节点网络在可控 NAT 宇宙中的入网、邻居维护、打洞和恢复行为。

目标：

- 在单个 VM 内运行自建 MQTT/STUN/probe/pcap 设施。
- 启动约 12 个真实 `miopunch` daemon 节点，每个节点位于不同可控 NAT 或链路画像后面。
- 验证 bootstrap 推荐、reachability bucket、`k=max(2,ceil(ln(n)))` 邻居维护、active edge 打洞、不可达 pair 的可解释失败，以及最终扰动恢复。

实现形态：

- 第三场景采用“Docker/systemd 产品节点 + lab NAT 网络夹具”的组合形态。
- Docker 只负责把每个被测节点变成独立 Linux/systemd/`miopunch` 主机；节点必须通过真实产品命令、system daemon 和 LocalAPI 运行。
- lab 负责提供 NAT domain、WAN、MQTT、STUN、probe、pcap、conntrack、netem 和扰动注入；lab 不得替主线选择 bootstrap、维护邻居、判定 reachability、预置拓扑或注入成功结果。
- harness 应通过容器 network namespace + veth 把 Docker/systemd 节点接入 lab-controlled NAT domain，避免 Docker 默认 bridge 平网绕过 NAT 验收。
- 第三场景缺口能力必须落到主线产品行为和稳定观测接口中；不得以 lab-only helper 代替 `bootstrap_more`、presence、reachability hints、logN active neighbor 或 recovery evidence。

Gate 分层：

- MNT-03 实现必须小步递增：先跑 2 节点真实闭环，再扩到 3、4、6，最后扩到 12 节点完整网络；每一步都必须在 QEMU lab VM 内使用真实 Docker/systemd `miopunch` 节点和真实 NAT 夹具验证。
- `mnt03-smoke`：覆盖 2 节点 substrate 与 3 节点 bootstrap 闭环，验证空白启动、真实建网/入网、基础拓扑快照和至少一条 successful payload edge。
- `mnt03-selftest`：在 smoke 基础上扩到 4/6 节点，验证 reachability bucket、portmap、presence/online、`bootstrap_more`、logN active neighbor、hard 节点承接和 active edge payload / explainable failure。
- `mnt03-fulltest`：扩到完整 12 节点，在稳定拓扑通过后加入 loss、offline/rejoin、revoke、IPv6 阻断、portmap 阻断、broker outage/recovery，并要求 pcap/conntrack/tc 证据。
- 三层 gate 均应输出可机读 summary，并把 artifacts 拉回 `lab/_artifacts/`。

## 第三场景节点画像

第三场景默认使用 12 个节点。12 不是协议常量，而是当前测试规模的合理点：当 `n≈12` 时，目标邻居数 `k≈3`，足以观察非平凡邻居选择、桶内随机/轮换、hard 节点承接和 admin 去中心化。

| 节点 | 角色 | 网络画像 | 主要验证点 |
| --- | --- | --- | --- |
| `n01` | primary admin / issuer | NAT1，IPv4+IPv6，稳定在线 | 发起建网、approve、bootstrap responder、基础可达锚点 |
| `n02` | backup admin | NAT2，IPv4 only，稳定在线 | admin 冗余、非 NAT1 admin 作为候选 |
| `n03` | easy member | NAT1，IPv4+IPv6 | easy peer 横向连接、IPv6 direct |
| `n04` | easy member | NAT2，IPv4 only | IPv4 easy 路径、UDP/TCP direct 或 punching |
| `n05` | portmap member | NAT3 + NAT-PMP，IPv4 only | portmap direct 候选和推荐 |
| `n06` | dual-stack fallback member | NAT3，IPv6 present，可配置阻断 | IPv6 first，IPv6 fail 后回落 IPv4 |
| `n07` | hard regular member | NAT4-regular，IPv4 only | hard1 被 easy 节点承接 |
| `n08` | hard regular member | NAT4-regular，IPv4 only | hard1 对称样本，避免单例偶然性 |
| `n09` | lossy hard member | NAT4-regular + mild loss/netem | 弱网下保活、重连、邻居替换 |
| `n10` | hard irregular member | NAT4-irregular，IPv4 only | hard2 / TCP spraying 候选 |
| `n11` | hard irregular / unknown member | NAT4-irregular，STUN 或 portmap 受限 | unknown/hard2 逐级放宽和失败解释 |
| `n12` | actor / lifecycle node | 初始 NAT2 或 NAT3，可切换 offline/rejoin | wrong actor、revoke、offline、rejoin |

逻辑分桶：

- `direct/easy` 池：`n01..n06`
- `hard` 池：`n07..n11`
- `actor/lifecycle`：`n12`

期望性质：

- hard 节点不要求彼此互通；它们应优先被 easy/direct 节点承接。
- admin 节点应稳定在线，但不应成为所有 active neighbor 的唯一 hub。
- `n12` 不计入稳定拓扑通过率统计，避免负例和生命周期动作污染主体网络指标。

## 第三场景验收流程

每一步都必须定义进入条件、动作、通过条件和证据。扰动只在稳定拓扑通过后添加。

### 1. 空白启动

动作：

- 启动 12 个节点的真实 `miopunch up`。
- 节点 state 初始为空，允许 fixture 提供本机网络画像和自建 MQTT endpoint，但不得预置 peers、membership、decls 或邻居。

通过条件：

- 每个节点 LocalAPI ready。
- 每个节点生成独立 identity。
- 合法节点没有预置 peer 关系。

证据：

- `status` / LocalAPI readiness。
- daemon journal。
- state snapshot。

### 2. 建网与入网

动作：

- `n01` 创建 invite。
- `n02..n11` 通过真实 `approve/join` 流程入网。
- `n12` 用于 wrong actor、late join 或后续 lifecycle 测试。

通过条件：

- 合法节点共享同一 `net_id`。
- governance head、decls、identity、net state 按产品流程落盘。
- join/approve 失败场景必须有明确 `reason_code` 和 facts。

证据：

- invite/approve/join report。
- `/var/lib/miopunch` 下的 net、identity、governance、decls snapshot。
- broker logs。

### 3. Bootstrap 推荐

动作：

- 新节点根据 membership/bootstrap 信息尝试推荐 peer。
- 推荐失败时请求更多候选。

通过条件：

- 推荐 peer 去重。
- 候选优先从 direct/easy bucket 选取，失败后逐级放宽到 hard/unknown。
- 尝试边界可解释，不无限重试。

证据：

- bootstrap attempts。
- selected bucket。
- candidate failure reasons。

### 4. 邻居形成

动作：

- 所有合法在线节点进入稳定期。
- 节点按 `k=max(2,ceil(ln(n)))` 维持 active neighbors；在 12 节点规模下目标约为 `k=3`。

通过条件：

- 稳定合法节点的 active neighbor 数接近目标值。
- hard 节点至少有 easy/direct active neighbor。
- admin 不成为唯一 hub。
- 邻居选择体现桶内随机/轮换，不固定到单一节点。

证据：

- neighbor list。
- degree distribution。
- before/after topology snapshot。

### 5. Active 边打洞验证

动作：

- 对 active neighbor 边执行 `ping` 或最小 payload exchange。

通过条件：

- 成功边必须有 `attempt_path`、`data_proto` 和 payload evidence。
- 失败边只允许出现在预期 hard/unknown 条件下，并且必须可解释。
- broker pcap 不应显示数据面 payload relay。

证据：

- task report。
- connectivity/data-plane event log。
- WAN pcap。
- NAT iptables、conntrack、tc snapshot。

### 6. 扰动与恢复

动作：

- 在稳定拓扑通过后，分轮加入扰动：loss、节点下线、revoke、rejoin、IPv6 阻断、portmap 阻断。

通过条件：

- 邻居替换或重连发生。
- 非故障节点不整体崩溃。
- revoked 节点不能继续访问受保护 peer。
- rejoin 节点能重新收敛或给出可解释失败。

证据：

- 扰动前后 topology diff。
- recovery report。
- failure reason。
- daemon journal 和网络 artifacts。

## 证据与可观测性

主线网络测试不能只看 exit code。每个 case 至少应收集：

- product command stdout/stderr、JSON envelope、task report。
- daemon journal。
- LocalAPI status、peers/tasks snapshot。
- state、net、identity、governance、decls snapshot。
- broker logs。
- WAN pcap。
- NAT 规则、conntrack、tc qdisc 统计。
- cleanup log。

第三场景还需要主线暴露或报告以下观测项：

- bootstrap candidate list、selected bucket、失败原因。
- reachability hints。
- active neighbor list。
- degree distribution。
- selected `attempt_path`、`data_proto`、payload evidence。
- revoke/offline/rejoin 后的拓扑变化。

## 实现缺口与后续 OpenSpec

本文先固定测试目标和验收口径。`mnt-03-mainline-nat-composite-network` change 应把第三场景补成完整可执行验收，而不是只记录 TODO。该 change 需要同时覆盖主线产品能力、稳定观测接口、Docker/systemd + NAT lab 拓扑、runner/gate 和 artifacts。

必须补齐的主线能力：

- `bootstrap_more`：joiner 初始推荐失败后通过控制面请求更多候选；候选去重、分桶、轮次和失败边界必须可观测。
- presence / online：节点周期性传播在线与 state head 摘要，用于 bootstrap、邻居维护和恢复排序。
- reachability hints：节点报告 `v4_hint` / `v6_hint`，只用于排序，不携带 endpoint；分级采用既有产品讨论草案。
- logN active neighbor：入网后按 `k=max(2,ceil(ln(n)))` 维护 active neighbors，12 节点规模目标约为 3。
- recovery evidence：offline、rejoin、revoke、IPv6/portmap/broker 扰动后，主线必须报告拓扑变化、重连/换邻居结果和失败原因。

稳定观测接口：

- LocalAPI / CLI JSON 应稳定暴露 bootstrap candidate list、selected bucket、失败原因、reachability hints、active neighbor list、degree distribution、selected `attempt_path`、`data_proto`、payload evidence 和 recovery evidence。
- daemon journal 和 pcap 是辅助证据，不得作为唯一稳定接口。
- state snapshots 可用于事后诊断，但测试不得通过直接修改 state 文件驱动主线行为。

OpenSpec change 口径：

- 当前 change 使用 `mnt-03-mainline-nat-composite-network`。
- OpenSpec 必须新增第三场景能力 spec，并修改 LocalAPI、control-plane mailbox/RPC、decl/reachability 和 mesh/neighbor 相关产品 spec。
- 进入代码、测试或 lab runtime 变更时按代码影响级别执行验证；docs/OpenSpec-only 更新只需 OpenSpec 校验。
