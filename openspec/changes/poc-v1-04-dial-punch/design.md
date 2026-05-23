## Context

dial/punch 是最小闭环中“网络不确定性”最强的一段。POC v1 要求：只保留一条正确路径（UDP punching），并把 attempt 策略、超时、证据输出都定死。

依赖：`poc-v1-03-presence-discover`（获取对端 `peer_id` 与 `x25519_pub`），以及 `poc-v1-06-persistence`（本地派生 `inbox_topic` 所需的 `mailbox_secret`）。

## Scope

- v1 仅支持 UDP punching；dial 协商只通过 `dial_offer/dial_answer`（peer_e2e_v1）完成。
- 本 change 的输出是 `PathResult`：选中的 UDP path、相关资源所有权、以及 punch evidence。
- `dial_offer/dial_answer` 固定最小字段集：`dial_id`、`punch_token`、`candidates`、`member_credential`。
- attempt 策略固定为 5B：最多并发 4 对 candidate pair，总预算 10s，先成功先收敛。
- presence 仅用于观测/便利；`inbox_topic` 必须由本地 `mailbox_secret + peer_id` 派生，不从 presence 获取。
- `KCP/TLS/yamux`、TLS pin、`PeerSession` 升级不属于本 change，交由 `poc-v1-05-secure-session`。

## Owned Paths (planned)

- `internal/task/poc_dial.go`
- `internal/punching/*`

## Done

- `dial_offer/dial_answer` 的最小字段集和投递边界冻结完成。
- `PathResult -> SessionRecipe -> PeerSession` 主干中的 `PathResult` 契约冻结完成，不夹带 KCP/TLS 选择。
- 5B attempt matrix、超时预算、evidence 输出冻结完成，不引入 overlay/mesh fallback。
