## Context

03 是 `02 -> 04/07` 之间的窄接口：成员成功 enroll 后，系统必须能在不引入第二套控制面查询协议的前提下，给 punch 与 GUI 一份最小成员快照。

依赖：

- `02`：拿到 `mailbox_secret`、`network_id` 与初始 `roster_snapshot`
- `06`：读写 `roster_snapshot` 与 `last_seen_peers` 的持久化 authority

## Extraction Strategy

- 新实现进入 `internal/pocv1/presence`。
- presence 只承接“观测/便利”语义，不再混入 topology、neighbor maintenance、governance 或 GUI stage 私货。
- GUI 与 punch 都消费同一份由 `persisted roster + presence state` 合成的 typed discover view，而不是各自从 legacy 状态拼装。

## Scope

**03 owns:**

- `net_root` 派生后的 presence topic：`mp/v1/net/<net_root>/presence/<peer_id>`
- retained `online` + retained LWT `offline`
- 固定 JSON payload：
  - `v`
  - `state`
  - `peer_id`
  - `device_name`
  - `platform`
  - `app_ver`
  - `ts_unix_ms`
- Discover 输入契约：订阅 `.../presence/+` 得到 `peer_id -> online/offline` 观测，再与 06 的 `roster_snapshot` 合并成 discover view
- `last_seen_peers` 的最小更新模型（通过 06 persist API）

**03 does not own:**

- security/trust semantics（presence 不是信任来源）
- 远端 `MemberCredential`、远端 `x25519` 与 inbox topic authority（`02/06` 提供输入）
- dial 协商、session、GUI stage transitions（`04/05/07`）

## Owned Paths (planned)

- `internal/pocv1/presence/*`
- `internal/pocv1/presence/*_test.go`

## Task Breakdown

1. 实现 presence topic 派生、connect publish、disconnect LWT 配置。
2. 实现固定 JSON payload encode/decode 与 `peer_id` keyed 的 observation model。
3. 实现 retained snapshot + live update 与 06 `roster_snapshot` 的合并逻辑，并通过 06 持久化 `last_seen_peers`。
4. 为 reconnect、offline、duplicate retained/live update 增加测试。

## Acceptance

- 同一 peer reconnect 后 Discover 仍只看到一份最新状态。
- 非正常断连时 retained LWT 能把状态更新为 `offline`。
- `04` 只把 Discover view 当作在线态输入；远端 `x25519` / inbox 定位仍来自 06 `roster_snapshot + TopicScope`。
- `07` 能直接读取同一 discover view 做 GUI 展示，而不必拼 legacy task state。
- presence payload 不进入任何签名、授权或路由信任链。
