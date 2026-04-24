# Door 2：TCP 打洞纲领（POC 后续方向）

## 文档状态

- 本文档用于收束“TCP 打洞”作为 **POC 收口后的后续方向** 的目标、边界、关键基线与风险评估。
- 本文档不展开到“实现细节/具体参数/最终 API 形态”；实现前必须基于本文档拆分为一个或多个 OpenSpec change。
- 本文档记录“已选定 / 待定 / 已知风险”，避免口径散落在聊天与临时笔记中。

## 背景与动机

- 当前 `miopunch` 的直连主路径是：`UDP punching` + `QUIC/KCP over UDP` 数据面。
- 真实网络中常见问题是：`UDP` 被封、被 QoS 限速、或不稳定；而 `TCP`（尤其 80/443 类端口）更可能可达。
- Door 2 的目标是在不改变既有 POC 语义边界的前提下，引入一条与 UDP 并列的 **TCP 直连/打洞路径**，并保持可观测性与可解释性。

## 范围与非目标

### 目标

- 把 `TCP candidate / attempt / session` 融入现有“候选采集 → 交换 → attempt 编排 → 数据面握手”的流程中，形成与 UDP 路径并列的补充能力。
- 明确 TCP 打洞的成功边界与风险，把“能不能成/为什么不成”做成可解释输出，而不是黑盒重试。

### 非目标（明确推后）

- 不把 `QUIC/KCP` 强行承载到 `TCP` 上（两者是 UDP 协议）；TCP 路径应形成独立的数据面实现。
- 不引入 TURN/数据面 relay（后续方向“更远期数据面与组网展望”再讨论）。
- 不引入 `udp2raw` 式伪装、用户态协议栈绕过内核、`VPP` 等能力（属于更远期方向）。

## 仓库事实基线（当前实现形态）

- `connectivity`：
  - `Gather`：UDP-only（绑定 UDP4/UDP6、收集 IPv6 candidates、portmap candidates、可选 STUN 观测）。
  - `Attempt`：UDP-only，固定顺序为 `IPv6 direct → IPv4 direct → IPv4 punching`。
- `internal/wire`：
  - `NatHoleVisitor/NatHoleClient` 交换字段语义偏 UDP：`DirectAddrs/MappedAddrs/AssistedAddrs`。
- `dataplane`：
  - 当前实现是 `KCP/QUIC over UDP`，前提是已经获得“可用 UDP path”（`*net.UDPConn + *net.UDPAddr`）。
- STUN（当前实现约束）：
  - 当前 STUN 观测与内置默认 STUN 列表均以 **UDP STUN** 为基线。
  - `connectivity` 侧显式不支持 `tcp://` STUN scheme（会直接报错）；因此本方向若要做 “TCP candidates / TCP NAT 类型评估”，需要补齐 **STUN over TCP**（或替代的 TCP 映射观测机制）。

结论：将 TCP 打洞引入后，会不可避免地触及 `Gather / Exchange(wire) / Attempt / Dataplane` 这四段边界。

## 外部事实基线（参考 gonc：本轮调研结论）

> 这些条目作为“工程经验/事实基线”记录，用于校准预期与避免踩坑；不代表本项目必然照搬其实现与参数。

### gonc 的默认尝试顺序（自动模式）

- gonc 在自动 P2P 模式下的协议优先级基线为：
  - `tcp6 → tcp4 → udp6 → udp4`
- 同时允许用户显式限制网络类型（例如只使用 UDP 或只使用 TCP 的运行模式），以匹配不同网络策略与环境约束。

### gonc 的 TCP punching 成功边界（基线）

- 参考 gonc 工程基线：**非同 LAN 时，TCP punching 通常至少需要一端为 `easy` NAT**；否则应视为极低成功率并降低预期/直接跳过。
- 本项目取向（已选定）：不把“至少一端 easy NAT”作为硬 gating；TCP punching 是否进入、以及是否启用 mode2/4 喷射，统一交给 analyzer+budget 决定，并把“成功预期低/跳过原因/预算触发”做成可解释输出。
- 落地口径（已选定）：TCP punching **沿用现有 UDP punching 的 `mode0..4` 语义与选择逻辑**（喷射仅在 `mode2/mode4` 出现）；差异仅在于 TCP 成本更高，因此端口喷射的预算/规模更小，并要求 report 明确解释“为什么进入 `mode2/mode4`/喷射”。

### gonc 的 TCP punching 流程形态（高层）

