## Context

03 是 `02 -> 04/07` 之间的窄接口：成员成功 enroll 后，系统必须能在不引入第二套控制面查询协议的前提下，给 punch 与 GUI 一份最小成员快照。

依赖：

- `02`：把 `self_member_credential + mailbox_secret + runtime_broker + roster_snapshot` 作为 joined bootstrap 交给 `06`
- `06`：提供 `runtime_broker`、`roster_snapshot`、`TopicScope` 与 device keys / `self_peer_id` 派生能力；如果 `last_seen_peers` 未来需要落盘，03 先冻结自己的 object model
- caller/runtime：提供本机 `device_name`、`platform`、`app_ver` 这些非信任 display hints

## Extraction Strategy

- 新实现进入 `internal/pocv1/presence`。
- presence 只承接“观测/便利”语义，不再混入 topology、neighbor maintenance、governance 或 GUI stage 私货。
- `internal/pocv1/presence` 拥有唯一的 domain discover contract；GUI 与 punch 都消费同一份由 `persisted roster + presence state` 合成的 `DiscoverView`，而不是各自从 legacy 状态拼装。
- `07` 只拥有从 `DiscoverView` 到 runtime DTO 的投影权，不重新定义 discover 语义；`04` 只消费其中的在线态 surface，不重新发明 trust lookup。

## Runtime Inputs

- `network_id`
- `runtime_broker`
- `TopicScope`
- `roster_snapshot`
- device keys 派生的 `self_peer_id`
- caller-supplied 的本机 `device_name` / `platform` / `app_ver`

这些输入里，trusted peer set 与 trust material 只来自 `06`；本机 display hints 只用于本机 publish presence payload，不反向写入 trusted roster authority。

## Domain Contracts

### Observation

`Observation` 是单条 presence topic 的规范化观测，字段固定为：

- `peer_id`
- `online_state`：`online | offline`
- `device_name`
- `platform`
- optional `app_ver`
- optional `last_observed_unix_ms`

### DiscoverView

`DiscoverView` 是 03 的唯一 discover-owned 输出合同，字段固定为：

- `network_id`
- `self_peer_id`
- `observed_at_unix_ms`
- `peers[]`

其中 `peers[]` 只包含 trusted remote peers：

- 每个 canonical `peer_id` 最多一条
- 排除 `self_peer_id`
- peer set 由 `roster_snapshot` 决定，而不是由 presence topic 枚举决定

### DiscoverPeer

`DiscoverPeer` 字段固定为：

- `peer_id`
- `online_state`：`online | offline`
- `device_name`
- `platform`
- optional `app_ver`
- optional `last_observed_unix_ms`

`DiscoverPeer` 不携带：

- `MemberCredential`
- 远端 `x25519`
- `inbox_topic`
- dial / session state

这些 authority 继续留在 `06` 与 `04`。

### LastSeenPeer

03 冻结最小 `LastSeenPeer` object model，但本 change 只定义模型与 merge 语义，不接持久化：

- `peer_id`
- `last_state`
- `last_observed_unix_ms`
- optional `last_online_unix_ms`

## Merge Rules

- trusted roster 决定 discover peer set；对每个 trusted remote peer 都产出一条 `DiscoverPeer`
- 如果某 trusted remote peer 从未出现 presence，则它仍出现在 `DiscoverView.peers[]` 中，但 `online_state=offline`
- 匹配上的 presence observation 只负责更新 `online_state`、`app_ver`、`last_observed_unix_ms`
- `device_name` / `platform` 以 roster 为 canonical；presence 只能在 roster 对应字段为空时补空，不允许覆盖非空 roster 值
- unknown presence-only peer 不进入 `DiscoverView.peers[]`；它只产生 typed diagnostic / evidence
- malformed JSON、unsupported `v`、invalid `peer_id`、topic-payload `peer_id` mismatch 都不得改变 `DiscoverView`
- `ts_unix_ms` 只作为显示与证据字段，不作为与 broker delivery order 竞争的 merge authority

