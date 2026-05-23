## Context

POC v1 的 Discover 目标不是“展示系统内部状态机”，而是给后续消费者一份最小、可解释的在线成员快照。

依赖：`poc-v1-02-enroll-bootstrap`（拿到 `mailbox_secret -> net_root -> presence topic`）。

## Scope

- 冻结 presence topic：`mp/v1/net/<net_root>/presence/<peer_id>`。
- 冻结 retained `online` + retained LWT `offline` 的上线/离线语义。
- 冻结 presence JSON payload 字段集：`v/state/peer_id/ed25519_pub_b64url/x25519_pub_b64url/device_name/platform/app_ver/ts_unix_ms`。
- 冻结 discover 输入契约：订阅 `.../presence/+` 后得到 peer list snapshot。
- presence 只用于观测和便利，不作为安全语义来源。

## Owned Paths (planned)

- `internal/controlplane/presence/*`

## Done

- presence topic、payload、retained/LWT 语义冻结完成。
- Discover 的输入被限制为 `.../presence/+` 的订阅结果，不引入目录查询 kind。
- GUI 渲染、状态机、reason_code 继续由 `poc-v1-07-gui-wizard` 拥有。
