## Why

POC v1 需要“对用户友好”的 Discover：用户必须能看到 peer 列表与在线状态，并且后续 dial/punch 能直接拿到对端的控制面公钥。

Hard-Min 选择 presence-only（不引入目录查询 kind），用 retained + LWT 给 GUI 提供即时快照。

## What Changes

- 定义并实现 `presence_topic`：上线 retained online + LWT retained offline。
- payload 固定为可读 JSON（不进入签名/安全语义），包含 peer_id + ed25519/x25519 pub。
- `Discover` 输入契约固定为订阅 `.../presence/+` 的结果；具体渲染归 `poc-v1-07-gui-wizard`。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-presence-discover`: presence-only discover 的 v1 口径。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：MQTT client 连接生命周期、presence publish/subscribe、以及 discover 数据契约。
