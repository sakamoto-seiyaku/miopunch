## Why

POC v1 的最小闭环必须包含 `Create/Join Network -> Approve/Enroll`。当前主线的问题之一是 invite code/入网 bundle 过大、语义混杂，导致流程复杂且难以解释。

本 change 把入网链路收敛为 Hard-Min：InviteCapability 只做 entry ticket；join_request 只提交身份与回邮箱；enroll_response 只下发 MemberCredential + mailbox_secret。

## What Changes

- 定义并实现 v1 `InviteCapability`（MPINV1 编码）与 `join_request/enroll_response` 的最小字段集。
- 固定 join/approve/enroll 的时序与幂等口径（基于 `msg_id` 去重）。
- 入网后持久化：`networks/<network_id>/` 写入 `MemberCredential + mailbox_secret + broker`。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-enroll-bootstrap`: v1 入网 bootstrap（invite/join/enroll）的收敛口径。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：`internal/task/invite|join|approve` 相关逻辑与 `internal/controlplane` 的入网对象定义。
- 不包含 punching/dial/data-plane（由后续 changes 负责）。
