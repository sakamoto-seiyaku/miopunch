## Why

POC 控制面需要支持 `mesh-first + MQTT fallback`，在真实网络下不可避免会出现丢包、重试、双路径重复投递与进程重启。若缺少明确的 **RPC 时间语义** 与 **请求幂等/uses 持久化** 约束，将导致：过期请求仍被处理、重复请求产生重复副作用、issuer 重启后重复扣减 `uses` 与重复交付 `membership_bundle`，从而破坏“可解释/可恢复”的 POC 目标。

## What Changes

- 冻结 POC v0 的 RPC 时间语义：
  - RPC request 必须携带 `route.expires_at_unix_ms`，接收端严格过期丢弃。
  - 保留 `abs(now_unix_ms-created_at_unix_ms)>10m` 的 sanity drop，并要求给出明确的校时提示（facts/suggestions）。
- 冻结 POC v0 的 RPC 幂等规则（与 `mesh-first + MQTT fallback` 兼容）：
  - 重试必须复用同一 `request_msg_id`（即 `route.msg_id`），接收端对重复 request 必须重发最终 response（不得重复副作用）。
  - RPC response 必须携带 `signed.in_reply_to=<request_msg_id>`，用于配对、解释与重试闭环。
- 针对 invite/approve 的幂等与 `uses` 计数引入最小持久化（issuer/admin 节点负责）：
  - 持久化 `uses_left`，并持久化 `handled_request_id -> cached_response`（覆盖 invite 有效期窗口），确保 issuer 重启后不重复扣减与不重复交付。
- 明确 dedup 与幂等的边界：
  - 对非 RPC/转发路径：重复 `msg_id` 仍可按 dedup 规则直接丢弃。
  - 对“本机作为 dst 的 RPC request”：重复投递不得被 dedup 直接吞掉，必须走幂等重发路径以完成 request/response。

## Capabilities

### New Capabilities
- `miopunch-poc-control-plane-rpc-time-semantics`: 定义 RPC request 的 `expires_at_unix_ms` 强约束、`created_at_unix_ms` 校时 sanity drop 与可解释输出要求。
- `miopunch-poc-control-plane-invite-approve-idempotency`: 定义 invite/approve 的幂等处理、`uses_left` 扣减一次性、以及 issuer 可重启恢复的最小持久化格式与行为。

### Modified Capabilities
- `miopunch-poc-control-plane-bounded-flooding`: 修改 dedup 规则以允许“dst=self 的 RPC request”触发幂等重发（不允许重复副作用）。
- `miopunch-poc-control-plane-mesh-first-fallback`: 强化双路径/重试下的幂等闭环要求（重复 request 必须可重发最终 response）。

## Impact

- 控制面实现侧（预期影响）：
  - `internal/controlplane` 的 inbound 处理、dedup 策略与 request/response 处理将需要显式区分 best-effort vs RPC request，并引入“已处理 request”缓存。
  - issuer/admin 节点需要新增最小持久化（`invites/<invite_id>.json`）以覆盖 invite 有效期窗口。
- 测试与回归（预期影响）：
  - 单元测试：过期丢弃/校时提示、幂等缓存命中、`uses_left` 不重复扣减。
  - 集成测试：issuer 重启后同一 `request_msg_id` 重放不产生新 `uses` 消耗、且可重发相同 response。
