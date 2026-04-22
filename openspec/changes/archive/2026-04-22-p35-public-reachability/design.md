## Context

`P3.5` 的目标是把“真实公网实验能跑起来”的最小可达性能力补齐，而不是引入发布级别的产品化抽象。当前公网实验暴露出三个主要摩擦点：

- 需要频繁手敲 `STUN/MQTT` 的 IP 形式（DNS/环境异常导致解析不可用）。
- 双栈环境下缺少对 `P2P/打洞` 的显式地址族限制，导致定位困难。
- `cn/global` 分流使 STUN 观测结果存在多个“公网视角”，如果不显式建模与记录，就无法稳定复现实验与解释决策链。

现状约束：

- `-4/-6` 仅影响 `P2P/打洞` 的候选与尝试路径；不限制 signaling（例如 MQTT 连接 broker 仍可按系统网络栈选择可用地址族）。
- 内置 DNS 只用于 `STUN/MQTT` 主机名解析；不扩展为全局网络栈语义。

## Goals / Non-Goals

**Goals:**
- 提供 `-4/-6` 的最小选择面，明确且可观测地限制 P2P 地址族。
- 在系统 DNS 异常时仍可解析 STUN/MQTT（默认 `auto` 回退），减少手工输入 IP 的依赖。
- 在未显式指定 STUN 时，对内置 STUN 的 `cn/global` 观测面进行采样与确定性仲裁，但该仲裁只作用于 STUN 派生的公网候选，不改变 direct/local 信息交换语义。
- 在 `debug` 日志中可追溯完整证据链；非 debug 记录最终选择与关键原因。

**Non-Goals:**
- 不做面向终端用户的完整配置 UX；不引入 DoT/DoH；不承诺最优线路选择。
- 不引入 relay/fallback 架构；不在本阶段推进 NAT66/UDP6 punching 泛化。

## Decisions

### 1) P2P 地址族限制：`-4/-6` 只约束 punching

- 在 `cmd/miopunch peer`（client/visitor）增加短选项 `-4/-6`。
- 在内部将其归一为一个 `P2PIPFamily`（`auto|v4|v6`）并贯穿 `gather/exchange/attempt`：
  - `v4`：跳过 IPv6 direct gather/attempt；仅保留 IPv4 direct/portmap/punching 分支。
  - `v6`：跳过 IPv4 portmap/punching 分支；仅保留 IPv6 direct 分支（以及必要的 self-check）。
- signaling 不受该开关影响：MQTT/coord 的连通性与地址族由其自身实现决定。

### 2) 内置 DNS：仅用于 STUN/MQTT，默认 `auto`，使用 TCP/53

实现上引入一个最小 `Resolver` 抽象，用于把 `host:port` 解析为 `[]netip.AddrPort`：

- **系统解析优先**：默认先走 `net.DefaultResolver`。
- **回退策略（mode）**：
  - `auto`：系统解析失败才回退到内置 resolver。
  - `on`：强制使用内置 resolver（便于复现实验）。
  - `off`：禁用内置 resolver（只用系统解析）。
- **作用域**：只在两类输入上使用该 resolver：
  - STUN server endpoints（当为域名时）
  - MQTT broker endpoint（当为域名时）
- **传输**：内置 resolver 默认用 `TCP/53`；上游服务器默认集合为 `1.1.1.1/8.8.8.8/223.5.5.5/119.29.29.29`，并支持 YAML 覆盖。

依赖选择：

- 优先使用 Go 标准库 + 最小 DNS message 解析（例如 `x/net/dns/dnsmessage`）实现 TCP 查询；
- 若实现成本过高，可引入一个成熟 DNS 库，但需控制依赖面与默认行为（必须支持 TCP/53）。

### 3) 内置 STUN 的 `cn/global` 分组与单视角仲裁

- STUN 服务器来源：
  - 若用户显式配置 `--stun`（或 YAML `stun:` 非空）：仅使用用户列表；不启用内置 STUN；不做 `cn/global` 分组与仲裁。
  - 否则：使用内置 STUN 列表，并按 `cn` / `global(!cn)` 分组。
- gather 阶段对两组分别采样，生成两份观测结果 `view=cn` 与 `view=global`（若某组全失败则视为不可用）。
- `exchange` 阶段仍按原有语义交换完整信息：
  - `direct/local/assisted/portmap` 相关信息照常传递，不受 `cn/global` 影响；
  - `cn/global` 只附加在 STUN 派生的公网地址观测上。
- 随后使用**确定性**规则选出一个最终视角 `selected_view`；该视角只用于从 `cn/global` 两组里选出“进入 NAT 分析与 punching 的公网 candidates”。

更明确地说，流程应该是：

1. `gather`：
   - 收集原有的 `direct/local/assisted/portmap` 信息；
   - 额外收集两组 `STUN public view`（`cn/global`）。
2. `exchange`：
   - 交换双方原有的非 STUN 信息；
   - 交换双方的 `STUN public view` 观测摘要与对应公网地址。
3. `coordinator`：
   - 对 `STUN public view` 做仲裁，得到唯一 `selected_view`；
   - 仅用该视角对应的公网地址做 NAT 分析与 punching candidate 生成。
4. `attempt/punching`：
   - direct 尝试仍基于原始 direct 信息；
   - assisted/local 信息仍保持原样参与；
   - 只有 STUN 公网 candidate 子集受 `selected_view` 约束。

仲裁顺序（固定）：
1. 可用性（该视角是否有成功观测/可用 candidates）
2. NAT feature 难度（粗粒度等级，避免过拟合；仅用于打平时的区分）
3. STUN RTT（使用 binding request 自身 RTT；阈值 30ms 内视为打平）
4. 成功次数（ok_count）
5. 默认 `global`

该决策在两端独立计算但必须一致：给定同一组输入（双方观测摘要 + 固定规则），两端应得到同一 `selected_view`。

### 4) 可观测性分层

- `debug`：输出两组观测的摘要（成功/失败、RTT、候选数量、NAT 难度等级）与逐步仲裁理由。
- `debug`：额外要能区分“哪些是 STUN 公网候选，哪些是 direct/local/assisted 信息”，避免把 `selected_view` 误解为全局裁剪。
- `info`/更低：至少输出最终 `selected_view` 以及触发该结果的关键理由（例如“cn 不可用 → 选 global”或“RTT 差异显著 → 选 global”）。

## Risks / Trade-offs

- [DNS fallback 误判/误用] → mode 默认 `auto`；仅在解析失败时触发；并在日志中明确记录使用了哪条解析路径。
- [CN/global 仲裁过简] → 以“可复现 + 可解释”为第一目标；规则固定且可观测，后续再用真实样本迭代。
- [仲裁边界被误用] → 明确规定 `selected_view` 只作用于 STUN 公网候选；direct/local/assisted 信息保持原语义与原路径。
- [两端仲裁不一致] → 交换阶段传递足够的观测摘要，并保持仲裁规则纯函数化（无随机、无时间依赖），同时在 debug 输出双方输入以便定位。
