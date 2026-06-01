## Why

当前仓库里“发现 peer”既混着 topology/neighbor 语义，又混着 GUI 状态组装。对于这轮 `poc-v1` 抽离，这样的 Discover 不可解释，也无法独立验收。

03 的职责是把 Discover 收敛成一件单纯的事：用 retained + LWT presence 形成一份最小在线成员快照，再和 `02/06` 持久化的 trusted roster 合并，供 `04` 与 `07` 使用。

## What Changes

- 将 03 重写为 `internal/pocv1/presence` 的抽离蓝图。
- 定义并后续实现唯一的 domain discover contract：`DiscoverView`、`DiscoverPeer` 与 `LastSeenPeer`。
- 定义 03 的固定 runtime 输入：`runtime_broker`、`TopicScope`、`roster_snapshot`、device-key 派生的 `self_peer_id`，以及 caller-supplied 的本机 display hints / `app_ver`。
- 定义 `net_root` 下的 presence topic、retained/LWT 生命周期与固定 JSON payload，并写死 graceful / unexpected disconnect 都收敛到 retained `offline`。
- 明确 Discover 的在线态 source of truth 只有 `.../presence/+` 订阅结果；成员身份与拨号所需信任材料来自 `02/06` persisted roster，而不是 presence payload。
- 明确 trusted roster 是 discover peer set 的边界；unknown / malformed presence 只进入 diagnostic/evidence，不进入 discover view。
- 给 03 增加独立的 retained snapshot、roster-only offline、reconnect、duplicate convergence，以及 `04/07` consumer seam 验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-presence-discover`: 当前 POC v1 的 presence-only discover 合同。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/presence/*`
- 计划参考的 legacy 行为：`internal/controlplane/inbox_topic.go`、现有 MQTT 会话生命周期、`archive/_legacy_poc_v0/internal/task/desktop_state.go` 中对 peer 快照的消费方式，以及当前 `internal/localapi/*` / `internal/desktopbridge/*` 的快照消费边界
- 不包含 mailbox/enroll 流程、dial/punch、session recipe 或 GUI stage runtime。