- 两端同时 `listen + dial`（simultaneous open 思路）以提高成功率。
- 端口复用依赖 OS socket 选项（常见为 `SO_REUSEADDR`；Unix 上常配合 `SO_REUSEPORT`）。
- 避免复用“STUN 使用过的 TCP 源端口”（gonc 使用启发式在 **非同 LAN** 时双方约定 `port + 100`）：
  - 来源：`threatexpert/gonc` `v2.4.0`（commit `3fc8f9d8`）的 `easyp2p/p2p.go` 中，非同 LAN 时把本地与对端端口都 `incPort(..., 100)`，并在注释里说明“该端口连接过 STUN，后续可能被 STUN 服务器 FIN/RST 影响其他会话，因此双方约定 +100”。
- 可能存在多条连接同时成功，需要“连接选择/确认”收敛到同一条连接，其它连接关闭。
- 端口喷射（随机源端口/随机目的端口）属于 gonc 的重要策略之一，但成本与网络友好性需要谨慎评估。

#### 端口喷射（Port spraying）到底是什么（直觉解释）

端口喷射可以理解为“用并发把未知端口空间撞出来”，其本质是：

- **喷射目的端口（random destination ports）**：对同一个对端公网 IP，在短时间内并发尝试大量不同的目的端口。
  - 直觉：当你无法确定对端 NAT 在本次会话里最终暴露的是哪一个公网端口时，就只能“猜很多个”。
- **喷射源端口（random source ports）**：本端并发用大量不同的本地源端口去发起连接，从而在 NAT 公网侧创建大量不同的映射端口。
  - 直觉：给对端“更多可撞中的洞口”，提高双方在同一时间窗口内命中彼此外部端口的概率。

它和“生日撞击/生日悖论”的相似点在于：当双方都在一个很大的端口空间里各自制造/尝试很多端口时，命中概率更像与 **尝试次数的乘积**相关，而不是线性增加。

代价也很直接：它会制造大量并发的 TCP 连接尝试，可能触发网络侧的限速/告警，并带来本机资源压力（FD/CPU/端口耗尽风险）。因此本项目需要把它当作“可解释的可选激进策略”，而不是默认黑盒行为。

#### 现阶段取向（端口喷射：非默认，但必须有理由）

- 端口喷射本身不是禁忌；关键是 **必须可解释**，且只在“确实需要”的场景出现。
- 在流程口径上，本方向保持与 UDP punching 一致（`mode0..4` 语义不变；喷射仅在 `mode2/mode4` 出现）：
  - **喷射是 `mode2/mode4` 的一部分**（端口变化不规律的语境），而不是“无理由默认就喷射”。
  - 预算必须明确（并发上限 / 端口数量上限 / 总时间上限 / 早停条件）。
  - 输出必须明确（告诉用户“为什么喷射、喷了多少、是否被限额/早停”）。
- 参数规模需针对 TCP 的成本模型单独收敛（不能照搬 UDP 的 `1000/256`）。

## 融入 miopunch 的影响面（会影响哪些流程）

### 1) Gather：从 UDP-only 扩展到 UDP + TCP

- 增加 `tcp4/tcp6` 的候选采集与可达性观测（通常依赖 STUN over TCP 的映射观测）。
- 明确 TCP NAT 类型/行为的评估方式与输出口径（与 UDP punching 的 NAT/behavior 不混用）。

### 2) Exchange（wire/控制面）：需要交换 TCP 候选与元信息

- 现有 `NatHoleVisitor/NatHoleClient` 的候选字段语义偏 UDP；引入 TCP 需要：
  - 能交换 TCP candidates（至少包括 tcp4/tcp6 的地址/端口）。
  - 能表达 TCP 侧 NAT 类型/难度（用于 attempt 编排与跳过策略）。
  - 需要定义兼容/版本协商策略，避免旧版本节点误读/忽略导致 silent failure。
- 实现取向（已选定）：以最小入侵为原则，在现有 NAT-hole 消息中增加并列字段表达 TCP（例如 `tcp_direct_addrs` / `tcp_mapped_addrs` / TCP STUN view），而不是复用字符串前缀或整体替换结构。
- 兼容与协商（已选定）：在 `PeerHello` 增加 `capabilities`（例如 `tcp_p2p_v0`），在 `p2p_network=tcp_only` 时可对“对端不支持 TCP Door-2”进行 fail-fast，并给出明确错误（而不是黑盒等待超时）。

### 3) Attempt 编排：不再只是 UDP attempt

