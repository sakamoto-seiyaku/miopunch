## Context

在 `poc-v1` 抽离路线里，02 负责把“加入网络”从旧 bundle/旧 state/旧 task 语义里剥出来。它必须只依赖：

- `01` 提供的 peer-targeted wire/security
- `06` 提供的 persist authority

这样 02 才能独立被实现、调试和验收，而不是继续受 legacy `internal/task` 与 `internal/pocstate` 结构牵制。

## Extraction Strategy

- 新实现进入 `internal/pocv1/enroll`。
- legacy `internal/controlplane/invite_*`、`join_code.go`、`invite_store.go` 与 `internal/task/invite|join|approve` 只保留行为参考，不再继续叠 v1 语义。
- 02 只拥有 bootstrap 对象与 authority/joiner 流程；目录结构与文件写入 authority 完全交给 06。

## Scope

**02 owns:**

- `InviteCapability` / `InviteCode (MPINV1)` 的最小字段集：
  - `network_id_bytes`
  - `authority_ed25519_pub`
  - `authority_x25519_pub`
  - `broker`（当前 v1 恰好一个 runtime broker endpoint）
  - `join_topic`
  - `invite_id`
  - `not_after_unix_ms`
  - `sig`
- `JoinRequest` 的最小字段集：
  - `invite_id`
  - `requester_ed25519_pub`
  - `requester_x25519_pub`
  - `reply_topic`
  - `device_name`（可选）
  - `platform`（可选）
  - `created_at_unix_ms`
  - `expires_at_unix_ms`
  - requester PoP signature
- `MemberCredential` Hard-Min 字段集：
  - `network_id`
  - `subject_ed25519_pub`
  - `subject_x25519_pub`
  - `role`
  - `not_before`
  - `not_after`
  - `issuer_key_id`
  - `sig`
- `EnrollResponse`：只交付：
  - `self_member_credential`
  - `mailbox_secret`
  - `runtime_broker`
  - `roster_snapshot`
- bootstrap handoff contract：
  - joiner 将上面四项作为一个 grouped package 交给 06
  - 06 负责把该 package 作为单个 joined-state write 落盘
- `roster_snapshot` 的最小 entry：
  - `peer_id`
  - `member_credential`
  - `device_name`（可选）
  - `platform`（可选）
- authority 侧基于 `msg_id` 的去重与 cached response 语义

**02 does not own:**

- seed peers、mesh、relay、独立目录查询协议
- `mailbox_secret` 的目录结构与权限策略（`06`）
- presence topic / snapshot（`03`）
- dial/punch/session/GUI 逻辑（`04/05/07`）

## Owned Paths (planned)

- `internal/pocv1/enroll/*`
- `internal/pocv1/enroll/testdata/*`
- `internal/pocv1/enroll/*_test.go`

## Task Breakdown

1. 实现 `InviteCapability` / `InviteCode (MPINV1)` 的 TLV encode/decode、签名与显示格式。
2. 实现 `JoinRequest` encode/decode、PoP 验证与 `reply_topic` 预订阅前置条件。
3. 实现 authority 侧 approve/enroll 逻辑：`msg_id` 去重、cached response、`EnrollResponse` peer_e2e_v1 投递。
4. 实现 `MemberCredential` 验签、`peer_id` 推导一致性、`roster_snapshot` 组装与 grouped bootstrap handoff 到 06 persist API。
5. 增加本地 MQTT smoke 与 authority restart/idempotency 测试。

## Acceptance

- 邀请码只包含 entry-ticket 字段，不夹带运行时状态。
- `JoinRequest` 与 `EnrollResponse` 都通过 `01` 的 v1 wire/security 路径传输。
- authority 重放同一 `msg_id` 不重复副作用，并能返回 cached response。
- joiner 成功入网后只写入 `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`，且写入通过 06 的 grouped bootstrap persist API 作为单个 joined-state handoff 完成。
