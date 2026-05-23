## Context

在 POC v1 中，入网 bootstrap 必须极简且可解释：Invite 不再塞运行时状态；批准后只签发 MemberCredential，并通过 peer_e2e_v1 下发 mailbox_secret。

依赖：`poc-v1-01-controlplane-wire`（TLV + transcript + peer_e2e_v1）。
建议：`poc-v1-06-persistence` 可以先 apply，用于提供稳定的落盘布局与原子写/权限策略；本 change 只负责“写什么”，不负责“目录结构怎么长”。

## Goals / Non-Goals

**Goals:**
- MPINV1 InviteCapability：entry ticket only（network_id + authority keys + broker + join_topic + expiry + sig）。
- join_request：只提交 joiner 的长期公钥与 reply_topic，并做 PoP 签名。
- enroll_response：只下发 MemberCredential + mailbox_secret。

**Non-Goals:**
- 不下发 seed peers/topology/mesh/relay 等状态。
- 不做多级 issuer/吊销系统。

## Decisions

- `reply_topic` 由 joiner 随机生成（不可猜），joiner 必须先 subscribe 再发 join_request。
- 幂等口径：authority 以 `msg_id` 去重，重复 join_request 不产生副作用。

## Owned Paths (planned)

- `internal/task/invite.go`
- `internal/task/join.go`
- `internal/task/approve.go`
- `internal/controlplane/invite_*` / `member_credential_*`
- `internal/pocstate/*`（若需要在现有 state 系统内挂接 v1 layout 的读写入口）