- 现状 attempt 固定为 UDP-only 顺序；引入 TCP 后需要明确：
  - 策略开关（已选定）：`p2p_network=auto | udp_only | tcp_only`（详见下文“输入与配置”）。
  - TCP direct vs TCP punching 的边界、超时预算、并发策略与可观测性（尤其端口喷射行为的“为什么要这么做/做了多少”需要可解释）。

### 4) Dataplane/加密：TCP 形成并列数据面

- 基线：TCP 路径拿到的是 `net.Conn`（stream）。
- 约束（当前倾向）：**TCP 数据面默认/仅支持 `TLS 1.3 + stream`**（端到端加密后再承载上层 `ping/sh_attach` 等双向 I/O）。
- 安全语义基线（已选定）：
  - 不改变既有 POC 的上层语义与数据形态：控制面仍按既有 E2E 加密/签名语义运行；数据面在 TCP/UDP 两条路径上都保持“加密后传输”。
  - TCP 数据面 TLS 需要做 **身份绑定**（pinning），避免“TLS 只加密不认证”导致的 MITM 口径下降；并且仍保留现有 `hello/secret_key` 等上层治理验真语义。
    - pinning 方案（已选定）：用 `HKDF(secret_key, sid, role)` 派生每会话的证书私钥（建议 `ed25519` seed），生成 self-signed cert；双方基于对端 `role` 计算期望的公钥指纹并 verify（mutual TLS），不额外新增 wire 交换字段。
- 明确：`QUIC/KCP` 不作为 TCP 路径的数据面承载方式；两条路径并列存在。

## 建议的整体流程（v0，端口喷射：非默认）

> 这一节回答“把 TCP 融入现有 UDP4 打洞与整个链路流程后，整体应该怎么走”。它是 **Door 2 的建议方案**，用于后续拆 change；不等价为最终实现。

### 输入与配置（用户可控）

- `stun_servers`：仍沿用现有“字符串列表”的写法（见本文后文“STUN 端点列表约定”）。
- `p2p_network`（拟新增）：`auto | udp_only | tcp_only`
  - `auto`：默认顺序 `tcp6 → tcp4 → udp6 → udp4`（对齐 gonc 的工程基线）。
  - `udp_only`：仅走 UDP（兼容现有 POC 主路径）。
  - `tcp_only`：仅走 TCP（面向 UDP 被封场景）。
- punching 进入规则（建议与 UDP4 对齐）：对某个 network，先 direct；direct 失败后若控制面给出 punching 行为（mode0..4 + budget），则进入 punching；否则跳过并解释原因。
- TCP punching 启用门槛（已选定）：不额外引入“至少一端 EasyNAT”或“禁止 hard+hard”这样的硬条件；完全复用 analyzer 的 `mode0..4` 选择逻辑与 role 口径（喷射仅在 `mode2/mode4` 出现），并依赖预算/护栏收敛。
- 当 `p2p_network=tcp_only` 且 TCP STUN 不可用：仍执行 TCP direct/portmap best-effort；但必须跳过 TCP punching（避免“无依据/不可解释”的喷射）；若最终无可用路径则返回明确错误并写入 report。
- TCP 端口约定（已选定）：TCP STUN 固定使用本地端口 `P`；TCP 对外 listen/punching 使用 `P+100`（双方约定；来源见下文 gonc 引用）。
  - `P` 的选择（已选定）：
    - 若用户 pin `ListenPort>0`：`P=ListenPort`，并采用 fail-fast（`P` 或 `P+100` 不可用则明确报错）。
    - 若未 pin（`ListenPort=0`）：优先复用 UDP bind 出来的端口号作为 `P`（不同协议允许复用相同端口号）；要求 `P+100 <= 65535` 且可用于 TCP listen/punching。若不满足，再在高端口范围内探测选择一个 `P`（探测失败才回退/报错）。
  - `+100` 的使用层级（已选定）：
    - `tcp_mapped_addrs`（以及 TCP STUN view）记录的是 **STUN 观测的原始映射端口**（即 `P` 的映射），不做 `+100` 改写。
    - coordinator 生成真正用于 dial 的 `TCPCandidateAddrs` / `TcpDetectBehavior.CandidatePorts` 时，对端端口基线按 `incPort(mapped_port, 100)` 偏移；下发到 attempt 的端口范围是 **最终绝对端口**（已包含 `+100`），attempt 不再做二次偏移。
  - TCP portmap（已选定）：映射 `P+100`（对应实际 listen/punching 端口），而不是 `P`（STUN 观测端口）。

### STUN 端点列表约定（不写两遍）

