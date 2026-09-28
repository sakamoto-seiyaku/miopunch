# 2026-04-20 文档评审结论（P0–P3，临时）

> 目的：把本轮评审输出与当前结论落盘，避免上下文丢失。  
> 范围：`docs/notes/2026-04-15-alpha-product-discussion.md`、`docs/notes/2026-04-16-alpha-glossary.md`。  
> 严重度：P0=阻断/安全硬伤；P1=高风险设计；P2=中等风险/可优化；P3=一致性/细节。  
> 状态：临时记录；最终以主纲领文档与后续实现 change 为准。

## Quick Positives

- 主能力聚焦在“远程 Shell + tmux 现场”，交互语义可执行（Ctrl-C、resize、单写者等）。
- 可解释性闭环：阶段机 + 诊断树 + `reason_code` + sidecar(best-effort)。
- 控制面安全边界清晰：broker 不可信、E2E AEAD + 签名、HKDF 域分离。

## P0 Blockers（评审原文要点）

1) **不做数据面 relay vs POC 可用性目标冲突**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:13`、（relay 不做的口径）`docs/notes/2026-04-15-alpha-product-discussion.md:592`。  
   - 建议：明确 POC 的网络假设/成功率目标，或引入最小 relay 兜底（哪怕只覆盖 `sh`）。

2) **默认依赖公共 MQTT + 默认不强推 TLS/WSS 的可用性/元数据风险**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:631`、`docs/notes/2026-04-15-alpha-product-discussion.md:626`。  
   - 建议：把“自建 broker”为主推荐路径；公共 broker 明确标注为 demo/测试默认。

3) **invite code 固定 `ip:port` 的可用性风险（IP 变更/anycast/NAT64/IPv6-only/SNI 演进）**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:980`。  
   - 建议：code 携带 hostname + 多地址候选/多端点。  

4) **数据面不做自动协商/降级 → 仅配置不同就完全不可用**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:1549`。  
   - 建议：POC 强制单一栈（如 `quic+bbr`），或把最小交集协商写进 `connect_request`（结合 `hello.capabilities`）。  

5) **签名不覆盖 `dst_peer_id` → 成员可重定向消息且不破坏签名**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:764`。  
   - 建议：把 `dst_peer_id` 纳入签名 transcript（`hop_limit` 可不签）。

6) **丢弃 `abs(now-created_at)>10m` 的消息 → 时钟不准/休眠误杀**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:838`。  
   - 建议：主要依赖 `expires_at_unix_ms`（仅 RPC）+ 去重窗口；墙钟偏差只降级置信度并提示校时。

7) **RPC 幂等缓存“重启清空” vs `approve/max_uses` 不重复计数冲突**  
   - 指向：`docs/notes/2026-04-15-alpha-product-discussion.md:833`（重启清空）+ 入网 `uses` 语义相关段落。  
   - 建议：把“邀请 uses 消耗/已处理 request_id”做最小持久化（覆盖 invite 有效期）。

8) **密钥落盘保护口径缺失**  
   - 指向：本地持久化最小集含 `identity key/net_secret`（例如 `docs/notes/2026-04-15-alpha-product-discussion.md:1146`）。  
   - 建议：明确 threat model；给出 system service 与用户态两种最小权限方案。

## P1 High（评审原文要点）

- POC 范围巨大且耦合：建议先锁 vertical slice（`join → ping → sh` + 基础诊断），再补治理/恢复/UI/mesh 转发。
- “mesh-first + MQTT 兜底”会放大复杂度：POC 可先全程 MQTT 或只在 dataplane 稳定后启用 mesh。
- QUIC 身份绑定需与 KCP 侧同级硬规则；固定预算对移动网络偏紧；单写者锁的心跳硬要求需明确；缺少本地紧急断开手势；面板 CSRF/同源策略对浏览器细节敏感（可考虑尽早引入最小 `ui_token`）；flooding+重试需最小限流口径；数据面多栈/多 cc 增加兼容复杂度（POC 可裁剪）。

### 当前回复/决策（P1，POC 口径）

