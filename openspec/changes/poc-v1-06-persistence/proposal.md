## Why

POC v1 要求“你能尽快理解”，离不开可预测的本地状态：设备密钥、已加入网络、MemberCredential、mailbox_secret、最近 peer 列表等都必须稳定落盘。

Hard-Min 选择 7B：多网络最小化目录结构，明文落盘但权限收紧（不引入解锁/加密 UI）。

## What Changes

- 定义 v1 持久化目录结构：`device/` + `networks/<network_id>/`。
- 定义每个文件的职责与原子写入方式。
- 权限策略：0600/0700。

## Capabilities

### New Capabilities

- `miopunch-poc-v1-persistence`: v1 本地持久化与目录结构。

### Modified Capabilities

- (none)

## Impact

- 预计主要修改：state/persist 模块与 first-run 初始化；后续 changes 通过统一 persist API 接入。