- `host:port`：视为 **dual**（UDP STUN / TCP STUN 都允许尝试；失败可观测并跳过）。
- `udp://host:port`：**UDP-only**（TCP STUN 阶段跳过）。
- `tcp://host:port`：**TCP-only**（UDP STUN 阶段跳过）。
- 内置默认 STUN buckets 写法（已选定）：dual 仍用 `host:port`；UDP-only 必须用 `udp://...`；TCP-only 必须用 `tcp://...`（避免“默认列表让 TCP 白白超时”）。
- 若用户强制 `tcp_only`：仅使用 `{dual + TCP-only}`（跳过 UDP-only）。
- 若用户强制 `udp_only`：仅使用 `{dual + UDP-only}`（跳过 TCP-only）。

实现建议（避免“默认列表全部 TCP 超时”）：

- TCP STUN 采用 **短超时 + 早停**（例如“达到 2 个 OK 样本就停止”）；并把“TCP STUN 不可用”作为可解释事实输出。
- 内置默认 STUN buckets 若已知为 UDP-only，应显式标注为 `udp://...`（否则 dual 会让 TCP STUN 白白超时）。

### TCP STUN 端点证据：获取与沉淀（已选定）

目标：回答“哪些 STUN 端点 **真的** 支持 STUN over TCP”，并把证据变成可复现产物（用于更新内置默认 buckets 的 `udp://` 标注与 dual 口径）。

- 复用原则（已选定）：把 STUN（UDP/TCP）client + endpoint 解析/分类能力整理为可复用模块（作为 `connectivity/` 内部组成）；**生产 Gather 与证据探测必须复用同一套代码**（避免重复实现导致行为漂移）。
- 证据入口（已选定）：提供一个“额外入口”用于探测 TCP/UDP STUN 的可用性（例如 `miopunch-lab stun probe`；名称与 flags 在实现 change 中定稿），入口本身只负责参数解析与输出落盘，核心逻辑来自 `connectivity`。
- 输出形态（已选定）：探测以 **逐端点结果** 输出（建议 JSON），至少包含：`endpoint`、`network(tcp4/tcp6/udp4/udp6)`、`ok`、`rtt_ms`、`mapped_addrs`、`error`。该输出由操作者自行保存/归档，作为“端点可用性证据”。
- 内置列表的分类规则（已选定）：对每个 endpoint 同时探测 UDP 与 TCP（含 `udp4/udp6/tcp4/tcp6`；按实际解析结果 best-effort），并采用 “`okCount>=2` 视为该协议可用” 的门槛：
  - UDP OK 且 TCP OK → 记为 dual：`host:port`
  - UDP OK 且 TCP not OK → 记为 UDP-only：`udp://host:port`
  - TCP OK 且 UDP not OK → 记为 TCP-only：`tcp://host:port`
  - 两者都不 OK → 从内置列表移除或移入 notes（避免污染默认超时）
- 实现要点（已选定）：
  - TCP 是 stream：读取 STUN 响应必须按 STUN header 的 length 读取完整消息（避免半包/粘包造成偶现失败）。
  - TCP STUN client 建议使用 `pion/stun` 的 TCP 连接形态并关闭重传（语义等价于 gonc 的实现取向），同时由上层 `timeout/early-stop` 控制整体预算。

### Gather：一次性采集候选（保持 no-trickle）

原则：继续沿用当前 `connectivity/Gather` 的理念——**限定时间预算，给出快照**，避免后续 trickle 造成状态机复杂化。

1) 绑定本地 socket / listener（拟扩展）：
   - UDP：沿用现状（`udp4Conn/udp6Conn` + `ListenPort` 规则）。
   - TCP：补齐 `tcp4/tcp6` listener（对外 listen/punching 端口使用 `P+100`；TCP STUN 观测固定使用本地端口 `P`）。
2) 采集 direct candidates（拟扩展）：
   - UDP：沿用现状（IPv6 local candidates + IPv4 portmap snapshot）。
   - TCP：采集本机接口地址 + listen port（`P+100`），形成 `tcp6/tcp4` direct candidates。
3) 端口映射（建议复用现有策略，拟扩展到 TCP）：
   - 继续使用 UPnP / NAT-PMP，形成“可直连的公网端口”候选；
   - TCP portmap 是“低噪音替代策略”，优先于端口喷射。
   - 实现形态（已选定）：portmap helper 扩展为可指定协议（UDP/TCP）；Gather 侧分别对 `udp4_port` 与 `tcp_listen_port(P+100)` 做 best-effort portmap，并把结果并入对应 network 的 candidate 列表。
