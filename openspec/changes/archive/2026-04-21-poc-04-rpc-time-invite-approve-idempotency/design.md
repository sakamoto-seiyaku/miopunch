## Context

仓库已落地 POC-02/POC-03 的控制面基础设施：

- wire format：`proto_version/route/signed`，其中 `route.created_at_unix_ms` 与 `route.expires_at_unix_ms` 已存在且被签名 transcript 覆盖。
- bounded flooding（H=3）+ dedup window（`seen(cap=8192, ttl=10m)`）+ mesh-first + MQTT fallback 的最小投递策略。

当前缺口：

- 时间字段尚未形成明确可测的语义：哪些消息必须携带 `expires_at_unix_ms`、接收端如何严格过期丢弃、以及触发 `abs(now-created_at)>10m` 时的可解释输出口径。
- dedup 与 RPC 幂等边界不清：在 `mesh-first + MQTT fallback`、重试与双路径重复投递下，重复 request 可能被 dedup 直接吞掉，从而无法重发 response 完成闭环。
- invite/approve 的 `uses` 与“已处理 request”在 issuer 重启后缺少恢复机制，会导致重复扣减与重复交付。

## Goals / Non-Goals

**Goals:**

- 冻结 POC v0 的 RPC 时间语义（`expires_at_unix_ms` 必需、严格过期丢弃、`created_at_unix_ms` 校时 sanity drop + 可解释输出）。
- 冻结 POC v0 的 RPC 幂等规则（请求重试复用 `request_msg_id`，重复 request 触发“重发最终 response”，不重复副作用）。
- 为 invite/approve 定义 issuer(admin) 的最小持久化（覆盖 invite 有效期窗口）：`uses_left` 与 `handled_request_id -> cached_response`，可重启恢复。
- 明确 dedup（转发/防环）与幂等（request/response）边界，保证双路径与重试不会破坏闭环。

**Non-Goals:**

- 不在本 change 内实现完整的产品 CLI/daemon 或所有控制面消息族；这里只冻结语义与最小持久化格式/边界。
- 不引入数据库；持久化仅定义最小 JSON 文件格式与原子写策略。
- 不引入新的加密落盘方案（POC 口径：依赖 OS ACL/权限收敛）。

## Decisions

### 1) RPC request 的识别与字段约束

- 识别规则（POC v0，KISS）：`signed.kind` 以 `_request` 结尾的消息视为 **RPC request**。
- RPC response 约束：response **必须**设置 `signed.in_reply_to=<request_msg_id>`（其中 `request_msg_id == route.msg_id`）。
- 语义边界：只有 RPC request 强制时间语义与幂等重发；best-effort 类消息沿用“可丢弃”口径。

### 2) 时间语义（expires + clock-skew sanity）

- RPC request **必须**携带 `route.expires_at_unix_ms`：
  - `now_unix_ms > expires_at_unix_ms` 视为过期；接收端严格丢弃（并记录可解释 facts/suggestions）。
- `created_at_unix_ms` 的 sanity drop（POC v0 固定阈值）：
  - 若 `abs(now_unix_ms-created_at_unix_ms) > 10m`：接收端丢弃，并输出“本机/对端可能未校时”的提示（建议用户同步时间）。
- 备注：时间语义使用墙钟（unix ms）；超时/重试预算仍应以本地 monotonic 计时为主，避免依赖对端时钟精度。

### 3) 重试规则与不变量（避免“同 msg_id 不同请求”）

- 重试必须复用同一 `request_msg_id`（`route.msg_id`）。
- 允许刷新 `created_at_unix_ms` 与 `expires_at_unix_ms`（用于长时间重试时避免被 10m sanity drop 误杀）。
- 除上述两个时间字段外，重试请求的其余签名 transcript 字段（`dst_peer_id/sender_peer_id/kind/in_reply_to/body`）必须保持一致；若 receiver 在同一 `request_msg_id` 上观察到不一致，视为协议违规并丢弃（并记录事实）。

### 4) 幂等处理：handled-request cache（通用 vs invite/approve）

- 通用 RPC（非 invite/approve）：
  - receiver 维护 in-memory `handled_requests`：`request_msg_id -> cached_response_ciphertext`。
  - TTL：至少覆盖该 request 的有效期窗口，且最小 `10m`；容量上限建议 `1024`（超出则按最旧/最早过期淘汰）。
  - 重复 request 到达：直接重发缓存的最终 response（不得重复副作用）。
- invite/approve（issuer/admin）：
  - 除 in-memory cache 外，必须持久化 `handled_requests` 与 `uses_left`，确保进程重启后仍可重发最终 response 且不重复扣 `uses`。

### 5) dedup 与幂等的边界（mesh/MQTT 双路径兼容）

- 转发路径（`dst_peer_id != self_peer_id`）：dedup 继续用于防环/限资源，重复 `msg_id` 可直接丢弃。
- 本机作为 dst（`dst_peer_id == self_peer_id`）：
  - best-effort：可继续按 dedup 规则丢弃重复消息。
  - RPC request：重复投递不得被 dedup “吞掉”；必须进入幂等处理路径（命中 `handled_requests` 则重发 response；未命中则执行一次性副作用并缓存最终 response）。

### 6) invite store：文件命名、格式、原子写

- `invite_id`（用于文件名/索引，不承载安全语义）：`invite_id = base32(raw,no-pad, sha256(invite_topic)[:16])`（固定 26 字符）。
- 持久化路径（相对 state dir）：`invites/<invite_id>.json`（仅 issuer/admin 节点写入与读取）。
- 最小字段（JSON）：
  - `invite_topic`（string）
  - `expires_at_unix_ms`（int64）
  - `max_uses`（int）
  - `uses_left`（int）
  - `handled_requests`（object：`request_msg_id -> response_ct_b64url`）
- 缓存 response 格式：`response_ct_b64url` 使用 base64url no-pad（便于稳定落盘与跨平台拷贝）。
- 写入策略：同目录 `tmp → fsync → rename` 原子更新；恢复时允许最佳努力读取，损坏则报错并给出修复建议（例如“重新签发 invite”）。

## Risks / Trade-offs

- [墙钟偏差导致误丢弃] → 以 `expires_at_unix_ms` 为主、`10m` 阈值足够宽；触发时必须输出校时建议；发送端重试允许刷新时间字段。
- [handled_requests 增长无界] → 明确容量上限与按过期/最旧淘汰；invite store 仅需覆盖 invite 有效期窗口。
- [落盘包含密文 response 仍有本机泄露面] → POC 口径依赖 OS ACL；文档明确 threat model（本机失陷=该 peer 失陷）。
- [同一 msg_id 被复用为不同请求] → 规定重试不变量；若观察到同 `request_msg_id` 的 transcript 关键字段不一致则丢弃并记录事实。