- “范围巨大/vertical slice”：属于实现排期问题，POC 文档不强行拆解（实现时再按阶段推进）。
- “mesh-first vs 全程 MQTT”：坚持 mesh-first（网内可传就网内传）；MQTT 仅作为无邻居/兜底。
- “flooding/放大风险”：采用受限泛洪（bounded flooding），`H=3`；转发仅做 `dedup + hop_limit-- + 不回传来源邻居`；`hop_limit>H` 直接丢弃；RPC `1s` 无回应再走 MQTT 兜底。
- “QUIC 身份绑定与 KCP 同级硬规则”：同意并已在主纲领文档补齐口径（两者一致）。
- “固定超时预算/预算偏紧”：POC 可接受固定预算（先跑通再调）。
- “单写者锁心跳/释放语义”：锁保活以 WS 存活为准（空闲不释放；`interval=15s`，`ttl=60s`）；异常时允许直接退出/kill 作为兜底。
- “本地紧急断开手势”：暂不做（POC 不过度设计）。
- “面板 CSRF/ui_token”：暂不加；先以 127.0.0.1 + 同源约束跑通，遇到真实坑再补。

## P2 Medium（评审原文要点）

- 配置加载“先匹配先用不 merge”需更醒目；`resolver`/内置 DNS 合规与可用性；`invite_brokers` 过少易被封；控制面 payload 上限与网络规模；多种 26 字符 ID 易贴错类型；面板固定端口对开发不友好；`--redact` 提醒；阶段机对 join/ping/approve 的映射明确性；对端收包矩阵缺失时如何降级；移动端 profile；`--format json` 顶层字段建议稳定。

### 当前回复/决策（P2，POC 口径）

- `--format json`：只承诺顶层最小稳定 envelope（`format/task_id/kind/status/stage/reason_code/exit_code/facts/suggestions`）；不承诺完整 schema。

## P3 Low（一致性/拼写/术语）

- 术语词典里 Session 承载处与“tmux 为硬依赖”口径不一致（`tmux/screen` vs `tmux only`）。
- 拼写：`dettach` → `detach`。
- glossary 中 “Inbox / Mailbox” 术语不一致：缺少 `mailbox` 定义或收敛命名。
- NAT filtering 描述可更精准；`control_plane_wire_format` 可拆分提升可维护性。

### 当前回复/决策（P3）

- 已修复：tmux-only 口径、`detach` 拼写、补齐 `mailbox` 同义词、NAT filtering 定义更精准。

## 当前回复/决策（先从 P0 开始）

### P0-1 relay（评审建议：加最小 relay）

- 结论：**不做**（POC 不做；未来也不引入中心化数据面 relay）。  
- 解释：最多允许“控制面网内转发/兜底入口”，不引入中心化中继服务器。

### P0-2 公共 MQTT（评审建议：自建为主、公共仅 demo）

- 结论：**接受公共 MQTT 作为默认可用入口**。  
- 理由：topic 入口高熵不可枚举 + 控制面端到端加密 + 签名；broker 被攻破也不泄露明文；可接受连接层元数据暴露。

### P0-3 invite 固定 `ip:port`（评审建议：hostname + 多端点）

- 结论：**暂不调整**（POC 以“确保 joiner/approver 命中同一 broker 实例”为优先）。  
- 待讨论：是否在不破坏“命中同一实例”的前提下，允许多端点/hostname 作为增强。  

### P0-4 数据面不做自动协商/降级（评审建议：单栈或协商）

- 结论：**POC 接受“不协商/不降级”**（强依赖后续链路层演进；POC 先用“配置一致 + 可解释性”兜住）。  
- 落地：保留 `DP_STACK_MISMATCH` 的强解释与修复建议；未来在链路层能力稳定后再引入协商/降级。  

### P0-5 `dst_peer_id` 不签名（评审建议：纳入 transcript）

- 结论：**必须签名覆盖 `dst_peer_id`**；`hop_limit` 不签名（用于转发递减）。  
- 影响：转发只能“按原 `dst_peer_id` 转发”；若要改收件人，必须生成一条新消息并由发起方重新签名（可附带原消息作为引用/证据）。  
- 状态：已更新主纲领文档的签名 transcript 口径。

### P0-6 `abs(now-created_at)>10m` 丢弃（评审建议：弱化墙钟依赖）

- 结论：选择 **A**：RPC request 强依赖时间（必须 `expires_at_unix_ms` 且严格过期丢弃）；并保留 `abs(now-created_at)>10m` 的 sanity drop（触发时提示校时）。  

### P0-7 幂等/uses/重启清空（评审建议：最小持久化）

- 结论：**同意**做最小持久化；invite 由签发者 admin 节点负责落盘与恢复。

### P0-8 密钥落盘保护（评审建议：threat model + 最小权限）

- 结论：补齐最低口径：以 OS ACL/权限收敛为主；明确 threat model（本机失陷=该 peer 失陷）；不做自研加密落盘（后置）。  
- 状态：已在主纲领文档补充“落盘保护最低口径”。  