4) STUN 观测（拟拆成两条独立 best-effort）：
   - UDP STUN：沿用现状（用于 UDP punching 的 NAT 观测与 cn/global view arbitration）。
   - TCP STUN：Door 2 新增（当 `p2p_network` 允许 TCP 时执行；best-effort），输出：
     - `tcp_mapped_addrs`（样本 >=2 才视为“可做 NAT 评估”）
     - `tcp_nat_difficulty`（先用粗粒度分类即可：`easy` / `unknown` / `hard`）

产出（拟演进方向）：

- 将候选从“无类型 string 列表”演进为“带 network 的候选集合”，避免 UDP/TCP 候选混在同一字段里：
  - `direct_candidates[{tcp4,tcp6,udp4,udp6}]`
  - `mapped_candidates[{tcp4,tcp6,udp4,udp6}]`
  - `assisted_candidates[{udp4,...}]`（TCP 是否需要 assisted 取决于 punching 方案；v0 可先不引入）

落地顺序（已选定）：

- v0 以 “`tcp4 + tcp6` + portmap（TCP/UDP 并行）” 作为最小落地面，避免先做 tcp4 再补 tcp6 导致 wire/attempt 二次拆改。
- 新增字段/事件命名遵循既有 UDP 命名模式（例如 `direct_addrs/mapped_addrs` 对应 `tcp_direct_addrs/tcp_mapped_addrs`；事件继续沿用 `gather.*`/`attempt.*`/`transport.*` 分层与动词风格）。

### Exchange（wire/控制面）：交换“分协议”的候选与观测

建议目标：让 coordinator 能对 UDP/TCP 分别做“可解释决策”，并把决策产出成 attempt 可执行的指令。

1) 候选交换：
   - UDP：保持兼容现有字段（`PeerDirectAddrs/CandidateAddrs/AssistedAddrs`）。
   - TCP（已选定）：在**现有 wire 消息**中新增并行字段表达 TCP（不引入新消息版本），例如：
     - `PeerTCPDirectAddrs`
     - `TCPCandidateAddrs`（来自 TCP STUN / TCP portmap 的候选）
   - 字段命名风格（已选定）：遵循 UDP 现有的 snake_case 语义映射：
     - request 侧（`NatHoleVisitor/NatHoleClient`）：`tcp_direct_addrs` / `tcp_mapped_addrs` / `tcp_stun_cn` / `tcp_stun_global`
     - response 侧（`NatHoleResp`）：`peer_tcp_direct_addrs` / `tcp_candidate_addrs` / `tcp_selected_view` / `tcp_selected_reason` / `tcp_punching_enabled` / `tcp_punching_error` / `tcp_detect_behavior`
2) STUN view selection：
   - UDP：沿用现有 `cn/global` 观测与仲裁。
   - TCP：若有 `tcp_stun_cn/global` 观测，也允许同样的仲裁；否则 TCP punching 直接标注为不可用，并给出原因。
3) punching enable / behavior：
   - UDP punching：沿用现状（mode0..4 + role/timeout 等）。
   - TCP punching（v0）：基线采用 **simultaneous open**（listen + dial），并在“流程口径”上与 UDP4 punching 对齐（同样由控制面选择 mode0..4）：
     - 行为下发结构（已选定）：UDP 继续使用现有 `DetectBehavior`；TCP 采用并列的 `TcpDetectBehavior`（字段同构，但避免复用 UDP 字段造成语义误解，例如 TCP 下没有 TTL 语义）。
     - 参数下发范围（已选定）：`TcpDetectBehavior` 只下发“attempt 语义必要”的字段（例如 mode/role/candidate ports/random ports/delay/timeout）；护栏参数（`MaxConcurrency/TotalBudget/DialTimeout/SettleWindow`）不进 wire，由 attempt 端本地默认值（或配置）控制，但必须写入 report/events 便于复盘。
     - `mode 0/1/3`：不喷射或仅做范围端口猜测（deterministic）。
     - `mode 2/4`：包含端口喷射（random ports + multi-listen），但必须满足“有理由 + 有预算 + 可解释输出”。
     - TCP 侧默认参数需要更小（成本更高）：例如 `SendRandomPorts=128`、`ListenRandomPorts=32`（同一套语义，但预算更小）。
     - mode 选择方法（建议与 UDP4 对齐）：coordinator 基于双方 `tcp_mapped_addrs` 做 `nat.ClassifyNATFeature`，再复用现有 analyzer 选出 `(mode,index)` 与双方 `Role/Delay/PortsRangeNumber/...`；仅在“每端 >=2 个样本”时才进入可分析路径，否则只能 best-effort 回退（类似 UDP 侧的 `nat analysis unavailable` 语境）。

