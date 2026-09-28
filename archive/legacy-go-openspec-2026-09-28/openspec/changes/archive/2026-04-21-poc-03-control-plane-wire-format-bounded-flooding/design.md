## Context

本仓库当前已落地 POC-02 的 mailbox 基础能力（inbox topic 派生、join code `invite_brokers` pinning；见 `internal/controlplane/*` 与 `openspec/specs/miopunch-poc-control-plane-mailbox/spec.md`），但控制面“已入网消息”的 wire format、安全封装（签名/密文 framing）、以及网内转发（bounded flooding / 去重 / 限流）仍停留在讨论文档与 roadmap 中：

- `docs/roadmap.md`：POC-03 的交付与测试口径（签名覆盖 dst、H=3、去重/限流、drop facts、LAN 3 进程 smoke）。
- `docs/notes/2026-04-15-alpha-product-discussion.md` 与 `docs/notes/2026-04-16-alpha-glossary.md`：POC v0 的字段集合、签名覆盖范围、密文 framing、去重窗口与 bounded flooding 语义（“敲定”口径）。

当前缺口导致两个直接问题：

1) 协议不冻结：后续实现会在字段/签名覆盖范围上反复摇摆，难以稳定测试与兼容。
2) 无可回归的网内转发最小实现：三节点/多节点验证无法系统化复现，且缺少 drop facts 时排障成本高。

## Goals / Non-Goals

**Goals:**
- 冻结并落地 POC v0 控制面 wire format（明文 JSON 结构 + 签名 transcript 覆盖规则）。
- 落地 group wrapper 的 AES-256-GCM 密文 framing（`v||nonce||ct`），作为网内转发与 MQTT mailbox 的统一承载 payload（broker/转发路径只看到密文 bytes）。
- 落地 bounded flooding（H=3）最小实现：去重窗口（LRU+TTL）、转发队列上限、drop facts 统计口径。
- 提供可回归验证：
  - 单元测试覆盖签名覆盖范围、密文 framing、去重窗口；
  - 3 节点模拟集成测试覆盖 H=3、dedup、queue drops；
  - 提供同一 LAN 的 3 进程 smoke harness（手动可复现）验证“mesh 优先 + MQTT 兜底”不会互相打架。

**Non-Goals:**
- 不实现完整 membership/state-sync/治理快照链等控制面业务（本 change 仅提供 wire/crypto/routing 基础设施与最小 smoke）。
- 不实现 pairwise 内层加密（仅实现 group wrapper）；后续如需“仅收件人可读”再单独开 change。
- 不把这些能力直接接入产品 `miopunch` CLI/daemon（POC-01 已拆分；本 change 的可运行入口放到 `tools/miopunch-cp-smoke/` 作为实验/回归工具）。
- 不引入可配置项膨胀：H/队列/窗口参数暂按 POC 约定常量冻结（后续压测再调整）。

## Decisions

### 1) 明文结构与 canonical JSON

- 明文消息结构固定为：
  - 顶层：`proto_version:int`、`route:{...}`、`signed:{...}`
  - `signed.body` 使用 `json.RawMessage`，要求生成端用 `json.Marshal(struct)` 产生确定性 bytes（避免 `map` 进入签名输入）。
- `canonical_json(x)` 口径：直接使用 Go 的 `json.Marshal` 输出（依赖 struct 字段顺序确定性）；签名输入禁止 `map`。

### 2) 签名覆盖范围（覆盖 dst，不覆盖 hop_limit）

- 签名 transcript 覆盖 `route.dst_peer_id`，防止成员在不破坏签名的情况下改收件人。
- `route.hop_limit` 不纳入 transcript：转发节点只允许 `hop_limit--` 并重封装外层 group wrapper。
- transcript 字段顺序冻结（与 glossary 一致）：`dst_peer_id + msg_id + created_at_unix_ms + expires_at_unix_ms? + sender_peer_id + kind + in_reply_to? + body`。

### 3) group wrapper AEAD 与密文 framing

- AEAD 固定使用 `AES-256-GCM`（不做算法协商）。
- key 派生：`HKDF(net_secret, salt=sha256(net_secret)[:16], info="miopunch/v0/aead.ctrl.group", L=32)`。
- framing 冻结：`v(1B) || nonce(12B) || ct`：
  - `v=0` 表示 `AES-256-GCM`；
  - `nonce` 使用随机 `12B`；
  - `ct` 含 GCM tag。

### 4) bounded flooding engine（H=3）与去重/队列

- bounded flooding 常量冻结：`H=3`，合法范围 `0..3`，`hop_limit>H` 直接丢弃。
- 转发节点处理流程（最小）：
  1) 解密 group wrapper；失败丢弃并计数（避免转发“不可验真/不可解密”的垃圾）。
  2) 解析 route；执行 `hop_limit` 上界检查与 `msg_id` 去重。
  3) 若 `dst_peer_id == self`：交给上层 handler（本 change 仅提供回调接口）。
  4) 否则：`hop_limit==0` 则不转发；`hop_limit>0` 则执行 `hop_limit--`，重封装密文，fan-out 到除来源邻居外的所有邻居。
- 去重窗口（POC v0）：LRU + TTL，`seen(cap=8192, ttl=10m)`；默认静默去重，debug/事实统计用于排障。
- 队列上限：转发队列 `forward_queue_max=1024`；队列满时直接丢弃新到的“待转发消息”，并累计 drop facts。

### 5) mesh-first + MQTT fallback（用于 LAN smoke 的最小闭环）

为满足 roadmap 的“同一 LAN 3 进程 smoke”，提供一个最小可运行 harness（不等同于最终产品实现）：

- 传输面：
  - mesh：UDP 发送/接收控制面密文 bytes（LAN 直连即可，无需 NAT 穿透）。
  - mqtt：订阅自身派生 inbox topic；fallback 时把同一密文 payload publish 到 `dst` 的 inbox topic。
- 策略（最小）：
  - 对 request/response 类消息：先尝试 mesh 发送（必要时 `hop_limit=H`）；等待 `1s` 无响应则执行 MQTT fallback。
  - 若 request 通过 MQTT inbox 路径到达（即对端已 fallback），接收端的 response 也通过 MQTT publish 回 `sender` 的 inbox，以保证“无 mesh 回程”时仍能闭环完成 request/response。
  - 接收端通过 `msg_id` 去重保证“重复投递不产生重复副作用”，避免 mesh 与 MQTT 双路径打架。

## Risks / Trade-offs

- [签名输入包含非确定性 JSON] → `body` 仅接受 `json.Marshal(struct)` 生成的 bytes；测试锁定 transcript 生成逻辑。
- [goroutine/队列泄漏或阻塞] → forwarding worker 提供显式 `Close()` 并可等待退出；队列满直接丢弃（不反压主流程）。
- [LAN smoke 与最终产品实现偏离] → harness 只承载最小验收口径（mesh-first + mqtt fallback + dedup），并在文档中明确其用途为 smoke/回归，而非产品入口。
