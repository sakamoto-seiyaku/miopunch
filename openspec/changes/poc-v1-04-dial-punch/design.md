## Context

dial/punch 是最小闭环中“网络不确定性”最强的一段。POC v1 要求：只保留一条正确路径（UDP punching），并把 attempt 策略、超时、证据输出都定死。

依赖：`poc-v1-03-presence-discover`（获取对端 x25519_pub + inbox topic）。

## Goals / Non-Goals

**Goals:**
- v1 仅支持 UDP punching（不引入 TCP punching/ICE/TURN）。
- dial 协商只通过 `dial_offer/dial_answer`（peer_e2e_v1）完成。
- attempt 策略固定为 5B 并发矩阵。

**Non-Goals:**
- 不做 trickle candidates。
- 不做 overlay/mesh fallback。

## Decisions

- `dial_id/punch_token` 固定 16B rand。
- candidates 仅包含 host/srflx（按需扩展放到 v2）。
- 成功口径以“punch 包有效 + KCP 建链 + TLS 验证成功”为准。

说明：
- presence 仅用于观测/便利（拿到 `peer_id` 与对端 `x25519_pub`）；**inbox_topic 不从 presence 获取**。
- dial_offer/dial_answer 的投递 topic 必须由本地 `mailbox_secret + peer_id` 派生得到（参照最小闭环 doc 的 inbox 派生口径）。

## Owned Paths (planned)

- `internal/task/poc_dial.go`
- `internal/punching/*`