## Lifecycle Rules

- consumer 订阅：`mp/v1/net/<net_root>/presence/+`
- producer connect 成功后立即 publish retained `online`
- producer graceful shutdown 时主动 publish retained `offline`，然后再 disconnect
- producer 同时配置同 topic 的 retained LWT `offline` 处理 unexpected disconnect
- retained snapshot 先 hydrate 本地 observation，再应用 live updates
- broker delivery order 决定同一 peer 的最终状态覆盖
- 相同 state 且 payload bytes 相同的重复 retained/live update 视为 no-op

## Scope

**03 owns:**

- `net_root` 派生后的 presence topic：`mp/v1/net/<net_root>/presence/<peer_id>`
- retained `online` + retained LWT `offline`
- graceful shutdown 主动 retained `offline`
- 固定 JSON payload：
  - `v`
  - `state`
  - `peer_id`
  - `device_name`
  - `platform`
  - `app_ver`
  - `ts_unix_ms`
- `peer_id` keyed `Observation`
- `DiscoverView` / `DiscoverPeer` domain contract
- roster-bounded merge policy 与 diagnostic-only rejection policy
- `LastSeenPeer` 的最小 object model（先在 03 冻结；不要求 06 foundation 先替它定 typed schema，也不在本 change 接持久化）

**03 does not own:**

- security/trust semantics（presence 不是信任来源）
- 远端 `MemberCredential`、远端 `x25519` 与 inbox topic authority（`02/06` 提供输入）
- roster authority、topic derivation authority、runtime broker selection（`06` 提供输入）
- dial 协商、session、GUI stage transitions（`04/05/07`）

## Owned Paths (planned)

- `internal/pocv1/presence/*`
- `internal/pocv1/presence/*_test.go`

## Task Breakdown

1. 实现 `Observation`、`DiscoverView`、`DiscoverPeer`、`LastSeenPeer` 这四个 domain contract。
2. 接入 `runtime_broker`、`TopicScope`、`roster_snapshot`、device-key 派生的 `self_peer_id` 与 caller-supplied display hints。
3. 实现 presence topic publish/subscribe、retained `online`、graceful retained `offline` 与 retained LWT `offline`。
4. 实现固定 JSON payload encode/decode、`peer_id` keyed observation model，以及 diagnostic-only rejection。
5. 实现 retained snapshot + live update 与 06 `roster_snapshot` 的合并逻辑：remote-only peer set、offline default、roster display precedence。
6. 只冻结 `LastSeenPeer` 的 object model 与内存 merge 语义；不在本 change 扩张 06 persist authority。
7. 为 retained snapshot、roster-only offline、graceful shutdown、unexpected disconnect、reconnect、duplicate convergence 增加测试。
8. 增加 `04/07` consumer seam 验收，确保不回退到 legacy re-join。

## Acceptance

- retained snapshot hydrate 后，每个 trusted remote peer 最多只出现一条 discover row。
- roster 中存在但尚未出现 presence 的 remote peer 仍出现在 discover 中，且默认 `online_state=offline`。
- 同一 peer reconnect 后 Discover 仍只看到一份最新状态。
- graceful shutdown 与非正常断连都会让状态收敛到 retained `offline`。
- unknown / malformed presence 不进入 discover view，只进入 typed diagnostic / evidence。
- `04` 只把 `DiscoverPeer.online_state` 当作在线态输入；远端 `x25519` / inbox 定位仍来自 06 `roster_snapshot + TopicScope`。
- `07` 直接读取同一 `DiscoverView` 再投影到 runtime DTO，而不必拼 legacy task state。
- 03 不再要求 06 foundation 先替 `last_seen_peers` 定 typed schema，也不在本 change 给 06 增加新的 file responsibility。
- presence payload 不进入任何签名、授权或路由信任链。