### Attempt：按优先级编排 direct → punching（复用 UDP4 的“预算/可观测/收敛”）

建议采用“**按网络优先级逐项推进**”的组织方式，深度对齐现有 UDP4 attempt 的心智模型：

- 先确定网络尝试顺序（基线沿用 gonc）：`tcp6 → tcp4 → udp6 → udp4`。
- 对于序列里的每一个 network，始终先做 **Direct**，再（在允许且可行时）进入 **Punching**；失败再进入下一个 network。
  - 这样可以保证：当用户/默认偏好 TCP 时，**TCP punching 会发生在回退到 UDP 之前**，而不是被“UDP direct”插队。

对单个 network 的 attempt 结构（复用 UDP4 的组织方式）：

- Stage A：Direct（含 portmap 直连候选）
  - 该 network 下并发 fanout 尝试多个候选，选出 winner（复用现有 `directHandshakeFanout` 思路）。
- Stage B：Punching（仅当 Stage A 失败后进入）
  - UDP4：沿用既有 punching kernel（mode0..4）。
  - TCP4/TCP6：Door 2 punching（simultaneous-open；同样由控制面选择 mode0..4；其中 mode2/4 包含喷射）：
    1) listener ready（本地 listen port）
    2) dial fanout 到对端 `TCPCandidateAddrs`（可包含小范围端口猜测：mode1/3 等价物）
    3) winner 收敛（已选定）：允许多个连接同时成功；在 TLS+pinning 完成后做一次轻量 election 握手（settle window 内收敛到同一条 winner），其余连接关闭
    - simultaneous-open 实现基线（已选定）：
      - socket 复用选项：Unix 上 `SO_REUSEADDR + SO_REUSEPORT`；Windows 上 `SO_REUSEADDR`；listen 与 dial 均设置。
      - dial 必须绑定到与 listener 相同的本地端口（`LocalAddr.Port=listenPort`），否则不视为 punching 路径（会退化成普通 outbound TCP）。
      - `tcp4/tcp6` listener 策略：当同时允许 v4/v6 时分别监听 `tcp4 0.0.0.0:P+100` 与 `tcp6 [::]:P+100`，不依赖 v4-mapped 的隐式行为差异。

#### 深度对齐：UDP4 的 mode1..4，哪些可复用到 TCP

现有 UDP4 punching（来自 `frp xtcp`）把“不同 NAT 组合”映射成 4 种主要策略（mode 1..4，见 `internal/punching/punching.go:36`、`internal/coordinator/nathole_analysis.go:51`）：

- `mode 1`（Hard + Easy，且端口变化“规律”）：对 Hard 侧做 **范围端口猜测**（`PortsRangeNumber`），不依赖随机喷射。
- `mode 2`（Hard + Easy，且端口变化“不规律”）：依赖 **随机端口喷射**（`PortsRandomNumber=1000`）+ **多端口监听**（`ListenRandomPorts=256`）。
- `mode 3`（Hard + Hard，且双方端口变化“规律”）：双方都做 **范围端口猜测**（`PortsRangeNumber`），不依赖随机喷射。
- `mode 4`（Hard + Hard，且仅一侧规律）：仍然依赖 **随机端口喷射 + 多端口监听**，只是叠加少量范围端口猜测（`PortsRangeNumber=2`）。

> UDP 里“范围端口猜测”的具体范围，是 coordinator 基于最后一个 mapped port 与 `PortsDifference` 计算出来的（见 `internal/coordinator/nathole_controller.go:557` 的 `getRangePorts`）。

对 TCP 的结论（以“默认不喷射，但可解释地启用”为约束）：

- **可复用（推荐 v0 支持）**
  - `mode 1` → TCP 可复用为：**一侧做小范围 dial 猜测**（对端端口范围来自 TCP STUN mapped port 的 range 推断 + budget 限制）。
  - `mode 3` → TCP 可复用为：**双方做小范围 dial 猜测**（同上，仍然是 deterministic range，不做 random）。
  - `mode 0`（补充）：TCP direct / simultaneous-open 的基线策略，等价于“不猜端口，只打已知候选端口集合”。
