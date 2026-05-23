## Context

在 POC v1 中，入网 bootstrap 必须极简且可解释：Invite 不再塞运行时状态；批准后只签发 MemberCredential，并通过 peer_e2e_v1 下发 mailbox_secret。

依赖：`poc-v1-01-controlplane-wire`（TLV + transcript + peer_e2e_v1）。
建议顺序：`poc-v1-06-persistence` 先 apply；本 change 只负责“写什么”，不负责“目录结构怎么长”。

## Scope

- 冻结 MPINV1 `InviteCapability` 的最小字段集：`network_id`、authority 公钥、`broker`、`join_topic`、`invite_id`、`not_after`、`sig`。
- 冻结 `join_request` 边界：joiner 长期公钥、`reply_topic`、PoP 签名；`reply_topic` 由 joiner 随机生成并先 subscribe。
- 冻结 `enroll_response` 边界：只下发 `MemberCredential + mailbox_secret`。
- 冻结幂等口径：authority 以 `msg_id` 去重。
- 不下发 seed peers、topology、mesh、relay 或其它运行时状态。

## Owned Paths (planned)

- `internal/task/invite.go`
- `internal/task/join.go`
- `internal/task/approve.go`
- `internal/controlplane/invite_*`
- `internal/controlplane/member_credential_*`

## Done

- MPINV1、`join_request`、`enroll_response` 的最小字段集与时序冻结完成。
- 入网后只通过 `poc-v1-06-persistence` 提供的布局写入 `MemberCredential + mailbox_secret + broker`。
- 目录结构、原子写、权限策略继续由 `poc-v1-06-persistence` 定义和落地。
