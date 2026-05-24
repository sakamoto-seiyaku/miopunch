## Why

如果这轮 v1 继续让 `PathResult`、legacy `dataplane`、旧 TLS pin 逻辑和 task 层直接纠缠，作者很难清楚回答“我到底在什么地方绑定了对端身份”。05 的职责就是把 session upgrade 抽成一条单一 recipe，并把 pin 口径讲死。

## What Changes

- 将 05 重写为 `internal/pocv1/session` 的抽离蓝图。
- 定义并后续实现 `SessionRecipe`：消费 `PathResult`，产出 `PeerSession`，唯一支持 `UDP + KCP + TLS1.3 + yamux`。
- 明确可以窄复用 legacy `dataplane` / `internal/tlsutil` 作为实现底座，但 v1 source of truth 在 05，而不是反过来让 legacy `dataplane` 决定 v1 contract。
- 为 05 增加 pin success/fail、session open/accept、`PathResult -> PeerSession` 集成验收项。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-secure-session`: 当前 POC v1 的 secure session recipe 与 identity pin contract。

### Modified Capabilities

- (none)

## Impact

- 计划新增代码：`internal/pocv1/session/*`
- 计划参考或窄适配的 legacy 实现：`dataplane/session*.go`、`dataplane/stream.go`、`internal/tlsutil/*`
- 不包含 candidate gather/attempt、GUI 编排或多 recipe/多 carrier 支持。