- **可复用（但必须强约束）**
  - `mode 2 / mode 4`：其核心是 `random ports + multi-listen`（端口喷射）。TCP 上实现代价和噪音更高（大量并发 dial/listen/半连接），因此不能“无理由默认喷射”，但可以在满足以下条件时启用：
    - 明确触发理由：coordinator/analyzer 选择 `mode2/4`（TCP 侧观测落入“端口变化不规律”的语境，对齐 UDP 的 `mode2/4` 定义）。
    - 明确预算：总时长、并发上限、随机端口数量上限、监听端口数量上限；并支持早停。
    - 明确输出：把“触发原因/预算/实际喷射规模/是否被限额或跳过”输出到事件与 report。
    - 字段语义（已选定，与 UDP 对齐）：
      - `SendRandomPorts`：随机 **目的端口**（对同一个对端 IP，尝试多个随机的 destination port）。
      - `ListenRandomPorts`：额外使用多个 **本地端口**（listen + dial 复用同一端口）以制造更多映射端口/洞口，与 `SendRandomPorts` 共同提高命中概率。
  - 参数规模必须针对 TCP 单独收敛：**不能直接照搬 UDP 的 `1000/256`**；需要以真实网络数据为依据逐步调大。
  - 护栏（已选定，必须落地到实现与 report）：
    - `MaxConcurrency`：限制并发 dial/listen/半连接 的数量；初始默认：`64`。
    - `TotalBudget`：限制 mode2/4 的总耗时预算（超时即收敛/回退）；初始默认：`5s`（auto）、`10s`（tcp_only）。
    - `DialTimeout`：每条 dial 的超时上限（避免系统默认超时过长导致“看起来卡死”）；初始默认：`1500ms`（auto）、`2500ms`（tcp_only）。
    - `SettleWindow`：winner election 的 settle window；初始默认：`200ms`。
    - `EarlyStop`：一旦出现 winner 即早停并关闭其余连接/尝试。
    - 可解释输出：report 必须包含触发原因（mode2/4）、预算、实际尝试规模、是否被限额或早停。

为让 `mode 1 / mode 3` 在 TCP 上“像 UDP 一样有意义”，有一个关键前提需要写死：

- **TCP STUN 观测必须固定本地源端口**，否则 `nat.ClassifyNATFeature` 的“端口变化”会被本机临时端口扰动污染，导致把“易 NAT”误判成“Hard/不规律”（`nat/classify.go:42`）。
- 取向（已选定）：TCP STUN 使用固定端口 `P` 进行观测；TCP punching/listen 使用 `P+100`（双方约定），以避免复用“连接过 STUN 的端口”。
  - 来源：参考 `threatexpert/gonc` `v2.4.0`（commit `3fc8f9d8`）在 `easyp2p/p2p.go` 的做法与注释（非同 LAN 时双方约定 `incPort(..., 100)`）。

可复用点（从 UDP4 迁移到 TCP 的“组织方式”）：

- 固定 attempt 顺序与时间预算（避免“看起来卡住了”的黑盒连接尝试）。
- candidate fanout + winner 选择（提升“最快成功”的概率）。
- 事件/日志结构（每个 candidate begin/end，明确 reason：timeout/skip/winner）。
- coordinator 统一给出“可尝试/不可尝试”的 gating 结论，并可解释。

### Dataplane：不改变上层语义，只替换承载

- 若 winner 为 UDP：沿用现有 `QUIC/KCP over UDP` 数据面与上层语义。
- 若 winner 为 TCP：使用 `TLS 1.3 + stream` 数据面；上层 `ping/sh_attach` 等语义保持不变（只是底层承载从 `*net.UDPConn` 变为 `net.Conn`）。

## 风险与可行性评估（能不能成）

### 可行性较高的场景（预期）

- 同 LAN / 同网段：TCP 直连成功率高（不依赖 punching）。
- 非同 LAN 且至少一端 `easy` NAT：按 gonc 工程基线，TCP punching 才更有现实成功预期。
- UDP 受限但 TCP 可达的网络：TCP 路径可能是“从不可用变可用”的关键增益。

### 低预期/高风险场景（需要提前写清）

- 非同 LAN 且双方都不是 `easy` NAT（例如 hard+hard / symm+symm）：成功率预期低。
- 端口喷射策略可能触发：
  - 防火墙/IDS 告警、连接被丢弃、或被策略限速。
  - 本机资源压力（文件描述符/并发连接/CPU）。
- OS 差异：端口复用、socket 选项、同时打开（simultaneous open）行为在不同平台可能存在差异，需要按平台验证与收敛。

