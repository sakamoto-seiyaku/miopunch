## Context

POC v1 的 Discover 目标不是“展示系统内部状态机”，而是让用户一眼看到：有哪些 peer、谁在线、以及后续 dial/punch 所需的最小信息。

依赖：`poc-v1-02-enroll-bootstrap`（拿到 mailbox_secret -> net_root -> presence topic）。

## Goals / Non-Goals

**Goals:**
- 订阅 `.../presence/+` 即可得到成员快照与在线状态。
- presence payload 包含对端 `x25519_pub`，让 dial_offer 可直接加密到对端。

**Non-Goals:**
- 不引入目录查询/应答 kind。
- presence 不作为安全语义来源（仅用于观测/便利）。

## Decisions

- 上线时 publish retained `online`；LWT 设置为 retained `offline`（同 topic）。
- payload 固定字段集：`v/state/peer_id/ed25519_pub_b64url/x25519_pub_b64url/device_name/platform/app_ver/ts_unix_ms`。

## Owned Paths (planned)

- `internal/controlplane/presence/*`
- `internal/task/desktop_state.go`（或后续新 GUI state 模块）
