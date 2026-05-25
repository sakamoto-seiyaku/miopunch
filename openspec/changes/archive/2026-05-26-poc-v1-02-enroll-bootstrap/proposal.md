## Why

如果 `Create/Join Network -> Approve/Enroll` 继续沿用当前混合 bundle、历史 invite code 与 legacy task/store 流程，后面的 v1 抽离就会失去可解释性：到底是“新主线”还是“旧产品流程拼装”会再次说不清。

02 的职责是把入网 bootstrap 变成一条能独立实现、能独立验收、且只依赖 `01` 与 `06` 的最小主线：

- `InviteCapability` 只做 entry ticket
- `JoinRequest` 只提交长期身份与回邮箱
- `EnrollResponse` 只下发当前节点最小 bootstrap：`self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`

## What Changes

- 将 02 重写为 `internal/pocv1/enroll` 的抽离蓝图，不再把 authority/joiner 语义继续堆进 legacy `internal/task` 与 `internal/controlplane`。
- 定义并后续实现 `InviteCapability`、`JoinRequest`、`MemberCredential`、`EnrollResponse` 的最小字段集与签名/加密边界，并把当前 v1 bootstrap 收窄到单 broker。
- 固定 authority 侧幂等口径：使用 outer/inner `msg_id` 去重，不另造第二套 bootstrap nonce/request id 体系。
- 规定 bootstrap 成功后的唯一持久化 handoff：通过 `poc-v1-06-persistence` 写入 `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot`。
- 为 02 增加独立的本地 MQTT smoke / restart / idempotency 验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-enroll-bootstrap`: 当前 POC v1 的最小 trust bootstrap 合同。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/enroll/*`
- 计划参考的 legacy 行为：`internal/controlplane/invite_*`、`join_code.go`、`handled_requests.go`、`invite_store.go`、`internal/task/invite.go|join.go|approve.go`
- 不包含 presence、dial/punch、session recipe、GUI 状态机或 topology/runtime state。