## 关键问题清单（已选定/待定）

- 默认 attempt 顺序（已初步选定）：沿用 gonc 的 `tcp6 → tcp4 → udp6 → udp4` 作为默认基线，并允许用户显式限制只使用 UDP 或只使用 TCP。
- TCP punching 启用门槛（已选定）：完全复用 analyzer 的 mode 选择结果，不额外添加“至少一端 EasyNAT”这类硬条件。
- TCP 侧 STUN 输入形态（已选定）：引入 `tcp://` scheme，并支持 `host:port` dual 的写法；内置默认 buckets 必须完成一轮整理与分类，明确哪些端点是 dual / tcp-only / udp-only（写进字符串 scheme）。
- TCP STUN 端点可用性（列表内容待定，流程已选定）：`connectivity/stun_internal.go` 的内置默认列表目前只有“UDP 可用性”证据（2026-04-14 的探测记录）；Door 2 必须补齐 STUN over TCP 可用性的探测证据，然后把端点分类标清楚（TCP-only/UDP-only/dual），避免“默认列表让 TCP 白白超时”。
- wire 演进策略（已选定）：以最小入侵为原则，在现有 NAT-hole 消息中增加并列字段表达 TCP（候选/观测/决策/行为），例如 `tcp_direct_addrs`、`tcp_mapped_addrs`、`tcp_stun_cn/global`、`tcp_selected_view/reason`、`tcp_punching_enabled/error`、`tcp_detect_behavior`，避免复用字符串前缀破坏现有语义。
- portmap（已选定）：portmap helper 扩展为可指定协议（UDP/TCP）；Gather 侧对 `udp4_port` 与 `tcp_listen_port(P+100)` 分别做 best-effort portmap。
- TCP 端口选择（已选定）：若用户 pin `ListenPort>0` 则 `P=ListenPort` 且 `P/P+100` 不可用即 fail-fast；若未 pin（`ListenPort=0`）则优先复用 UDP bind 出的端口号作为 `P`，并要求 `P+100` 可用；不满足时再探测选择 `P`。
- TLS pinning（已选定）：`HKDF(secret_key, sid, role)` 派生每会话证书密钥并 mutual verify，不额外新增 wire 交换字段。
- capabilities（已选定）：在 `PeerHello` 增加 `capabilities`（如 `tcp_p2p_v0`），`tcp_only` 可 fail-fast。
- mode2/4 的跨平台资源保护（初始默认已选定，仍需迭代）：`MaxConcurrency=64`；`TotalBudget=5s(auto)/10s(tcp_only)`；`DialTimeout=1500ms(auto)/2500ms(tcp_only)`；仍需在 Windows/Linux/macOS 的 socket 行为差异下通过真实网络样本与 lab 回归逐步收敛。
- 测试与验证（方向已明确，细节待定）：沿用现有 NAT lab（`lab/guest/cases`）的回归形态与“payload exchanged”判定方式，新增 TCP 侧用例与 expect events；哪些 TCP punching 行为可以被 lab 覆盖、以及哪些必须依赖真实网络样本，仍需在实现后逐步收敛。

## 下一步（建议）

- 基于本文纲领拆分为多个 OpenSpec change（已选定拆分口径）：
  - Change 1：`STUN 模块化 + probe + 更新内置列表`
    - 把当前 STUN（UDP/TCP）相关实现提取/整理为可复用模块（供 `Gather` 与 probe 复用），并让现有代码改为引用该模块。
    - 增加独立 probe 入口（TCP + UDP），跑出“端点可用性证据”并据此把内置 STUN 列表标注为 `udp://` / `tcp://` / dual（`host:port`）。
  - Change 2：`wire 扩展（控制平面携带 TCP 信息）`
    - 在现有 NAT-hole wire 消息上新增并列 TCP 字段（候选/观测/决策/行为）与 `capabilities` 协商；保持 UDP 语义不变、旧端兼容。
  - Change 3：`TCP 直连 + punching（含 simultaneous-open + mode2/4 喷射与参数收敛）`
    - 落地 TCP gather/attempt 的最小闭环（`tcp4+tcp6` listener/candidates、TCP portmap、TCP STUN 观测、direct + punching 编排）。
    - 落地 simultaneous-open、winner 收敛与资源护栏；`mode2/mode4` 允许喷射但预算更小，并把进入原因写入 report（“无理由喷射”不允许）。
    - TCP 数据面采用 `TLS 1.3 + stream`（含 pinning 身份绑定），不改变上层语义（`ping/sh_attach` 等）。
