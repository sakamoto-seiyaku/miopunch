## Context

当前仓库中 STUN 相关逻辑主要集中在 `connectivity/*`，并且以 **UDP STUN** 为前提：

- endpoint 解析只接受 `host:port` 与 `udp://host:port`（`tcp://...` 会直接报错）。
- 采样实现针对 UDP 做了“受限并发 + 单 socket 读循环 + txid 分发”（见 `sharedSTUNClient`），但这些能力被写死在 `connectivity` 内，难以被其他入口复用。
- `connectivity/stun_internal.go` 的内置 STUN buckets 目前只依据 UDP 证据排序与收敛；对于后续 TCP 打洞方向来说，缺少“哪些端点支持 STUN over TCP”的证据与标注。

Door 2（TCP 打洞）要落地，必然需要：

1) STUN over TCP（或等价的 TCP 映射观测机制）；以及  
2) 端点列表按协议能力分类（dual / udp-only / tcp-only），否则默认配置会在 TCP 路径上大量超时。

因此本 change 先做“模块化 + probe + 证据驱动更新内置列表”的铺垫，避免后续 TCP 方向在基础设施上反复返工。

## Goals / Non-Goals

**Goals:**
- 把 STUN 的“endpoint 解析/过滤、DNS 解析、UDP/TCP roundtrip、probe 聚合输出”整理成可复用模块 `internal/stunclient`。
- 在同一套代码中同时支持 STUN over UDP 与 STUN over TCP（TCP 为 stream，需要按 header length 读取完整响应）。
- 为证据采集提供独立入口：`miopunch-lab stun probe`（输出 JSONL），用于验证内置 STUN buckets 的 TCP/UDP 支持性。
- 允许 STUN endpoint 以 `udp://` / `tcp://` / `host:port(dual)` 表达协议能力；在仅 UDP 的 gather 语义中，`tcp://` 端点必须被忽略而不是导致整体失败。
- 基于 probe 证据更新内置 STUN buckets 的 scheme 标注，以支撑 Door 2 后续工作。

**Non-Goals:**
- 不在本 change 中引入 TCP punching、wire 扩展或 TCP dataplane（这些属于后续 Door 2 changes）。
- 不在运行时动态在线探测/排序/自学习内置 STUN 列表；本 change 只提供可复现 probe 工具与人工更新代码的路径。
- 不改变现有 peer gather 的“UDP-only STUN 观测与 cn/global 仲裁”语义（只扩展 endpoint 语法并重构复用代码）。

## Decisions

### 1) 复用模块位置：`internal/stunclient`

**Decision:** 新建 `internal/stunclient`，由 `connectivity` 与 `cmd/miopunch-lab` 共同复用。

**Why:**  
- `connectivity` 是对外语义层，不适合承载“可被其他入口复用”的低层实现细节。  
- 现有 `stun/` 包是 STUN server；把 client 混入同包会混淆职责（server vs client），并引入不必要的依赖耦合。  
- `internal/` 允许我们明确“这是仓库内部复用模块，不是公共 API”。

**Alternatives considered:**
- 放在 `connectivity` 内：少一个包，但 `miopunch-lab` 复用会变得不自然，且将来 TCP 方向会继续膨胀 `connectivity` 的内部细节。
- 扩展 `stun` 包：导入更简单，但 server/client 职责混杂，且对未来维护不友好。

### 2) Endpoint 语法：`host:port`（dual）+ `udp://` + `tcp://`

**Decision:** 采用 scheme 前缀表达协议能力：
- `host:port`：dual（UDP/TCP 均可能可用）
- `udp://host:port`：UDP-only
- `tcp://host:port`：TCP-only

**Why:**  
- 与现有代码对 `udp://` 的兼容保持一致（已有 strip 逻辑）。  
- 避免为“内置列表/显式列表/未来 tcp_only 模式”引入多套字段或多份列表。  
- 让 probe 产出的分类结果可以直接落到内置列表字符串上（可读、可 diff、可 review）。

**Alternatives considered:**
- 在代码里维护三份列表（dual/udp/tcp）：实现简单，但会造成配置/文档重复，且最终仍需要在 CLI/YAML 表达。
- 额外引入结构化 endpoint（JSON/YAML object）：更规范，但破坏现有 CLI 的 “comma list” 风格，且对 POC/实验不划算。

### 3) TCP STUN 实现：严格按 STUN header length 读取

**Decision:** TCP STUN client 读取响应时必须按 STUN header 的 message length 精确 `ReadFull`（处理半包/粘包）。

**Why:** TCP 是 stream；如果按一次 `Read` 视为一条消息，会导致偶发 decode 失败或“读到半包”。

### 4) Probe 入口：复用 `miopunch-lab stun` 命令树，新增 `stun probe`

**Decision:** 新增 `miopunch-lab stun probe` 子命令；保留现有 `miopunch-lab stun`（UDP STUN server）不变。

**Why:**  
- `miopunch-lab` 已承担实验/辅助工具链入口，probe 也属于此范畴。  
- 不增加新的二进制，减少发布/使用成本。

**Alternatives considered:**
- 新增独立 `miopunch-stun-probe`：更纯粹，但会扩散二进制数量，与当前 POC 后续方向不匹配。

### 5) 输出格式：JSONL（1 行 1 endpoint 聚合结果）

**Decision:** probe 输出 JSONL（stdout），可选 `--out` 落盘同内容。

**Why:**  
- 端点多时可流式输出，便于边跑边看；也便于后续脚本处理与 grep。  
- 和仓库已有 `docs/reports/*.jsonl` 证据形式一致（如 2026-04-14 的 STUN 相关记录）。

## Risks / Trade-offs

- [TCP STUN 端点大量超时] → 默认短超时 + 受限并发；并把 “被忽略/不支持” 作为可解释输出而不是硬失败。
- [重复实现导致行为漂移] → 强制 `connectivity` 与 `miopunch-lab probe` 复用 `internal/stunclient`。
- [Go 并发读同一 UDPConn 抢包] → 复用现有 “单 read loop + txid 分发” 模式，并把实现迁入模块。
- [证据与内置列表不同步] → 在 tasks 中明确：probe 产物（docs/reports）与内置列表更新是同一个 change 的验收项。
