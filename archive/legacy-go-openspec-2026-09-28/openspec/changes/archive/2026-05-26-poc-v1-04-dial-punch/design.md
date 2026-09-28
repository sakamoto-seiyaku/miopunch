## Context

04 连接的是：

- `03` 的在线态 discover view
- `06` 的 `roster_snapshot + TopicScope`
- `05` 的 `PathResult -> SessionRecipe` 交接面

因此它必须只做协商和建 path，不夹带 session 或 GUI 语义。

## Extraction Strategy

- 新实现进入 `internal/pocv1/punch`。
- 可复用 legacy punching/connectivity 的叶子 mechanics，但新的 v1 orchestration、`dial_offer/dial_answer`、`PathResult` 和 evidence authority 必须在 `internal/pocv1/punch`。
- 远端 `x25519`、远端 `MemberCredential` 与 inbox topic 定位来自 `06` 的 trusted roster / `TopicScope`；presence 只用于在线态参考。
- `dial_offer/dial_answer` 仍携带 `member_credential`，但它不是单独可信源。接收方必须验证 inner sender、credential 派生身份、trusted roster entry 与 authority signature 四者一致后，才可把远端身份写入 `PathResult`。
- `PathResult` 是 04 的最终产物，不夹带 recipe 选择。
- 当前 04 runtime 只消费调用方提供的单个 UDP owner/socket；candidate matrix 的 local 轴是该 socket 对外声明的 host/srflx evidence 集，而不是在 04 内部为每个 local candidate 重新 bind 新 socket。

## Scope

**04 owns:**

- UDP-only dial/punch
- `dial_offer/dial_answer` 的最小 body：
  - `dial_id`
  - `punch_token`
  - `candidates`
  - `member_credential`
- 对方 inbox topic 上的 `peer_e2e_v1` 投递
- 固定 5B attempt matrix：
  - 最多并发 4 对 candidate pair
  - 总预算 10s
  - 先成功先收敛
- `PathResult`
  - selected UDP path
  - 资源所有权
  - 已验证的远端 trusted identity（`peer_id` + `MemberCredential`）
  - punch evidence

**04 does not own:**

- 远端 roster authority 与 inbox topic 派生 authority（`02/06` 提供）
- KCP/TLS/yamux、TLS pin、`PeerSession`（`05`）
- TCP/QUIC/overlay fallback
- GUI stage transitions（`07`）

## Owned Paths (planned)

- `internal/pocv1/punch/*`
- `internal/pocv1/punch/*_test.go`

## Task Breakdown

1. 实现 `dial_offer/dial_answer` encode/decode 与基于 `roster_snapshot + TopicScope` 的 inbox-topic 投递。
2. 建立 v1 `PathResult`、candidate model、evidence model。
3. 在 `internal/pocv1/punch` 中实现 5B attempt orchestrator，并窄适配 legacy UDP punching 叶子 mechanics。
4. 添加两节点 smoke、timeout、partial-failure、selected-path evidence 测试。
5. 通过包内 fake `peerMessageSession` 与 focused unit tests 补齐 exchange/verify/config/runtime 负路径，不把 04 扩成新的重型集成工程。

## Acceptance

- `dial_offer/dial_answer` 只经过 inbox topic + `peer_e2e_v1` 传输。
- 远端 `x25519`、远端 inbox topic 与 `MemberCredential` 都来自 trusted roster + `TopicScope`，而不是来自 presence payload。
- 收到的 `member_credential` 只有在同时匹配 inner sender、trusted roster 与 authority 验签时才可进入 `PathResult`。
- answer 只有在 `dial_id`、`punch_token`、`in_reply_to` 与响应者 `peer_id` 同时匹配当前 exchange 时才可被接受。
- 任何当前 v1 punch run 都只尝试 UDP candidate pairs，不分支到其它 carriers。
- `normalizeCandidates`、attempt budget cause、attempt evidence failure modes 与 winner 收敛行为都有独立自动化测试覆盖。
- `PathResult` 不携带 KCP/TLS/yamux 或 recipe 选择。
- `05` 只消费 `PathResult` 建会话。
